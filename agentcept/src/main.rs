//! agentcept: multicall PATH shim that intercepts common agent tool calls
//! (`find`, `grep`, `python`, ...) and refuses the unbounded ones before
//! exec'ing the real binary.
//!
//! Dispatch is by `argv[0]` basename (multicall symlinks), or by the first
//! argument when invoked as `agentcept` itself.

use std::ffi::OsStr;
use std::ffi::OsString;
use std::os::unix::ffi::OsStrExt as _;
use std::path::Component;
use std::path::Path;
use std::process::ExitCode;

mod exec;
mod find;
mod grep;
mod python;

/// Name of the dispatcher, i.e. when no applet symlink is used.
const BIN_NAME: &str = "agentcept";

/// Help text, printed for `agentcept` with no arguments or `--help`.
const USAGE: &str = indoc::indoc! {"
    agentcept - intercept agent tool calls and police them

    Usage:
        agentcept <cmd> [ARGS...]    policy-check <cmd>, then run the real one
        agentcept                    this help

    Multicall: invoke through a symlink named after the tool (find, grep,
    egrep, fgrep, python, python3, pip, ...), placed in $PATH before the
    real binary:
        PATH=/path/to/agentcept/bin:$PATH

    Policies:
        find    refuse a search starting at `/`
        grep    refuse a recursive search starting at `/`
        python  refuse: no global python on this host, use uv
        pip     refuse: no global pip on this host, use uv

    Unrecognized names error out; agentcept never runs a tool silently.
"};

fn main() -> ExitCode {
    let _log_guard = ino_tracing::init_tracing_subscriber();

    let mut argv_iter = std::env::args_os();
    // `argv[0]` always exists in practice; default to the dispatcher name.
    let invoked_as =
        argv_iter.next().unwrap_or_else(|| OsString::from(BIN_NAME));
    let args: Vec<OsString> = argv_iter.collect();

    dispatch(&invoked_as, &args)
}

/// An interception policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// `find`: refuse unbounded starting points.
    Find,
    /// `grep` (and `egrep`/`fgrep`): refuse unbounded recursive searches.
    Grep,
    /// `python`/`pip` family: refuse, point at `uv`.
    Python,
}

/// Map a tool name (basename) to its policy, if intercepted.
#[must_use]
fn classify(name: &OsStr) -> Option<Kind> {
    match name.as_encoded_bytes() {
        b"find" => Some(Kind::Find),
        b"grep" | b"egrep" | b"fgrep" => Some(Kind::Grep),
        name if python::is_family(name) => Some(Kind::Python),
        _ => None,
    }
}

/// Basename of a path-like string (`argv[0]` or a tool argument).
#[must_use]
fn basename(path: &OsStr) -> &OsStr {
    let raw = path.as_encoded_bytes();
    raw.iter()
        .rposition(|ch| *ch == b'/')
        .map_or(path, |at| {
            let (_, name) = raw.split_at(at + 1);
            OsStr::from_bytes(name)
        })
}

/// Resolve one invocation and run it to completion.
fn dispatch(invoked_as: &OsStr, args: &[OsString]) -> ExitCode {
    let name = basename(invoked_as);

    if name.as_encoded_bytes() != BIN_NAME.as_bytes() {
        let Some(kind) = classify(name) else {
            eprintln!(
                "agentcept: `{}` is not an intercepted tool; \
                 symlinked as the wrong name?",
                name.to_string_lossy()
            );
            return ExitCode::FAILURE;
        };
        return run(kind, name, args);
    }

    // Dispatcher mode: the first argument selects the tool.
    let Some((first, rest)) = args.split_first() else {
        eprintln!("{USAGE}");
        return ExitCode::FAILURE;
    };
    match first.as_encoded_bytes() {
        b"--help" | b"-h" | b"help" => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        _ => {
            let Some(kind) = classify(basename(first)) else {
                eprintln!(
                    "agentcept: unknown tool `{}`; run bare `agentcept` for usage",
                    first.to_string_lossy()
                );
                return ExitCode::FAILURE;
            };
            run(kind, first.as_os_str(), rest)
        }
    }
}

/// Apply the policy for `kind` and exec the real tool (or refuse).
fn run(kind: Kind, name: &OsStr, args: &[OsString]) -> ExitCode {
    match kind {
        Kind::Find => find::run(name, args),
        Kind::Grep => grep::run(name, args),
        Kind::Python => python::run(name, args),
    }
}

