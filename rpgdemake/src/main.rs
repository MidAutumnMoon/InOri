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
use lore::EncryptedKind;
use project::EngineRev;

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

    run(&engine_rev, cliopts.mode)
}

fn run(engine_rev: &EngineRev, mode: DecryptMode) -> anyhow::Result<()> {
    let resource_dirs = match mode {
        DecryptMode::Light => vec![engine_rev.get_img_dir()],
        DecryptMode::Full => {
            vec![engine_rev.get_img_dir(), engine_rev.get_audio_dir()]
        }
    };

    // Collect files to decrypt

    debug!(?mode, ?resource_dirs, "collect files to decrypt");

    let files = {
        use anyhow::Result as AResult;

        let files: Vec<PathBuf> = resource_dirs
            .iter()
            .map(|p| find(p, mode))
            .collect::<AResult<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect();

        debug!(?files, "found files");

        files
    };

    // Get encryption key (full mode only)

    let enc_key = match mode {
        DecryptMode::Full => {
            let system_json =
                engine_rev.get_data_dir().join("System.json");

            debug!(?system_json, "try read encryption key");

            ensure! { system_json.is_file(),
                "System.json doesn't exist at \"{}\"",
                system_json.display()
            };

            let key = key::Key::parse_json(&std::fs::read_to_string(
                system_json,
            )?)?;

            match key {
                Some(k) => Some(k),
                None => bail!(
                    "System.json does not contain encryption key, maybe not encrypted?"
                ),
            }
        }
        DecryptMode::Light => None,
    };

    debug!(?enc_key);

    task::run(&files, enc_key.as_ref())?;

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
                    let kind = EncryptedKind::from_ext(ext)?;
                    match mode {
                        DecryptMode::Light if !kind.is_png() => None,
                        _ => Some(path),
                    }
                })
                .collect()
        })?;

    Ok(files)
}
