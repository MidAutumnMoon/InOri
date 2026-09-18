//! `find` policy: refuse searches starting at `/`.

use std::ffi::OsStr;
use std::ffi::OsString;
use std::process::ExitCode;

use crate::exec;
use crate::starts_at_root;

/// Returns whether `find` would start searching from `/`.
///
/// Parsing stops at the first expression token because later `--help` and
/// `-files0-from` tokens may be predicate or action arguments.
#[must_use]
fn check(args: &[OsString], cwd: Option<&std::path::Path>) -> bool {
    let mut index = 0;

    // find's global options come before its starting points.
    while let Some(arg) = args.get(index) {
        let raw = arg.as_encoded_bytes();
        match raw {
            b"-H" | b"-L" | b"-P" => index += 1,
            b"--" => {
                index += 1;
                break;
            }
            b"--help" | b"-help" | b"--version" | b"-version" => {
                return false;
            }
            b"-D" => {
                let Some(list) = args.get(index + 1) else {
                    return false;
                };
                if list
                    .as_encoded_bytes()
                    .split(|ch| *ch == b',')
                    .any(|opt| opt == b"help")
                {
                    return false;
                }
                index += 2;
            }
            level if is_olevel(level) => index += 1,
            // Let find reject malformed options before searching.
            level if level.starts_with(b"-O") => return false,
            _ => break,
        }
    }

    let first_point = index;
    while let Some(arg) = args.get(index) {
        let raw = arg.as_encoded_bytes();
        if raw.first() == Some(&b'-') || raw == b"(" || raw == b"!" {
            break;
        }
        index += 1;
    }
    let Some(points) = args.get(first_point..index) else {
        return false;
    };

    // These expressions either exit or obtain starting points elsewhere.
    if let Some(expression) = args.get(index) {
        let raw = expression.as_encoded_bytes();
        if matches!(raw, b"--help" | b"-help" | b"--version" | b"-version")
            || raw.starts_with(b"-files0-from")
        {
            return false;
        }
    }

    points
        .iter()
        .map(OsString::as_os_str)
        .any(|point| starts_at_root(Some(point), cwd))
        || (points.is_empty() && starts_at_root(None, cwd))
}

// find accepts optimization levels only in attached form, such as `-O3`.
fn is_olevel(raw: &[u8]) -> bool {
    let Some(level) = raw.strip_prefix(b"-O") else {
        return false;
    };
    !level.is_empty() && level.iter().all(u8::is_ascii_digit)
}

pub fn run(name: &OsStr, args: &[OsString]) -> ExitCode {
    let cwd = std::env::current_dir().ok();
    if check(args, cwd.as_deref()) {
        refuse(name);
        return ExitCode::FAILURE;
    }
    exec::real(name, args)
}

fn refuse(name: &OsStr) {
    eprint!(
        "{}",
        indoc::formatdoc! {"
            {name}: blocked: a starting point is `/`
            Use a narrower starting point:
                find <dir> -name 'PATTERN'
                find . -maxdepth 3 -type f
        ",
        name = name.to_string_lossy(),
        }
    );
}

#[cfg(test)]
mod test {
    use std::ffi::OsString;
    use std::path::Path;

    use super::check;

    fn refused(args: &[&str], cwd: &str) -> bool {
        let args: Vec<OsString> =
            args.iter().map(OsString::from).collect();
        check(&args, Some(Path::new(cwd)))
    }

    #[test]
    fn finds_explicit_root_points() {
        assert!(refused(&["/", "file"], "/tmp"));
        assert!(refused(&["/tmp", "/", "file"], "/tmp"));
        assert!(!refused(&["/tmp"], "/tmp"));
    }

    #[test]
    fn expression_arguments_are_not_points() {
        assert!(!refused(&[".", "-path", "/"], "/tmp"));
        assert!(refused(&["/", "-path", "."], "/tmp"));
    }

    #[test]
    fn terminal_options_do_not_search() {
        for args in [
            vec!["--help"],
            vec!["-help"],
            vec!["--version"],
            vec!["-version"],
            vec!["-D", "help"],
            vec!["-D", "exec,help"],
            vec!["/", "--help"],
        ] {
            assert!(!refused(&args, "/"), "{args:?}");
        }
    }

    #[test]
    fn real_options_precede_points() {
        assert!(refused(&["-D", "rates", "/"], "/tmp"));
        assert!(refused(
            &["-H", "-L", "-O2", "-D", "rates", "/", "(", "-true"],
            "/tmp"
        ));
        // Let find reject malformed options before searching.
        assert!(!refused(&["-D"], "/"));
        assert!(!refused(&["-Oinvalid", "/"], "/"));
    }

    #[test]
    fn double_dash_ends_leading_options() {
        assert!(refused(&["--", "/", "-name", "x"], "/tmp"));
    }

    #[test]
    fn no_points_means_cwd() {
        assert!(refused(&["-name", "x"], "/"));
        assert!(refused(&[], "/"));
        assert!(!refused(&["-name", "x"], "/tmp"));
        assert!(!refused(&[], "/tmp"));
    }

    #[test]
    fn expression_may_start_with_paren_or_bang() {
        assert!(refused(&["(", "-name", "x"], "/"));
        assert!(refused(&["!", "-name", "x"], "/"));
        assert!(!refused(&["/tmp", "!", "-name", "x"], "/tmp"));
    }

    #[test]
    fn opaque_points_pass_through() {
        assert!(!refused(&["-files0-from", "-"], "/"));
        assert!(!refused(&["/", "-files0-from", "/dev/null"], "/tmp"));
        // Let find reject the invalid attached form.
        assert!(!refused(&["-files0-from=/dev/null"], "/"));
    }
}
