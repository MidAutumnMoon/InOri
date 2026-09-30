//! `grep` policy: refuse recursive searches rooted at `/`.

use std::ffi::OsStr;
use std::ffi::OsString;
use std::process::ExitCode;

use crate::exec;
use crate::starts_at_root;

/// Short options that take a value, attached (`-A3`) or separate (`-A 3`).
const SHORT_VALUE: &[u8] = b"ABCDXdefm";

/// GNU grep's value-less short options. Anything else is unknown to GNU
/// and belongs on the real tool, whose rejection is part of today's
/// behavior; a replacement may instead accept it as its own flag.
const SHORT_FLAGS: &[u8] = b"EFGHILPRTUZabchilnoqrsvwxyz";

#[derive(Clone, Copy)]
enum LongOption {
    Flag,
    Value(ValueEffect),
    OptionalValue,
    Recursive,
    Terminal,
}

#[derive(Clone, Copy)]
enum ValueEffect {
    Ignored,
    Pattern,
    Directories,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Refusal {
    RootSearch,
    LikelyMissingPattern,
}

/// What `run` should do with a checked argv.
#[derive(Debug, Eq, PartialEq)]
enum Outcome {
    /// Refuse the call.
    Refuse(Refusal),
    /// Run the real tool: terminal options, argv the parser cannot judge,
    /// or a directory operand without recursion — calls where a
    /// replacement engine would behave differently from GNU grep.
    Real,
    /// Prefer the replacement engine, falling back to the real tool.
    Prefer,
}

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

/// Checks `args` against the policy and picks the execution outcome.
///
/// Options and operands can interleave. The first unclaimed operand is the
/// pattern, and later recursion options override earlier ones. Argument
/// shapes the parser cannot judge stay on the real tool, whose rejection
/// of them is part of today's behavior; so do options whose GNU meaning a
/// replacement engine would not preserve. Refusals always win over
/// routing.
#[must_use]
fn check(args: &[OsString], cwd: Option<&std::path::Path>) -> Outcome {
    let mut recursive = false;
    let mut have_pattern = false;
    // Options that GNU accepts but a replacement would serve differently;
    // set during the scan, honored only after the refusal checks.
    let mut needs_gnu = false;
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

            let Some(option) = long_option(name) else {
                return Outcome::Real;
            };
            match option {
                LongOption::Terminal => return Outcome::Real,
                LongOption::Flag => {
                    if attached.is_some() {
                        return Outcome::Real;
                    }
                }
                LongOption::OptionalValue => {}
                LongOption::Recursive => {
                    if attached.is_some() {
                        return Outcome::Real;
                    }
                    recursive = true;
                }
                LongOption::Value(effect) => {
                    // Let grep report a missing option value.
                    let value = if let Some(value) = attached {
                        value
                    } else {
                        let Some(value) = iter.next() else {
                            return Outcome::Real;
                        };
                        value.as_encoded_bytes()
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
            cluster = cluster.get(digits..).unwrap_or_default();

            while let Some((ch, rest)) = cluster.split_first() {
                if *ch == b'V' {
                    // `-V` is `--version`; lowercase `-v` is invert.
                    return Outcome::Real;
                }
                if SHORT_VALUE.contains(ch) {
                    let value = if rest.is_empty() {
                        let Some(value) = iter.next() else {
                            return Outcome::Real;
                        };
                        value.as_encoded_bytes()
                    } else {
                        rest
                    };
                    match *ch {
                        b'e' | b'f' => have_pattern = true,
                        b'd' => recursive = value == b"recurse",
                        // `-X` selects a matcher dialect; a replacement
                        // reads the option as something else entirely.
                        b'X' => needs_gnu = true,
                        // Context lines, filters, limits: no policy effect.
                        _ => {}
                    }
                    break;
                }
                if !SHORT_FLAGS.contains(ch) {
                    // Unknown to GNU grep: let it reject the option.
                    return Outcome::Real;
                }
                if *ch == b'y' {
                    // GNU treats the obsolete `-y` as `-i`; a replacement
                    // may read it as a different option.
                    needs_gnu = true;
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

    // All operands name files when the pattern came from `-e`/`-f`;
    // otherwise the first operand is the pattern. No operand left means
    // grep reports a usage error.
    let files: &[&OsStr] = if have_pattern {
        &operands
    } else if let Some((_, rest)) = operands.split_first() {
        rest
    } else {
        return Outcome::Real;
    };

    if recursive {
        // A lone `/` is parsed as the pattern but usually means the
        // pattern was omitted from a root scan.
        if !have_pattern
            && operands.len() == 1
            && operands
                .first()
                .is_some_and(|first| first.as_encoded_bytes() == b"/")
        {
            return Outcome::Refuse(Refusal::LikelyMissingPattern);
        }
        let rooted_at_root = files
            .iter()
            .copied()
            .any(|file| starts_at_root(Some(file), cwd))
            || (files.is_empty() && starts_at_root(None, cwd));
        if rooted_at_root {
            return Outcome::Refuse(Refusal::RootSearch);
        }
        // A `-` operand means stdin, which GNU reads alone; a
        // replacement also scans the working tree.
        if files.iter().any(|file| file.as_encoded_bytes() == b"-") {
            return Outcome::Real;
        }
    }

    // ugrep searches directory operands where GNU errors (or skips them
    // with `-d skip`); keep those calls on the real tool.
    if !recursive && files.iter().copied().any(is_directory) {
        return Outcome::Real;
    }

    if needs_gnu {
        return Outcome::Real;
    }
    Outcome::Prefer
}

fn is_directory(operand: &OsStr) -> bool {
    std::fs::metadata(operand).is_ok_and(|meta| meta.is_dir())
}

/// The preferred grep replacement: `AGENTCEPT_UGREP_PATH`, else the
/// build-time `CFG_UGREP_PATH` pin. An empty override counts as unset.
fn replacement() -> Option<OsString> {
    std::env::var_os("AGENTCEPT_UGREP_PATH")
        .filter(|value| !value.is_empty())
        .or_else(|| option_env!("CFG_UGREP_PATH").map(OsString::from))
}

pub fn run(name: &OsStr, args: &[OsString]) -> ExitCode {
    let cwd = std::env::current_dir().ok();
    match check(args, cwd.as_deref()) {
        Outcome::Refuse(reason) => {
            refuse(name, reason);
            ExitCode::FAILURE
        }
        Outcome::Real => exec::real(name, args),
        Outcome::Prefer => {
            exec::prefer(replacement().as_deref(), name, args)
        }
    }
}

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

    use super::Outcome;
    use super::Refusal;
    use super::check;

    fn refusal(args: &[&str], cwd: &str) -> Option<Refusal> {
        let args: Vec<OsString> =
            args.iter().map(OsString::from).collect();
        match check(&args, Some(Path::new(cwd))) {
            Outcome::Refuse(reason) => Some(reason),
            Outcome::Real | Outcome::Prefer => None,
        }
    }

    fn outcome(args: &[&str], cwd: &str) -> Outcome {
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
        // Let grep reject a missing pattern before searching.
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
    fn unjudgeable_or_terminal_argv_stays_on_the_real_tool() {
        assert_eq!(
            outcome(&["--frobnicate", "p", "/"], "/tmp"),
            Outcome::Real
        );
        assert_eq!(
            outcome(&["-R", "--frobnicate=1", "p", "/"], "/tmp"),
            Outcome::Real
        );
        assert_eq!(
            outcome(&["--recursive=yes", "p", "/"], "/tmp"),
            Outcome::Real
        );
        for args in [
            vec!["--help"],
            vec!["-V"],
            vec!["--version", "p", "/"],
            vec!["-nV", "p", "."],
        ] {
            assert_eq!(outcome(&args, "/tmp"), Outcome::Real, "{args:?}");
        }
    }

    #[test]
    fn directory_operands_without_recursion_run_the_real_tool() {
        // `.` relative to this test's working directory is a directory.
        assert_eq!(outcome(&["p", "."], "/tmp"), Outcome::Real);
        assert_eq!(
            outcome(&["-d", "skip", "p", "."], "/tmp"),
            Outcome::Real
        );
        // Recursive searches of directories are the fast path.
        assert_eq!(outcome(&["-r", "p", "."], "/tmp"), Outcome::Prefer);
    }

    #[test]
    fn the_obsolete_y_option_runs_the_real_tool() {
        // GNU treats `-y` as `-i`; ugrep reads it as `--any-line`.
        assert_eq!(outcome(&["-y", "p", "x"], "/tmp"), Outcome::Real);
    }

    #[test]
    fn refusals_win_over_option_gates() {
        // `-y` routes to the real tool, but only after the policy has
        // had its say — GNU would search, so the refusal must fire.
        assert_eq!(
            refusal(&["-R", "-y", "p", "/"], "/tmp"),
            Some(Refusal::RootSearch)
        );
    }

    #[test]
    fn short_options_unknown_to_gnu_run_the_real_tool() {
        // GNU rejects these; a replacement may accept them as its own.
        for args in [
            vec!["-j", "p", "x"],
            vec!["-u", "p", "x"],
            vec!["-X", "emacs", "p", "x"],
            vec!["-nj", "p", "x"],
        ] {
            assert_eq!(outcome(&args, "/tmp"), Outcome::Real, "{args:?}");
        }
    }

    #[test]
    fn a_stdin_operand_with_recursion_runs_the_real_tool() {
        // GNU reads stdin alone; a replacement also scans the tree.
        assert_eq!(outcome(&["-r", "p", "-"], "/tmp"), Outcome::Real);
        assert_eq!(outcome(&["-r", "p", "-", "."], "/tmp"), Outcome::Real);
        // Without recursion, stdin stays on the fast path.
        assert_eq!(outcome(&["p", "-"], "/tmp"), Outcome::Prefer);
    }

    #[test]
    fn plain_searches_prefer_the_replacement() {
        assert_eq!(outcome(&["p"], "/tmp"), Outcome::Prefer);
        assert_eq!(
            outcome(&["p", "definitely-missing-file"], "/tmp"),
            Outcome::Prefer
        );
        assert_eq!(
            outcome(&["-R", "p", "missing-dir"], "/tmp"),
            Outcome::Prefer
        );
    }
}
