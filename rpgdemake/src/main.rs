use std::path::PathBuf;

use tracing::debug;

use anyhow::Context;
use anyhow::bail;
use anyhow::ensure;

mod finder;
mod key;
mod lore;
mod project;
mod task;

use project::EngineRev;

/// Decrypt mode.
#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
enum DecryptMode {
    /// Decrypt all assets using the encryption key from System.json.
    Full,
    /// Decrypt PNG images only, without needing the encryption key.
    /// Restores the PNG header by exploiting the fixed PNG signature.
    Light,
}

/// A simple CLI tool for batch decrypting RPG Maker MV/MZ assets.
#[derive(clap::Parser, Debug)]
struct CliOpts {
    /// Path to the directory containing the game.
    game_dir: PathBuf,

    /// Decryption mode.
    ///
    /// "full" reads the encryption key from System.json and decrypts
    /// all asset types (PNG, OGG, M4A). "light" skips the key and
    /// only decrypts PNG images by restoring the known PNG header.
    #[arg(long, value_enum, default_value = "full")]
    mode: DecryptMode,
}

fn main() -> anyhow::Result<()> {
    ino_tracing::init_tracing_subscriber();
    let cliopts = <CliOpts as clap::Parser>::parse();

    debug!(?cliopts);

    debug!("increase NOFILE rlimit");
    rlimit::increase_nofile_limit(u64::MAX)?;

    // Setup & sanity checks

    debug!("Probe game engine revision");

    let engine_rev = EngineRev::probe_revision(&cliopts.game_dir)
        .context("Failed to understand game's engine revision")?;

    match cliopts.mode {
        DecryptMode::Light => run_light(&engine_rev),
        DecryptMode::Full => run_full(&engine_rev),
    }
}

fn run_light(engine_rev: &EngineRev) -> anyhow::Result<()> {
    let img_dir = engine_rev.get_img_dir();

    debug!(?img_dir, "light mode: scanning for encrypted PNGs");

    let files = finder::find_png(&img_dir)?;

    debug!(?files, "light mode: found files");

    task::run_light(&files)?;

    Ok(())
}

fn run_full(engine_rev: &EngineRev) -> anyhow::Result<()> {
    let system_json = engine_rev.get_data_dir().join("System.json");

    let resource_dirs =
        vec![engine_rev.get_img_dir(), engine_rev.get_audio_dir()];

    debug!(?system_json, ?resource_dirs);

    // Get encryption key

    debug!("try read encryption key");

    let enc_key = {
        ensure! { system_json.is_file(),
            "System.json doesn't exist at \"{}\"",
            system_json.display()
        };

        let key =
            key::Key::parse_json(&std::fs::read_to_string(system_json)?)?;

        match key {
            Some(k) => k,
            None => bail!(
                "System.json does not contain encryption key, maybe not encrypted?"
            ),
        }
    };

    debug!(?enc_key);

    // Collect files to decrypt

    debug!("collect files to decrypt");

    let files = {
        use anyhow::Result as AResult;

        let files: Vec<PathBuf> = resource_dirs
            .iter()
            .map(|p| finder::find_all(p))
            .collect::<AResult<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect();

        debug!(?files, "all found files");

        files
    };

    task::run(&files, &enc_key)?;

    Ok(())
}
