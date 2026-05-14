use std::path::Path;
use std::path::PathBuf;

use anyhow::ensure;
use tracing::debug;

/// A validated RPG Maker game directory.
#[derive(Debug, Clone)]
pub struct GameDir(PathBuf);

impl GameDir {
    #[tracing::instrument]
    pub fn probe(root: PathBuf) -> anyhow::Result<Self> {
        debug!("Probe game directory");

        ensure! { root.is_dir(),
            "{} is not a directory", root.display()
        };
        ensure! { root.join("locales").try_exists()?,
            "Game folder doesn't contain necessary files to be recognized \
            as a RPG Maker game. Maybe the directory is wrong, \
            it's not a RPG Maker MV/MZ game, or the files are packed into the exe."
        };

        Ok(Self(root))
    }

    pub fn root(&self) -> &Path {
        &self.0
    }
}
