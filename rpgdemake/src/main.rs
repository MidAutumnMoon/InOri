use std::path::Path;
use std::path::PathBuf;

use tap::Pipe;
use tracing::debug;

use anyhow::Context;
use anyhow::ensure;
use clap::Parser;

mod key;
mod lore;
mod task;

use lore::DecryptMethod;
use lore::EncryptedAsset;

/// Decrypt mode.
#[derive(clap::ValueEnum, Debug, Clone, Copy)]
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
    let cliopts = CliOpts::parse();

    debug!(?cliopts);

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

    let assets = find(root, cliopts.mode)?;

    debug!(?assets, "found files");

    // Build decrypt method

    let method = match cliopts.mode {
        DecryptMode::Full => {
            let system_json = find_system_json(root)
                .context("Failed to locate System.json")?
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "System.json not found in game directory"
                    )
                })?;

            debug!(?system_json, "read encryption key from System.json");

            let key = std::fs::read_to_string(system_json)?
                .pipe_as_ref(key::Key::parse_json)?
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "System.json does not contain encryption key, maybe assets are not encrypted?"
                    )
                })?;

            DecryptMethod::Full(key)
        }
        DecryptMode::Light => DecryptMethod::Light,
    };

    debug!(?method);

    task::run(&assets, &method)?;

    Ok(())
}

/// Find encrypted assets under `toplevel` according to `mode`.
///
/// - `DecryptMode::Light`: only encrypted PNG files (`.rpgmvp` / `.png_`).
/// - `DecryptMode::Full`: all encrypted RPG Maker asset types.
#[tracing::instrument]
fn find(
    toplevel: &Path,
    mode: DecryptMode,
) -> anyhow::Result<Vec<EncryptedAsset>> {
    let assets = walkdir::WalkDir::new(toplevel)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter_map(|entry| {
            let asset = EncryptedAsset::new(entry.path().to_owned())?;
            match mode {
                DecryptMode::Light if !asset.is_png() => None,
                _ => Some(asset),
            }
        })
        .collect();

    Ok(assets)
}

/// Locate `System.json` anywhere under `root`.
///
/// FOOTGUN: returns the *first* match, which may not be the game's
/// System.json if other files share that name (e.g. bundled plugins).
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
