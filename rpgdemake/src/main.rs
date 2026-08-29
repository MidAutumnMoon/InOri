use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;

use bpaf::OptionParser;
use bpaf::Parser as _;
use bpaf::construct;
use bpaf::long;
use bpaf::positional;
use rootcause::Result;
use rootcause::bail;
use rootcause::option_ext::OptionExt as _;
use rootcause::prelude::ResultExt as _;
use tap::Pipe as _;
use tracing::debug;

mod key;
mod lore;
mod task;

use lore::DecryptAction;
use lore::EncryptedAsset;

/// Decrypt mode.
#[derive(Debug, Clone, Copy)]
enum Mode {
    /// Decrypt PNG images only, without needing the encryption key.
    Light,
    /// Decrypt all assets using the encryption key from System.json.
    Full,
}

impl FromStr for Mode {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "light" => Ok(Self::Light),
            "full" => Ok(Self::Full),
            other => Err(format!(
                "expected one of `light`, `full`, got `{other}`"
            )),
        }
    }
}

impl std::fmt::Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Light => "light",
            Self::Full => "full",
        })
    }
}

/// A simple CLI tool for batch decrypting RPG Maker MV/MZ assets.
#[derive(Debug)]
struct CliOpts {
    /// Decryption mode.
    mode: Mode,

    /// Path to the directory containing the game.
    game_dir: PathBuf,
}

#[must_use]
fn cli() -> OptionParser<CliOpts> {
    let mode = long("mode")
        .argument::<Mode>("MODE")
        .help(
            "Decryption mode: `light` (default) skips the key and only \
             decrypts PNG images by restoring the known PNG header; \
             `full` reads the encryption key from System.json and \
             decrypts all asset types (PNG, OGG, M4A)",
        )
        .fallback(Mode::Light)
        .display_fallback();
    let game_dir = positional::<PathBuf>("GAME_DIR")
        .help("Path to the directory containing the game");
    construct!(CliOpts { mode, game_dir })
        .to_options()
        .descr("A simple CLI tool for batch decrypting RPG Maker MV/MZ assets")
        .version(env!("CARGO_PKG_VERSION"))
}

fn main() -> Result<()> {
    let _log_guard = ino_tracing::init_tracing_subscriber();
    rlimit::increase_nofile_limit(u64::MAX)?;

    let cliopts = cli().run();

    debug!(?cliopts);

    let root = &cliopts.game_dir;

    if !root.is_dir() {
        bail!("{} is not a directory", root.display());
    }
    let has_locales = root
        .join("locales")
        .try_exists()
        .context("Failed to check for the locales entry")?;
    if !has_locales {
        bail!(
            "Game folder doesn't contain necessary files to be recognized \
            as a RPG Maker game. Maybe the directory is wrong, \
            it's not a RPG Maker MV/MZ game, or the files are packed into the exe."
        );
    }

    // Collect encrypted files

    debug!(?cliopts.mode, "collect files to decrypt");

    let assets = find(root, cliopts.mode)?;

    debug!(?assets, "found files");

    // Build decrypt method

    let method = match cliopts.mode {
        Mode::Full => {
            let system_json = find_system_json(root)
                .context("Failed to locate System.json")?
                .context("System.json not found in game directory")?;

            debug!(?system_json, "read encryption key from System.json");

            let key = std::fs::read_to_string(system_json)?
                .pipe_as_ref(key::Key::parse_json)?
                .context(
                    "System.json does not contain encryption key, \
                     maybe assets are not encrypted?",
                )?;

            DecryptAction::Full(key)
        }
        Mode::Light => DecryptAction::Light,
    };

    debug!(?method);

    task::run(&assets, &method)?;

    Ok(())
}

/// Find encrypted assets under `toplevel` according to `mode`.
///
/// - `Mode::Light`: only encrypted PNG files (`.rpgmvp` / `.png_`).
/// - `Mode::Full`: all encrypted RPG Maker asset types.
#[tracing::instrument]
fn find(toplevel: &Path, mode: Mode) -> Result<Vec<EncryptedAsset>> {
    let assets = walkdir::WalkDir::new(toplevel)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .filter(|entry| !entry.file_type().is_dir())
        .filter_map(|entry| {
            let asset = EncryptedAsset::new(entry.path().to_owned())?;
            match mode {
                Mode::Light if !asset.is_png() => None,
                Mode::Light | Mode::Full => Some(asset),
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
fn find_system_json(root: &Path) -> Result<Option<PathBuf>> {
    for entry in walkdir::WalkDir::new(root) {
        let entry = entry?;
        if !entry.file_type().is_dir()
            && entry.file_name() == "System.json"
        {
            return Ok(Some(entry.path().to_owned()));
        }
    }
    Ok(None)
}