/// Render `name args...` on one line, for refusal messages.
#[must_use]
fn command_line(name: &OsStr, args: &[OsString]) -> String {
    std::iter::once(name.to_string_lossy())
        .chain(args.iter().map(|arg| arg.to_string_lossy()))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether the path lexically denotes the filesystem root
/// (e.g. `/`, `//`, `/.`, `/a/..`). Relative paths never do.
///
/// Purely lexical — no symlink resolution, no filesystem access —
/// which spots the habit, not evasion. `..` saturates at the root,
/// as Linux resolves it (`/..` is `/`).
#[must_use]
fn is_lexical_root(path: &Path) -> bool {
    let mut absolute = false;
    let mut depth = 0_usize;
    for component in path.components() {
        match component {
            Component::RootDir => absolute = true,
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            Component::ParentDir => depth = depth.saturating_sub(1),
            Component::Prefix(_) => return false,
        }
    }
    absolute && depth == 0
}

/// Whether a search starting at `operand` — or, when `None`, at the
/// working directory, both tools' default — begins at `/`.
///
/// Relative operands resolve against `cwd`. Without a cwd only
/// absolute operands can be judged; everything unresolvable passes
/// (fail open, like every other unknowable case).
#[must_use]
fn starts_at_root(operand: Option<&OsStr>, cwd: Option<&Path>) -> bool {
    match (operand, cwd) {
        (None, Some(cwd)) => cwd == Path::new("/"),
        (None, None) => false,
        (Some(operand), Some(cwd)) => {
            let path = Path::new(operand);
            if path.is_absolute() {
                is_lexical_root(path)
            } else {
                is_lexical_root(&cwd.join(path))
            }
        }
        (Some(operand), None) => is_lexical_root(Path::new(operand)),
    }
}

#[cfg(test)]
mod test {
    use std::ffi::OsStr;
    use std::ffi::OsString;
    use std::path::Path;

    use super::basename;
    use super::classify;
    use super::command_line;
    use super::is_lexical_root;
    use super::starts_at_root;
    use super::Kind;

    #[test]
    fn classifies_tools() {
        assert_eq!(classify(OsStr::new("find")), Some(Kind::Find));
        assert_eq!(classify(OsStr::new("grep")), Some(Kind::Grep));
        assert_eq!(classify(OsStr::new("egrep")), Some(Kind::Grep));
        assert_eq!(classify(OsStr::new("python3.12")), Some(Kind::Python));
        assert_eq!(classify(OsStr::new("pip")), Some(Kind::Python));
        assert_eq!(classify(OsStr::new("pipx")), None);
        assert_eq!(classify(OsStr::new("pytest")), None);
        assert_eq!(classify(OsStr::new("ls")), None);
    }

    #[test]
    fn basenames_paths() {
        assert_eq!(basename(OsStr::new("/a/b/find")), OsStr::new("find"));
        assert_eq!(basename(OsStr::new("find")), OsStr::new("find"));
    }

    #[test]
    fn renders_command_lines() {
        let args: Vec<OsString> =
            ["/", "-name", "x"].iter().map(OsString::from).collect();
        assert_eq!(command_line(OsStr::new("find"), &args), "find / -name x");
    }

    #[test]
    fn detects_lexical_roots() {
        for path in ["/", "//", "/./", "/a/..", "/a/../"] {
            assert!(is_lexical_root(Path::new(path)), "{path}");
        }
        for path in ["/a", "/a/../b", ".", "..", "a/..", "", "/tmp"] {
            assert!(!is_lexical_root(Path::new(path)), "{path}");
        }
    }

    #[test]
    fn resolves_starting_roots_against_the_cwd() {
        // The tools' implicit starting point is the cwd itself.
        assert!(starts_at_root(None, Some(Path::new("/"))));
        assert!(!starts_at_root(None, Some(Path::new("/tmp"))));
        // Relative operands resolve against the cwd.
        assert!(starts_at_root(Some(OsStr::new(".")), Some(Path::new("/"))));
        assert!(starts_at_root(Some(OsStr::new("..")), Some(Path::new("/tmp"))));
        assert!(!starts_at_root(Some(OsStr::new(".")), Some(Path::new("/tmp"))));
        assert!(!starts_at_root(
            Some(OsStr::new("subdir")),
            Some(Path::new("/"))
        ));
        // Absolute operands ignore the cwd; without a cwd, only they
        // can be judged.
        assert!(starts_at_root(Some(OsStr::new("/")), Some(Path::new("/tmp"))));
        assert!(starts_at_root(Some(OsStr::new("/a/..")), None));
        assert!(!starts_at_root(Some(OsStr::new(".")), None));
        assert!(!starts_at_root(None, None));
    }
}
