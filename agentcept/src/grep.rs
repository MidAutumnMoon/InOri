//! `grep` policy: refuse recursive searches rooted at `/`.

use std::ffi::OsStr;
use std::ffi::OsString;
use std::process::ExitCode;

use crate::exec;
use crate::starts_at_root;

/// Short options that take a value, attached (`-A3`) or separate (`-A 3`).
const SHORT_VALUE: &[u8] = b"ABCDdefm";

/// A long option's argument shape and effect on this policy.
#[derive(Clone, Copy)]
enum LongOption {
    Flag,
    Value(ValueEffect),
    OptionalValue,
    Recursive,
    Terminal,
}

/// Policy effect of a required long-option value.
#[derive(Clone, Copy)]
enum ValueEffect {
    Ignored,
    Pattern,
    Directories,
}

/// Why this invocation must not run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Refusal {
    RootSearch,
    LikelyMissingPattern,
}

/// Classify the GNU grep long options this policy understands.
#[must_use]
fn long_option(name: &[u8]) -> Option<LongOption> {
    match name {
        b"basic-regexp"
        | b"binary"
        | b"byte-offset"
        | b"count"
        | b"extended-regexp"
        | b"files-with-matches"
        | b"files-without-match"
        | b"fixed-regexp"
        | b"fixed-strings"
        | b"ignore-case"
        | b"initial-tab"
        | b"invert-match"
        | b"line-buffered"
        | b"line-number"
        | b"line-regexp"
        | b"no-filename"
        | b"no-group-separator"
        | b"no-ignore-case"
        | b"no-messages"
        | b"null"
        | b"null-data"
        | b"only-matching"
        | b"perl-regexp"
        | b"quiet"
        | b"silent"
        | b"text"
        | b"with-filename"
        | b"word-regexp" => Some(LongOption::Flag),
        b"after-context" | b"before-context" | b"binary-files"
        | b"context" | b"devices" | b"exclude" | b"exclude-dir"
        | b"exclude-from" | b"group-separator" | b"include" | b"label"
        | b"max-count" => Some(LongOption::Value(ValueEffect::Ignored)),
        b"regexp" | b"file" => {
            Some(LongOption::Value(ValueEffect::Pattern))
        }
        b"directories" => {
            Some(LongOption::Value(ValueEffect::Directories))
        }
        b"color" | b"colour" => Some(LongOption::OptionalValue),
        b"recursive" | b"dereference-recursive" => {
            Some(LongOption::Recursive)
        }
        b"help" | b"version" => Some(LongOption::Terminal),
        _ => None,
    }
}

/// Decide whether grep would recursively search from `/`.
///
/// Options and operands interleave, the first operand is the pattern
/// unless `-e`/`-f` claimed it, and the last recursion setting wins.
/// Unknown long options fail open because their arity cannot be guessed.
#[must_use]
fn check(
    args: &[OsString],
    cwd: Option<&std::path::Path>,
) -> Option<Refusal> {
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

            let option = long_option(name)?;
            match option {
                LongOption::Terminal => return None,
                LongOption::Flag => {
                    if attached.is_some() {
                        return None;
                    }
                }
                LongOption::OptionalValue => {}
                LongOption::Recursive => {
                    if attached.is_some() {
                        return None;
                    }
                    recursive = true;
                }
                LongOption::Value(effect) => {
                    // argv ran out of the value; real grep errors itself.
                    let value = match attached {
                        Some(value) => value,
                        None => iter.next()?.as_encoded_bytes(),
                    };
                    match effect {
                        ValueEffect::Ignored => {}
                        ValueEffect::Pattern => have_pattern = true,
                        ValueEffect::Directories => {
                            recursive = value == b"recurse";
                        }
                    }
                }
            }
            continue;
        }

        if raw.first() == Some(&b'-') && raw.len() > 1 {
            let (_, mut cluster) = raw.split_at(1);
            // `-NUM` is a context value; later bytes remain options.
            let digits = cluster
                .iter()
                .take_while(|ch| ch.is_ascii_digit())
                .count();
            cluster = cluster.get(digits..)?;

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
                        // Context lines, filters, limits: no policy effect.
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

    if !recursive {
        return None;
    }
    if !have_pattern
        && operands.len() == 1
        && operands
            .first()
            .is_some_and(|operand| operand.as_encoded_bytes() == b"/")
    {
        return Some(Refusal::LikelyMissingPattern);
    }

    let files = if have_pattern {
        operands.as_slice()
    } else {
        operands.split_first()?.1
    };
    let rooted_at_root = files
        .iter()
        .copied()
        .any(|file| starts_at_root(Some(file), cwd))
        || (files.is_empty() && starts_at_root(None, cwd));
    rooted_at_root.then_some(Refusal::RootSearch)
}

/// Policy entry: refuse unbounded recursive searches, else exec real grep.
pub fn run(name: &OsStr, args: &[OsString]) -> ExitCode {
    let cwd = std::env::current_dir().ok();
    if let Some(reason) = check(args, cwd.as_deref()) {
        refuse(name, reason);
        return ExitCode::FAILURE;
    }
    exec::real(name, args)
}

