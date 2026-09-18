//! Finding and exec'ing the real tool, skipping agentcept's own shims
//! so PATH dispatch can't loop back into us.

use std::ffi::OsStr;
use std::ffi::OsString;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::MetadataExt as _;
use std::os::unix::process::CommandExt as _;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::ExitCode;

use ino_path::is_executable::IsExecutable as _;
use rootcause::bail;
use rootcause::report;

/// Exec the real tool `name` with `args`, replacing this process.
///
/// Only returns when exec failed; on success the real command runs to
/// completion and its exit status becomes ours directly.
pub fn real(name: &OsStr, args: &[OsString]) -> ExitCode {
    match resolve_and_exec(name, args) {
        Ok(never) => match never {},
        Err(report) => {
            eprintln!("agentcept: {report}");
            ExitCode::from(127)
        }
    }
}

/// Unix file identity, for recognizing our own executable
/// no matter how the shims were set up (symlink, hardlink, copy).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileId {
    dev: u64,
    ino: u64,
}

impl FileId {
    /// Identity of our own executable. Required, not optional: without it
    /// we cannot promise to skip our own shims, so we fail closed instead
    /// of risking an exec loop.
    ///
    /// # Errors
    ///
    /// A dynamic [`Report`](rootcause::Report) when the current executable
    /// cannot be resolved or stat'ed.
    fn discover() -> rootcause::Result<Self> {
        let exe = match std::env::current_exe() {
            Ok(exe) => exe,
            Err(err) => {
                return Err(report!(err)
                    .context("resolving our own executable")
                    .into());
            }
        };
        Self::of(&exe).ok_or_else(|| {
            rootcause::report!("stat'ing our own executable failed")
        })
    }

    /// Identity of what `path` refers to (symlinks followed);
    /// `None` if it can't be stat'ed.
    fn of(path: &Path) -> Option<Self> {
        let meta = std::fs::metadata(path).ok()?;
        Some(Self {
            dev: meta.dev(),
            ino: meta.ino(),
        })
    }
}

/// Exec the real tool, searching `$PATH` and skipping anything that is
/// this very executable, so we never dispatch to ourselves.
fn resolve_and_exec(
    name: &OsStr,
    args: &[OsString],
) -> rootcause::Result<std::convert::Infallible> {
    let myself = FileId::discover()?;

    if name.as_encoded_bytes().contains(&b'/') {
        // An explicit path: exec it directly, but never ourselves.
        let path = PathBuf::from(name);
        if FileId::of(&path) == Some(myself) {
            bail!(
                "`{}` is agentcept itself; refusing to exec in a loop",
                name.to_string_lossy()
            );
        }
        return exec_path(&path, args);
    }

    let search_path = std::env::var_os("PATH")
        .unwrap_or_else(|| OsString::from("/usr/bin:/bin"));
    for entry in path_entries(&search_path) {
        let candidate = entry.join(name);
        if FileId::of(&candidate) == Some(myself) {
            continue;
        }
        let Ok(meta) = std::fs::metadata(&candidate) else {
            continue;
        };
        if !meta.is_file() || !candidate.is_executable() {
            continue;
        }
        return exec_path(&candidate, args);
    }
    bail!(
        "no usable `{}` in $PATH after skipping agentcept itself",
        name.to_string_lossy()
    );
}

/// Replace this process with `path`; `exec` only returns on failure.
fn exec_path(
    path: &Path,
    args: &[OsString],
) -> rootcause::Result<std::convert::Infallible> {
    let err = Command::new(path).args(args).exec();
    Err(report!(err)
        .context(format!("exec {}", path.display()))
        .into())
}

/// `$PATH` split into directories; an empty entry means the current
/// directory, as shells read it.
fn path_entries(search_path: &OsStr) -> impl Iterator<Item = PathBuf> {
    search_path
        .as_encoded_bytes()
        .split(|ch| *ch == b':')
        .map(|entry| {
            if entry.is_empty() {
                PathBuf::from(".")
            } else {
                PathBuf::from(OsStr::from_bytes(entry))
            }
        })
}

#[cfg(test)]
mod test {
    use std::ffi::OsStr;
    use std::path::PathBuf;

    use super::path_entries;

    #[test]
    fn splits_path_entries() {
        let entries: Vec<PathBuf> =
            path_entries(OsStr::new("/a/bin::/b/bin")).collect();
        assert_eq!(
            entries,
            [
                PathBuf::from("/a/bin"),
                PathBuf::from("."),
                PathBuf::from("/b/bin")
            ]
        );
    }
}
