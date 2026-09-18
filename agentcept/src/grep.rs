//! `grep` policy: refuse recursive searches rooted at `/`.

use std::ffi::OsStr;
use std::ffi::OsString;
use std::process::ExitCode;

use crate::command_line;
use crate::exec;
use crate::starts_at_root;

/// Long options whose value must be attached (`--name=value`)
/// or given as the next argument.
const LONG_VALUE: &[&[u8]] = &[
    b"after-context",
    b"before-context",
    b"binary-files",
    b"context",
    b"devices",
    b"directories",
    b"exclude",
    b"exclude-dir",
    b"exclude-from",
    b"file",
    b"include",
    b"label",
    b"max-count",
    b"regexp",
];

/// Long options with an optional value: only the attached form is one.
const LONG_OPTIONAL_VALUE: &[&[u8]] = &[b"color", b"colour"];

/// Short options that take a value, attached (`-A3`) or separate (`-A 3`).
const SHORT_VALUE: &[u8] = b"ABCDdefm";

struct Scan<'args> {
    /// Whether the search descends into directories.
    recursive: bool,
    /// Operands naming search targets; the PATTERN operand is excluded.
    files: Vec<&'args OsStr>,
    /// The `grep -R /` shape: the sole operand became the pattern.
    /// A forgotten pattern, not a search for `/` — refuse on any cwd.
    root_sole_operand: bool,
}

/// Find grep's search targets: options and operands interleave, the
/// first operand is the PATTERN unless `-e`/`-f` claimed it, and the
/// last recursion setting wins.
///
/// `None` = unjudgeable argv; the caller fails open: an unknown long
/// option (its arity would be a guess), or terminal help/version
/// (grep exits without searching).
#[must_use]
fn scan(args: &[OsString]) -> Option<Scan<'_>> {
    let mut recursive = false;
    let mut have_pattern = false;
    let mut operands: Vec<&OsStr> = Vec::new();

    let mut iter = args.iter().map(OsString::as_os_str);
    while let Some(arg) = iter.next() {
        let raw = arg.as_encoded_bytes();

        if raw == b"--" {
            operands.extend(&mut iter);
            break;
        }

        if raw.starts_with(b"--") {
            let (_, body) = raw.split_at(2);
            let (name, attached) = body
                .iter()
                .position(|ch| *ch == b'=')
                .map_or((body, None), |at| {
                    let (name, rest) = body.split_at(at);
                    let (_, value) = rest.split_at(1); // drop `=`
                    (name, Some(value))
                });

            if LONG_VALUE.contains(&name) {
                // argv ran out of the value; real grep errors itself.
                let value = match attached {
                    Some(value) => value,
                    None => iter.next()?.as_encoded_bytes(),
                };
                match name {
                    b"regexp" | b"file" => have_pattern = true,
                    b"directories" => recursive = value == b"recurse",
                    // Context lines, filters, limits: no policy effect.
                    _ => {}
                }
            } else if name == b"help" || name == b"version" {
                // Terminal, wherever they appear.
                return None;
            } else if LONG_OPTIONAL_VALUE.contains(&name) {
            } else if name == b"recursive" || name == b"dereference-recursive" {
                recursive = true;
            } else {
                // Unknown option: don't guess whether it takes a value.
                return None;
            }
            continue;
        }

        if raw.first() == Some(&b'-') && raw.len() > 1 {
            let (_, mut cluster) = raw.split_at(1);
            // `-NUM` is the context option with its value attached.
            if cluster.first().is_some_and(u8::is_ascii_digit) {
                continue;
            }
            while let Some((ch, rest)) = cluster.split_first() {
                if *ch == b'V' {
                    // `-V` is `--version`; lowercase `-v` is invert.
                    return None;
                }
                if SHORT_VALUE.contains(ch) {
                    let value = if rest.is_empty() {
                        iter.next()?.as_encoded_bytes()
                    } else {
                        rest
                    };
                    match *ch {
                        b'e' | b'f' => have_pattern = true,
                        b'd' => recursive = value == b"recurse",
                        // Context lines, devices: no policy effect.
                        _ => {}
                    }
                    break;
                }
                if *ch == b'r' || *ch == b'R' {
                    recursive = true;
                }
                cluster = rest;
            }
            continue;
        }

        operands.push(arg);
    }

    let root_sole_operand = !have_pattern
        && operands.len() == 1
        && operands.first().is_some_and(|only| only.as_encoded_bytes() == b"/");

    let mut files = operands;
    if !have_pattern && !files.is_empty() {
        files.remove(0);
    }
    Some(Scan {
        recursive,
        files,
        root_sole_operand,
    })
}