/// Print the reason and the safe command shape.
fn refuse(name: &OsStr, reason: Refusal) {
    let (reason, instruction) = match reason {
        Refusal::RootSearch => (
            "recursive search rooted at `/`",
            "Use a narrower search root:",
        ),
        Refusal::LikelyMissingPattern => (
            "likely missing PATTERN before `/`",
            "Specify both the pattern and search directory:",
        ),
    };
    eprint!(
        "{}",
        indoc::formatdoc! {"
            {name}: blocked: {reason}
            {instruction}
                grep -R PATTERN <dir>
        ",
        name = name.to_string_lossy(),
        }
    );
}

#[cfg(test)]
mod test {
    use std::ffi::OsString;
    use std::path::Path;

    use super::Refusal;
    use super::check;

    fn refusal(args: &[&str], cwd: &str) -> Option<Refusal> {
        let args: Vec<OsString> =
            args.iter().map(OsString::from).collect();
        check(&args, Some(Path::new(cwd)))
    }

    #[test]
    fn finds_explicit_root_targets() {
        assert_eq!(
            refusal(&["-R", "p", "/"], "/tmp"),
            Some(Refusal::RootSearch)
        );
        assert_eq!(
            refusal(&["-R", "-e", "a", "-e", "b", "/"], "/tmp"),
            Some(Refusal::RootSearch)
        );
        assert_eq!(
            refusal(&["-R", "-f", "patterns", "/"], "/tmp"),
            Some(Refusal::RootSearch)
        );
    }

    #[test]
    fn resolves_implicit_target_from_cwd() {
        assert_eq!(
            refusal(&["-R", "pattern"], "/"),
            Some(Refusal::RootSearch)
        );
        assert_eq!(refusal(&["-R", "pattern"], "/tmp"), None);
        // No pattern means real grep errors before searching.
        assert_eq!(refusal(&["-R"], "/"), None);
    }

    #[test]
    fn distinguishes_root_pattern_from_root_target() {
        assert_eq!(refusal(&["/", "-R", "."], "/tmp"), None);
        assert_eq!(
            refusal(&["-R", "/"], "/tmp"),
            Some(Refusal::LikelyMissingPattern)
        );
        assert_eq!(refusal(&["-R", "/", "."], "/tmp"), None);
        assert_eq!(refusal(&["pattern", "/"], "/tmp"), None);
    }

    #[test]
    fn option_values_do_not_become_operands() {
        assert_eq!(
            refusal(
                &[
                    "-R",
                    "-A3",
                    "-B",
                    "2",
                    "--context=2",
                    "--color=always",
                    "-m5",
                    "pattern",
                    "/"
                ],
                "/tmp"
            ),
            Some(Refusal::RootSearch)
        );
        assert_eq!(
            refusal(
                &[
                    "--exclude",
                    "*.o",
                    "--exclude-dir=.git",
                    "-d",
                    "recurse",
                    "pattern",
                    "/"
                ],
                "/tmp"
            ),
            Some(Refusal::RootSearch)
        );
    }

    #[test]
    fn long_options_preserve_the_search_target() {
        assert_eq!(
            refusal(&["--line-number", "-R", "p", "/"], "/tmp"),
            Some(Refusal::RootSearch)
        );
        assert_eq!(
            refusal(&["--fixed-regexp", "-R", "p", "/"], "/tmp"),
            Some(Refusal::RootSearch)
        );
        assert_eq!(
            refusal(&["--group-separator", "SEP", "-R", "p", "/"], "/tmp"),
            Some(Refusal::RootSearch)
        );
    }

    #[test]
    fn numeric_context_prefix_keeps_short_options() {
        assert_eq!(
            refusal(&["-5r", "p", "/"], "/tmp"),
            Some(Refusal::RootSearch)
        );
        assert_eq!(
            refusal(&["-10R", "p", "/"], "/tmp"),
            Some(Refusal::RootSearch)
        );
    }

    #[test]
    fn recursion_setting_uses_the_last_option() {
        assert_eq!(
            refusal(&["-d", "recurse", "p", "/"], "/tmp"),
            Some(Refusal::RootSearch)
        );
        assert_eq!(refusal(&["-r", "-d", "read", "p", "/"], "/tmp"), None);
        assert_eq!(
            refusal(&["-rn", "p", "/"], "/tmp"),
            Some(Refusal::RootSearch)
        );
        assert_eq!(
            refusal(&["--dereference-recursive", "p", "/"], "/tmp"),
            Some(Refusal::RootSearch)
        );
    }

    #[test]
    fn double_dash_makes_the_rest_operands() {
        assert_eq!(
            refusal(&["-R", "p", "--", "/"], "/tmp"),
            Some(Refusal::RootSearch)
        );
        assert_eq!(refusal(&["--", "-R", "p", "/"], "/tmp"), None);
    }

    #[test]
    fn unjudgeable_or_terminal_argv_passes_through() {
        assert_eq!(refusal(&["--frobnicate", "p", "/"], "/tmp"), None);
        assert_eq!(
            refusal(&["-R", "--frobnicate=1", "p", "/"], "/tmp"),
            None
        );
        assert_eq!(refusal(&["-R", "--help"], "/"), None);
        assert_eq!(refusal(&["--version", "p", "/"], "/"), None);
        assert_eq!(refusal(&["-R", "-V"], "/"), None);
        assert_eq!(refusal(&["-nV", "p", "."], "/"), None);
        assert_eq!(refusal(&["--recursive=yes", "p", "/"], "/"), None);
    }
}
