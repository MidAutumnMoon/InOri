//! `python`/`pip` policy: neither exists on this host; `uv` is the tool.

use std::ffi::OsStr;
use std::process::ExitCode;

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
pub fn run(name: &OsStr) -> ExitCode {
    refuse(name);
    ExitCode::FAILURE
}

/// Print the reason and the relevant `uv` command shapes.
fn refuse(name: &OsStr) {
    let pip = name.as_encoded_bytes().starts_with(b"pip");
    let name = name.to_string_lossy();

    if pip {
        eprint!(
            "{}",
            indoc::formatdoc! {"
                {name}: global pip is unavailable; use `uv pip`
                    uv pip ARGS
            "}
        );
    } else {
        eprint!(
            "{}",
            indoc::formatdoc! {"
                {name}: global Python is unavailable; use `uv`
                    script:  uv run SCRIPT [ARGS...]
                    module:  uv run -m MODULE [ARGS...]
                    inline:  uv run python -c 'CODE'
            "}
        );
    }
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
