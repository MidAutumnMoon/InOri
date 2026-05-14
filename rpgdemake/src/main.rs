use std::path::Path;
use std::path::PathBuf;

use tracing::debug;

use anyhow::Context;
use anyhow::bail;
use anyhow::ensure;

mod key;
mod lore;
mod project;
mod task;

use lore::DecryptMode;
use project::EngineRev;

use crate::lore::map_encrypted_extension;

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

    let files = find(&img_dir, DecryptMode::Light)?;

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
            .map(|p| find(p, DecryptMode::Full))
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

/// Find encrypted files under `toplevel` according to `mode`.
///
/// - `DecryptMode::Light`: only encrypted PNG files (`.rpgmvp` / `.png_`).
/// - `DecryptMode::Full`: all encrypted RPG Maker asset types.
#[tracing::instrument]
fn find(
    toplevel: &Path,
    mode: DecryptMode,
) -> anyhow::Result<Vec<PathBuf>> {
    use itertools::Itertools;
    use rayon::prelude::*;

    let files = walkdir::WalkDir::new(toplevel)
        .into_iter()
        .process_results(|iter| {
            iter.par_bridge()
                .map(|entry| entry.path().to_owned())
                .filter(|path| path.is_file())
                .filter_map(|path| {
                    let ext = path.extension()?.to_str()?;
                    match mode {
                        DecryptMode::Light => match ext {
                            "rpgmvp" | "png_" => Some(path),
                            _ => None,
                        },
                        DecryptMode::Full => {
                            map_encrypted_extension(ext)?;
                            Some(path)
                        }
                    }
                })
                .collect()
        })?;

    Ok(files)
}
