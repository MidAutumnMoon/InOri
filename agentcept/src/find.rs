//! `find` policy: refuse searches starting at `/`.

use std::ffi::OsStr;
use std::ffi::OsString;
use std::process::ExitCode;

use crate::command_line;
use crate::exec;
use crate::starts_at_root;

enum Scan<'args> {
    /// Starting points from argv. Empty means find's default: the cwd.
    Points(Vec<&'args OsStr>),
    /// find exits without searching: help/version, `-D help`.
    Terminal,
    /// Starting points are not on argv (`-files0-from`).
    Opaque,
}

/// Extract find's starting points: options first, then every operand
/// up to the first expression token. An unrecognized leading option is
/// an unknown predicate — find rejects the argv without searching —
/// so the points so far are all there are.
#[must_use]
fn scan(args: &[OsString]) -> Scan<'_> {
    let mut index = 0;

    // Options must precede any path.
    while let Some(arg) = args.get(index) {
        let raw = arg.as_encoded_bytes();
        match raw {
            b"-H" | b"-L" | b"-P" => index += 1,
            b"--help" | b"-help" | b"--version" | b"-version" => {
                return Scan::Terminal;
            }
            // `-D help` explains and exits.
            b"-D" => {
                let explains = args.get(index + 1).is_some_and(|list| {
                    list.as_encoded_bytes()
                        .split(|ch| *ch == b',')
                        .any(|opt| opt == b"help")
                });
                if explains {
                    return Scan::Terminal;
                }
                index += 2;
            }
            level if is_olevel(level) => index += 1,
            _ if raw.starts_with(b"-files0-from") => return Scan::Opaque,
            _ => break,
        }
    }

    let points = args
        .iter()
        .skip(index)
        .take_while(|arg| {
            let raw = arg.as_encoded_bytes();
            raw.first() != Some(&b'-') && raw != b"(" && raw != b"!"
        })
        .map(OsString::as_os_str)
        .collect();

    Scan::Points(points)
}

/// `-Olevel` is written with the level attached (e.g. `-O3`).
fn is_olevel(raw: &[u8]) -> bool {
    let Some(level) = raw.strip_prefix(b"-O") else {
        return false;
    };
    !level.is_empty() && level.iter().all(u8::is_ascii_digit)
}

/// Policy entry: refuse unbounded starting points, else exec real `find`.
pub fn run(name: &OsStr, args: &[OsString]) -> ExitCode {
    let points = match scan(args) {
        Scan::Points(points) => points,
        Scan::Terminal | Scan::Opaque => return exec::real(name, args),
    };
    let cwd = std::env::current_dir().ok();
    let unbounded = points
        .iter()
        .copied()
        .any(|point| starts_at_root(Some(point), cwd.as_deref()))
        || (points.is_empty() && starts_at_root(None, cwd.as_deref()));
    if unbounded {
        refuse(name, args);
        return ExitCode::FAILURE;
    }
    exec::real(name, args)
}

/// Yell at the agent for trying to search the whole filesystem.
fn refuse(name: &OsStr, args: &[OsString]) {
    eprintln!(
        "{}",
        indoc::formatdoc! {"
            agentcept: refusing `{name}`: a starting point is `/`
            Searching the whole filesystem is never the right call. Scope it:
                find <dir> -name 'PATTERN'
                find . -maxdepth 3 -type f
            Command was: {command}
        ",
        name = name.to_string_lossy(),
        command = command_line(name, args),
        }
    );
}

#[cfg(test)]
#[expect(clippy::panic, reason = "in tests")]
mod test {
    use std::ffi::OsString;

    use super::Scan;
    use super::scan;

    /// Starting points for the given argv, as strings.
    fn points(args: &[&str]) -> Vec<String> {
        let args: Vec<OsString> =
            args.iter().map(OsString::from).collect();
        let Scan::Points(points) = scan(&args) else {
            panic!("expected starting points, got Opaque");
        };
        points
            .iter()
            .map(|point| point.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn root_is_a_starting_point() {
        assert_eq!(points(&["/", "file"]), ["/", "file"]);
    }

    #[test]
    fn expression_is_not_a_starting_point() {
        // `-path /` belongs to the expression, not the starting points.
        assert_eq!(points(&[".", "-path", "/"]), ["."]);
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
        ] {
            let args: Vec<OsString> =
                args.iter().map(OsString::from).collect();
            assert!(matches!(scan(&args), Scan::Terminal), "{args:?}");
        }
    }

    #[test]
    fn other_debug_lists_still_search() {
        assert_eq!(points(&["-D", "rates", "/tmp"]), ["/tmp"]);
    }

    #[test]
    fn real_options_precede_paths() {
        assert_eq!(
            points(&[
                "-H", "-L", "-O2", "-D", "rates", "/tmp", "(", "-true"
            ]),
            ["/tmp"]
        );
    }

    #[test]
    fn no_points_means_cwd() {
        assert_eq!(points(&["-name", "x"]), Vec::<String>::new());
        assert_eq!(points(&[]), Vec::<String>::new());
    }

    #[test]
    fn expression_may_start_with_paren_or_bang() {
        assert_eq!(points(&["(", "-name", "x"]), Vec::<String>::new());
        assert_eq!(points(&["!", "-name", "x"]), Vec::<String>::new());
        assert_eq!(points(&["/tmp", "!", "-name", "x"]), ["/tmp"]);
    }

    #[test]
    fn files0_from_hides_the_starting_points() {
        let args: Vec<OsString> =
            ["-files0-from", "-"].iter().map(OsString::from).collect();
        assert!(matches!(scan(&args), Scan::Opaque));
    }

    #[test]
    fn unknown_leading_option_ends_the_paths() {
        // Real find errors on it before searching anywhere.
        assert_eq!(points(&["-frobnicate", "/"]), Vec::<String>::new());
    }
}
