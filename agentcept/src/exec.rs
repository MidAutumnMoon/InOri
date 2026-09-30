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
use rootcause::Result;
use rootcause::bail;
use rootcause::report;

/// Replaces this process with `name`, skipping this executable during lookup.
///
/// Returns exit code 127 if lookup or `exec` fails.
pub fn real(name: &OsStr, args: &[OsString]) -> ExitCode {
    match exec_real(name, args) {
        Ok(never) => match never {},
        Err(report) => {
            eprintln!("agentcept: {report}");
            ExitCode::from(127)
        }
    }
}

/// Replaces this process with `name`'s real tool, preferring `replacement`
/// when it locates a usable executable other than agentcept itself.
///
/// The replacement runs under the intercepted `name` as `argv[0]`, so a
/// multicall replacement such as ugrep serves the matching dialect.
/// `AGENTCEPT_TOOLS=gnu` disables the preference; a failed `exec` of the
/// replacement falls back to the real tool with a warning.
pub fn prefer(
    replacement: Option<&OsStr>,
    name: &OsStr,
    args: &[OsString],
) -> ExitCode {
    let replacement = replacement.filter(|_| !opted_out());
    if let Some(replacement) = replacement {
        // Warnings must survive the imminent `exec`, so they cannot go
        // through the lossy non-blocking tracing writer.
        match exec_replacement(replacement, name, args) {
            Ok(never) => match never {},
            Err(report) => {
                eprintln!("agentcept: {report}");
                eprintln!(
                    "agentcept: using the real `{}` instead",
                    name.to_string_lossy(),
                );
            }
        }
    }
    real(name, args)
}

/// `AGENTCEPT_TOOLS=gnu` keeps every call on the real tools.
fn opted_out() -> bool {
    std::env::var_os("AGENTCEPT_TOOLS").is_some_and(|value| {
        value.as_os_str().as_encoded_bytes() == b"gnu"
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileId {
    dev: u64,
    ino: u64,
}

impl FileId {
    // Without a reliable identity, PATH lookup could select this shim again.
    fn discover() -> Result<Self> {
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

    // Follows symlinks so paths to the same executable compare equal.
    fn of(path: &Path) -> Option<Self> {
        let meta = std::fs::metadata(path).ok()?;
        Some(Self {
            dev: meta.dev(),
            ino: meta.ino(),
        })
    }
}

fn exec_real(
    name: &OsStr,
    args: &[OsString],
) -> Result<std::convert::Infallible> {
    let myself = FileId::discover()?;

    if name.as_encoded_bytes().contains(&b'/') {
        let path = PathBuf::from(name);
        if FileId::of(&path) == Some(myself) {
            bail!(
                "`{}` is agentcept itself; refusing to exec in a loop",
                name.to_string_lossy()
            );
        }
        return exec_as(&path, path.as_os_str(), args);
    }

    let Some(path) = locate_in_path(myself, name) else {
        bail!(
            "no usable `{}` in $PATH after skipping agentcept itself",
            name.to_string_lossy()
        );
    };
    exec_as(&path, path.as_os_str(), args)
}

/// Runs `replacement` for `name`, or reports why it cannot serve the call.
fn exec_replacement(
    replacement: &OsStr,
    name: &OsStr,
    args: &[OsString],
) -> Result<std::convert::Infallible> {
    let myself = FileId::discover()?;
    let path = locate_replacement(myself, replacement)?;
    exec_as(&path, name, args)
}

/// Resolves the preferred replacement: an explicit path must name a usable
/// executable, a bare name is looked up via `$PATH`.
fn locate_replacement(
    myself: FileId,
    replacement: &OsStr,
) -> Result<PathBuf> {
    if replacement.as_encoded_bytes().contains(&b'/') {
        let path = PathBuf::from(replacement);
        if FileId::of(&path) == Some(myself) {
            bail!(
                "`{}` is agentcept itself",
                replacement.to_string_lossy()
            );
        }
        if !usable(&path) {
            bail!(
                "`{}` is not a usable executable",
                replacement.to_string_lossy()
            );
        }
        return Ok(path);
    }
    locate_in_path(myself, replacement).ok_or_else(|| {
        rootcause::report!(
            "no usable `{}` in $PATH",
            replacement.to_string_lossy()
        )
    })
}

fn usable(path: &Path) -> bool {
    std::fs::metadata(path)
        .is_ok_and(|meta| meta.is_file() && path.is_executable())
}

/// Searches `$PATH` for `name`, skipping unusable entries, this
/// executable itself, and other agentcept installs.
fn locate_in_path(myself: FileId, name: &OsStr) -> Option<PathBuf> {
    let search_path = std::env::var_os("PATH")
        .unwrap_or_else(|| OsString::from("/usr/bin:/bin"));
    path_entries(&search_path)
        .map(|entry| entry.join(name))
        .find(|candidate| {
            FileId::of(candidate) != Some(myself)
                && !is_shim_install(candidate)
                && usable(candidate)
        })
}

/// Detects a multicall install: a tool-named symlink pointing at an
/// executable named `agentcept` — this build, an older build, or a copy.
///
/// Skipping only our own inode is not enough: with two agentcept installs
/// in `$PATH`, each exec's the other in a loop.
fn is_shim_install(candidate: &Path) -> bool {
    std::fs::read_link(candidate).is_ok_and(|target| {
        target
            .file_name()
            .is_some_and(|name| name.as_encoded_bytes() == b"agentcept")
    })
}

/// Execs `path` as `argv0`, replacing this process.
///
/// `exec` returns only if the process replacement fails.
fn exec_as(
    path: &Path,
    argv0: &OsStr,
    args: &[OsString],
) -> Result<std::convert::Infallible> {
    let mut command = Command::new(path);
    command.arg0(argv0);
    command.args(args);
    let err = command.exec();
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
