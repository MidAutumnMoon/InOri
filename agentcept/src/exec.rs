//! Locate and exec the real tool without recursing into agentcept.

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

/// Replaces this process with `name`, skipping this executable during lookup.
///
/// Returns exit code 127 if lookup or `exec` fails.
pub fn real(name: &OsStr, args: &[OsString]) -> ExitCode {
    match resolve_and_exec(name, args) {
        Ok(never) => match never {},
        Err(report) => {
            eprintln!("agentcept: {report}");
            ExitCode::from(127)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileId {
    dev: u64,
    ino: u64,
}

impl FileId {
    // Without a reliable identity, PATH lookup could select this shim again.
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

    // Follow symlinks so paths to the same executable compare equal.
    fn of(path: &Path) -> Option<Self> {
        let meta = std::fs::metadata(path).ok()?;
        Some(Self {
            dev: meta.dev(),
            ino: meta.ino(),
        })
    }
}

fn resolve_and_exec(
    name: &OsStr,
    args: &[OsString],
) -> rootcause::Result<std::convert::Infallible> {
    let myself = FileId::discover()?;

    if name.as_encoded_bytes().contains(&b'/') {
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

fn exec_path(
    path: &Path,
    args: &[OsString],
) -> rootcause::Result<std::convert::Infallible> {
    // `exec` returns only if process replacement fails.
    let err = Command::new(path).args(args).exec();
    Err(report!(err)
        .context(format!("exec {}", path.display()))
        .into())
}

/// Splits `$PATH`, treating empty entries as the current directory.
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
