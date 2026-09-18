//! `python`/`pip` policy: neither exists on this host; `uv` is the tool.

use std::ffi::OsStr;
use std::ffi::OsString;
use std::process::ExitCode;

use crate::command_line;

/// Whether `name` is the `python`/`pip` family: `python`, `python3`,
/// `python3.12`, `pip`, `pip3`, ... Lookalikes with other suffixes
/// (`pipx`, `pytest`) are left alone.
#[must_use]
pub fn is_family(name: &[u8]) -> bool {
    let Some(rest) = name
        .strip_prefix(b"python")
        .or_else(|| name.strip_prefix(b"pip"))
    else {
        return false;
    };
    rest.iter().all(|ch| ch.is_ascii_digit() || *ch == b'.')
}

/// Policy entry: no global python/pip exists; refuse and point at `uv`.
pub fn run(name: &OsStr, args: &[OsString]) -> ExitCode {
    refuse(name, args);
    ExitCode::FAILURE
}

/// Yell at the agent for reaching for a python that isn't there.
fn refuse(name: &OsStr, args: &[OsString]) {
    eprintln!(
        "{}",
        indoc::formatdoc! {"
            agentcept: `{name}` is not available on this machine.
            This host runs Python through uv. Rewrite the invocation:
                python SCRIPT ARGS       =>  uv run SCRIPT ARGS
                python -m MODULE ARGS    =>  uv run -m MODULE ARGS
                python -c 'CODE'         =>  uv run python -c 'CODE'
                pip install PKG          =>  uv pip install PKG
                pip ARGS                 =>  uv pip ARGS
            Command was: {command}
        ",
        name = name.to_string_lossy(),
        command = command_line(name, args),
        }
    );
}

#[cfg(test)]
mod test {
    use super::is_family;

    #[test]
    fn matches_the_family() {
        for name in [
            "python",
            "python3",
            "python3.12",
            "python3.13",
            "pip",
            "pip3",
            "pip3.12",
        ] {
            assert!(is_family(name.as_bytes()), "{name}");
        }
    }

    #[test]
    fn leaves_lookalikes_alone() {
        for name in ["pipx", "pytest", "pythonw", "pipenv", "uv", "sh"] {
            assert!(!is_family(name.as_bytes()), "{name}");
        }
    }
}