/// Policy entry: refuse unbounded recursive searches, else exec real grep.
pub fn run(name: &OsStr, args: &[OsString]) -> ExitCode {
    let Some(scan) = scan(args) else {
        return exec::real(name, args);
    };
    let cwd = std::env::current_dir().ok();
    // With no file operand, recursive grep scans the working directory.
    let rooted_at_root = scan.root_sole_operand
        || scan
            .files
            .iter()
            .copied()
            .any(|file| starts_at_root(Some(file), cwd.as_deref()))
        || (scan.files.is_empty() && starts_at_root(None, cwd.as_deref()));
    if scan.recursive && rooted_at_root {
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
            agentcept: refusing `{name}`: recursive search rooted at `/`
            Searching the whole filesystem is never the right call. Scope it:
                grep -R PATTERN <dir>
            Command was: {command}
        ",
        name = name.to_string_lossy(),
        command = command_line(name, args),
        }
    );
}

#[cfg(test)]
mod test {
    use std::ffi::OsString;

    use super::scan;

    /// `(recursive, files, root_sole_operand)` for the given argv.
    fn parsed(args: &[&str]) -> Option<(bool, Vec<String>, bool)> {
        let args: Vec<OsString> = args.iter().map(OsString::from).collect();
        scan(&args).map(|parsed| {
            (
                parsed.recursive,
                parsed
                    .files
                    .iter()
                    .map(|file| file.to_string_lossy().into_owned())
                    .collect(),
                parsed.root_sole_operand,
            )
        })
    }

    #[test]
    fn first_operand_is_the_pattern() {
        // `grep -R PATTERN /`: the only file operand is the root.
        assert_eq!(parsed(&["-R", "foo", "/"]), Some((true, vec!["/".into()], false)));
        assert_eq!(
            parsed(&["-R", "foo", ".", "/"]),
            Some((true, vec![".".into(), "/".into()], false))
        );
    }

    #[test]
    fn root_pattern_is_not_a_target() {
        // `grep / -R .`: the pattern is `/`, the search starts at `.`.
        assert_eq!(parsed(&["/", "-R", "."]), Some((true, vec![".".into()], false)));
    }

    #[test]
    fn sole_root_operand_scans_cwd() {
        // `grep -R /` has `/` as the pattern and scans the cwd: the agent
        // meant a search rooted at `/`.
        assert_eq!(parsed(&["-R", "/"]), Some((true, Vec::<String>::new(), true)));
        // With a second operand, `/` is a legit pattern.
        assert_eq!(parsed(&["-R", "/", "."]), Some((true, vec![".".into()], false)));
    }

    #[test]
    fn non_recursive_root_is_not_a_scan() {
        // Reads `/` as one file and fails; no search to refuse.
        assert_eq!(parsed(&["foo", "/"]), Some((false, vec!["/".into()], false)));
    }

    #[test]
    fn e_and_f_move_the_pattern_out() {
        assert_eq!(parsed(&["-R", "-e", "a", "-e", "b", "/"]), Some((true, vec!["/".into()], false)));
        assert_eq!(parsed(&["-R", "-f", "pats", "/"]), Some((true, vec!["/".into()], false)));
        assert_eq!(
            parsed(&["-R", "--regexp=/", "."]),
            Some((true, vec![".".into()], false))
        );
    }

    #[test]
    fn values_never_become_operands() {
        assert_eq!(
            parsed(&["-A3", "-B", "2", "--context=2", "--color=always", "-m5", "p", "/etc"]),
            Some((false, vec!["/etc".into()], false))
        );
        assert_eq!(
            parsed(&["--exclude", "*.o", "--exclude-dir=.git", "-d", "skip", "p", "/etc"]),
            Some((false, vec!["/etc".into()], false))
        );
        assert_eq!(parsed(&["-3", "p", "f"]), Some((false, vec!["f".into()], false)));
    }

    #[test]
    fn recursion_is_one_setting_last_wins() {
        assert_eq!(parsed(&["-d", "recurse", "p", "/"]), Some((true, vec!["/".into()], false)));
        assert_eq!(parsed(&["-r", "-d", "read", "p", "/"]), Some((false, vec!["/".into()], false)));
        assert_eq!(parsed(&["-rn", "p", "."]), Some((true, vec![".".into()], false)));
        assert_eq!(
            parsed(&["--dereference-recursive", "p", "."]),
            Some((true, vec![".".into()], false))
        );
    }

    #[test]
    fn double_dash_makes_operands() {
        assert_eq!(parsed(&["-R", "p", "--", "/"]), Some((true, vec!["/".into()], false)));
        assert_eq!(
            parsed(&["--", "-R", "p", "/"]),
            Some((false, vec!["p".into(), "/".into()], false))
        );
    }

    #[test]
    fn unjudgeable_argv_fails_open() {
        // Unknown options: arity can't be guessed.
        assert_eq!(parsed(&["--frobnicate", "p", "/"]), None);
        assert_eq!(parsed(&["-R", "--frobnicate=1", "p", "/"]), None);
        // Terminal options: real grep exits without searching.
        assert_eq!(parsed(&["-R", "--help"]), None);
        assert_eq!(parsed(&["--version", "p", "/"]), None);
        assert_eq!(parsed(&["-R", "-V"]), None);
        assert_eq!(parsed(&["-nV", "p", "."]), None);
    }
}
