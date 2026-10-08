//! Persistence of the last-applied blueprint.
//!
//! The state file is itself a blueprint (v1) holding the *rendered*
//! symlinks of the last fully successful run, not template text, so an
//! environment change migrates links rather than orphaning them.
//! Reading it yields the old generation; a fully successful run
//! atomically supersedes it, so "recorded" and "applied" coincide. An
//! interrupted run retries against the previous state and converges.

use std::fs::DirBuilder;
use std::fs::File;
use std::io::ErrorKind;
use std::io::Write as _;
use std::os::unix::fs::DirBuilderExt as _;
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr as _;

use rand::RngExt as _;
use rootcause::Result;
use rootcause::bail;
use rootcause::prelude::ResultExt as _;
use rootcause::report;
use tap::Tap as _;
use tracing::debug;
use tracing::trace;

use crate::blueprint::Blueprint;
use crate::blueprint::Symlink;

/// Read the old generation from the state file.
///
/// A missing state file is a first run, so nothing has been applied yet.
/// Anything else (unreadable, not a regular file, invalid content) is a
/// hard error: a damaged state must never masquerade as a first run and
/// orphan the previously managed symlinks.
#[tracing::instrument]
pub fn load_old(path: &Path) -> Result<Vec<Symlink>> {
    match path.symlink_metadata() {
        Ok(metadata) => {
            if !metadata.is_file() {
                bail!(
                    r#"State path "{}" is not a regular file"#,
                    path.display()
                );
            }
        }
        Err(err) if err.kind() == ErrorKind::NotFound => {
            debug!("state file absent, first run");
            return Ok(Vec::new());
        }
        Err(err) => {
            return Err(report!(err)
                .context(format!(
                    r#"Failed to inspect the state file "{}""#,
                    path.display()
                ))
                .into());
        }
    }

    let raw = std::fs::read_to_string(path).context_with(|| {
        format!(r#"Failed to read the state file "{}""#, path.display())
    })?;

    let blueprint = Blueprint::from_str(&raw)
        .context_with(|| {
            format!(
                r#"The state file "{}" is not a valid blueprint"#,
                path.display()
            )
        })?
        .tap(|blueprint| trace!(?blueprint));

    Ok(blueprint.into_symlinks())
}

/// The state write owed by a fully successful run.
pub struct PendingState {
    path: PathBuf,
    content: String,
}

impl PendingState {
    #[tracing::instrument(skip_all)]
    pub fn new(path: PathBuf, new_blueprint: &Blueprint) -> Result<Self> {
        // Serialize upfront so a failure surfaces before any
        // filesystem mutation.
        let content = serde_json::to_string_pretty(new_blueprint)
            .context("Failed to serialize the blueprint for the state")?;
        trace!(%content);
        Ok(Self { path, content })
    }

    /// Atomically install the state. On failure the previous state is
    /// intact.
    #[tracing::instrument(skip_all)]
    pub fn write(self) -> Result<()> {
        let Self { path, content } = self;

        let Some(parent) = path.parent() else {
            bail!(
                r#"State path "{}" has no parent directory"#,
                path.display()
            );
        };
        // Like symlink destinations, missing state directories are
        // created rather than failing the run after a full apply.
        // 0o700 to match StateDirectoryMode in deployment.
        DirBuilder::new()
            .mode(0o700)
            .recursive(true)
            .create(parent)
            .context_with(|| {
                format!(
                    r#"Failed to create the state directory "{}""#,
                    parent.display()
                )
            })?;

        let tmp_path = {
            use rand::distr::Alphanumeric;
            let suffix = rand::rng()
                .sample_iter(&Alphanumeric)
                .take(6)
                .map(char::from)
                .collect::<String>();
            let mut ostr = path.as_os_str().to_owned();
            ostr.push(format!(".tmp-{suffix}"));
            let tmp_path = PathBuf::from(ostr);
            trace!(?tmp_path, "temporary state file");
            tmp_path
        };

        if let Err(err) = Self::install(&tmp_path, &path, &content) {
            debug!("state installation failed, remove the temporary file");
            if let Err(cleanup_err) = std::fs::remove_file(&tmp_path)
                && cleanup_err.kind() != ErrorKind::NotFound
            {
                return Err(report!(cleanup_err)
                    .context(format!(
                        r#"Failed to remove the intermediate state file \
                            "{}", filesystem might be cooked"#,
                        tmp_path.display(),
                    ))
                    .into());
            }
            return Err(err);
        }

        Ok(())
    }

    fn install(tmp_path: &Path, path: &Path, content: &str) -> Result<()> {
        let mut file = File::create(tmp_path).context_with(|| {
            format!(
                r#"Failed to create the temporary state file "{}""#,
                tmp_path.display()
            )
        })?;
        file.write_all(content.as_bytes()).context_with(|| {
            format!(
                r#"Failed to write the temporary state file "{}""#,
                tmp_path.display()
            )
        })?;
        file.sync_all().context_with(|| {
            format!(
                r#"Failed to sync the temporary state file "{}""#,
                tmp_path.display()
            )
        })?;
        std::fs::rename(tmp_path, path).context_with(|| {
            format!(
                r#"Failed to install the state file "{}""#,
                path.display()
            )
        })?;
        Ok(())
    }
}
