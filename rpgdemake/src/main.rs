use std::path::Path;
use std::path::PathBuf;

use tracing::debug;

use anyhow::Context;
use anyhow::bail;
use anyhow::ensure;

mod key;
mod lore;
mod task;

use lore::EncryptedKind;

/// Decrypt mode.
#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
enum DecryptMode {
    /// Decrypt PNG images only, without needing the encryption key.
    Light,
    /// Decrypt all assets using the encryption key from System.json.
    Full,
}

/// A simple CLI tool for batch decrypting RPG Maker MV/MZ assets.
#[derive(clap::Parser, Debug)]
struct CliOpts {
    /// Path to the directory containing the game.
    game_dir: PathBuf,

    /// Decryption mode.
    ///
    /// "light" (default) skips the key and only decrypts PNG images
    /// by restoring the known PNG header. "full" reads the encryption
    /// key from System.json and decrypts all asset types (PNG, OGG, M4A).
    #[arg(long, value_enum, default_value = "light")]
    mode: DecryptMode,
}

fn main() -> anyhow::Result<()> {
    ino_tracing::init_tracing_subscriber();
    let cliopts = <CliOpts as clap::Parser>::parse();

    debug!(?cliopts);

    debug!("increase NOFILE rlimit");
    rlimit::increase_nofile_limit(u64::MAX)?;

    let root = &cliopts.game_dir;

    ensure! { root.is_dir(),
        "{} is not a directory", root.display()
    };
    ensure! { root.join("locales").try_exists()?,
        "Game folder doesn't contain necessary files to be recognized \
        as a RPG Maker game. Maybe the directory is wrong, \
        it's not a RPG Maker MV/MZ game, or the files are packed into the exe."
    };

    // Collect encrypted files

    debug!(?cliopts.mode, "collect files to decrypt");

    let files = find(root, cliopts.mode)?;

    debug!(?files, "found files");

    // Get encryption key (full mode only)

    let enc_key = match cliopts.mode {
        DecryptMode::Full => {
            let system_json = find_system_json(root)
                .context("Failed to locate System.json")?;

            let Some(system_json) = system_json else {
                bail!("System.json not found in game directory")
            };

            debug!(?system_json, "try read encryption key");

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

/// Locate `System.json` anywhere under `root`.
#[tracing::instrument]
fn find_system_json(root: &Path) -> anyhow::Result<Option<PathBuf>> {
    for entry in walkdir::WalkDir::new(root) {
        let entry = entry?;
        if entry.file_type().is_file()
            && entry.file_name() == "System.json"
        {
            return Ok(Some(entry.path().to_owned()));
        }
    }
    Ok(None)
}
