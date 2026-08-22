//! Multicall applet registry and signaling types.

use std::ffi::OsStr;
use std::ffi::OsString;
use std::path::Path;

use crate::BIN_NAME;
use crate::applets;

/// How an applet run can fail; `main` renders both.
#[derive(Debug)]
pub enum RunFailure {
    /// CLI parse exit, including `--help`/`--version`; bpaf renders it.
    Cli(bpaf::ParseFailure),
    /// Runtime failure; rendered as a report.
    Applet(rootcause::Report),
}

/// One multicall applet.
#[derive(Debug)]
pub struct Applet {
    /// Selector: matched against `argv[0]` basename and `{BIN_NAME} NAME`.
    pub name: &'static str,
    /// One-liner shown in the applet listing.
    pub descr: &'static str,
    /// Parse `args` (the applet's own `argv[1..]`), run, and print any output.
    pub run: fn(args: &[OsString]) -> Result<(), RunFailure>,
}

/// All applets, in listing order; names are unique.
pub const APPLETS: &[Applet] = &[
    Applet {
        name: applets::qr::NAME,
        descr: "Generate a QR code from stdin or the clipboard",
        run: applets::qr::applet_main,
    },
    Applet {
        name: applets::uuid7::NAME,
        descr: "Print a freshly generated UUIDv7",
        run: applets::uuid7::applet_main,
    },
    Applet {
        name: applets::completion::NAME,
        descr: "Generate shell completion scripts for applets",
        run: applets::completion::applet_main,
    },
];

fn find_applet(name: &str) -> Option<&'static Applet> {
    APPLETS.iter().find(|applet| applet.name == name)
}

/// Usage text listing all applets.
#[must_use]
pub fn usage() -> String {
    use std::fmt::Write as _;
    let mut out =
        format!("Usage: {BIN_NAME} APPLET [OPTIONS]...\nApplets:\n");
    let width = APPLETS.iter().map(|applet| applet.name.len()).max().unwrap_or(0);
    for applet in APPLETS {
        let _ =
            writeln!(out, "  {:<width$}  {}", applet.name, applet.descr);
    }
    out
}

/// What `main` should do after inspecting `argv`.
#[derive(Debug)]
pub enum Selection<'argv> {
    /// Run `applet` with `args`.
    Run {
        applet: &'argv Applet,
        args: &'argv [OsString],
    },
    /// Print usage to stdout, exit 0.
    Help,
    /// Print the version to stdout, exit 0.
    Version,
    /// Dispatcher invoked with no applet: usage to stderr, exit 1.
    NoApplet,
    /// Unrecognized applet name: error and usage to stderr, exit 1.
    UnknownApplet { name: String },
}

fn resolve_applet(name: String, args: &[OsString]) -> Selection<'_> {
    find_applet(&name).map_or_else(
        || Selection::UnknownApplet { name },
        |applet| Selection::Run { applet, args },
    )
}

/// Multicall dispatch: pick the applet from `argv[0]`, or from the first
/// argument when invoked under the dispatcher name.
#[must_use]
pub fn select<'argv>(
    invoked_as: &'argv OsStr,
    args: &'argv [OsString],
) -> Selection<'argv> {
    let basename = Path::new(invoked_as).file_name().map_or_else(
        || BIN_NAME.to_owned(),
        |n| n.to_string_lossy().into_owned(),
    );

    if basename != BIN_NAME {
        // Multicall name: `argv[0]` selects the applet.
        return resolve_applet(basename, args);
    }

    // Dispatcher name: the first argument selects the applet.
    let Some((first, rest)) = args.split_first() else {
        return Selection::NoApplet;
    };
    match first.to_str() {
        Some("-h" | "--help" | "help") => Selection::Help,
        Some("-V" | "--version" | "version") => Selection::Version,
        _ => resolve_applet(first.to_string_lossy().into_owned(), rest),
    }
}

#[cfg(test)]
#[expect(clippy::panic, reason = "in tests")]
#[expect(clippy::pointer_format, reason = "in tests")]
mod test {
    use super::*;
    use std::assert_matches;

    fn args(items: &[&str]) -> Vec<OsString> {
        items.iter().map(OsString::from).collect()
    }

    #[test]
    fn applet_via_argv0() {
        let arg0 = args(&["-c"]);
        let sel = select(OsStr::new("/usr/bin/qr"), &arg0);
        match sel {
            Selection::Run { applet, args } => {
                assert_eq!(applet.name, "qr");
                assert_eq!(args, &[OsString::from("-c")]);
            }
            other @ (Selection::Help
            | Selection::Version
            | Selection::NoApplet
            | Selection::UnknownApplet { .. }) => {
                panic!("expected Run, got {other:?}")
            }
        }
    }

    #[test]
    fn dispatcher_subcommand() {
        let arg0 = args(&["uuid7"]);
        let sel = select(OsStr::new("derputils"), &arg0);
        match sel {
            Selection::Run { applet, args } => {
                assert_eq!(applet.name, "uuid7");
                assert!(args.is_empty());
            }
            other @ (Selection::Help
            | Selection::Version
            | Selection::NoApplet
            | Selection::UnknownApplet { .. }) => {
                panic!("expected Run, got {other:?}")
            }
        }
    }

    #[test]
    fn dispatcher_without_args_is_no_applet() {
        let arg0 = args(&[]);
        assert_matches!(
            select(OsStr::new("derputils"), &arg0),
            Selection::NoApplet
        );
    }

    #[test]
    fn dispatcher_help_and_version() {
        let help = args(&["--help"]);
        assert_matches!(
            select(OsStr::new("derputils"), &help),
            Selection::Help
        );
        let version = args(&["version"]);
        assert_matches!(
            select(OsStr::new("derputils"), &version),
            Selection::Version
        );
    }

    #[test]
    fn dispatcher_unknown_applet() {
        let arg0 = args(&["nope"]);
        assert_matches!(
            select(OsStr::new("derputils"), &arg0),
            Selection::UnknownApplet { name } if name == "nope"
        );
    }
}
