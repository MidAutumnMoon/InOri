//! Multicall applet registry and signaling types.

use std::ffi::OsStr;
use std::ffi::OsString;
use std::path::Path;

use crate::BIN_NAME;
use crate::applets;

/// What an applet reports to the dispatcher on success.
#[derive(Debug)]
pub enum Outcome {
    /// Finished; nothing further to report.
    Quiet,
    /// Finished; `main` prints this human notice to stderr.
    Notice(String),
}

/// Successful run result of an applet.
pub type AppletResult = rootcause::Result<Outcome>;

/// Failure that `main` must interpret.
#[derive(Debug)]
pub enum RunFailure {
    /// CLI parse/help/version exit — bpaf renders the message.
    Cli(bpaf::ParseFailure),
    /// Applet runtime failure.
    Applet(rootcause::Report),
}

/// One multicall applet.
#[derive(Debug)]
pub struct Applet {
    /// Selector: matched against `argv[0]` basename and `{BIN_NAME} NAME`.
    pub name: &'static str,
    /// One-line description for the applet listing.
    pub descr: &'static str,
    /// Parse `args` (applet's own `argv[1..]`) and run.
    pub run: fn(args: &[OsString]) -> Result<Outcome, RunFailure>,
}

// Manual: fn pointers are not meaningfully comparable (addresses can vary
// between codegen units), so equality is defined on the identity fields only.
impl PartialEq for Applet {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.descr == other.descr
    }
}

impl Eq for Applet {}

/// All applets, in listing order.
pub const APPLETS: &[Applet] = &[
    Applet {
        name: applets::quraa::NAME,
        descr: "Generate a QR code from stdin or the clipboard",
        run: applets::quraa::applet_main,
    },
    Applet {
        name: applets::uuid7::NAME,
        descr: "Print a freshly generated UUIDv7",
        run: applets::uuid7::applet_main,
    },
];

fn find_applet(name: &str) -> Option<&'static Applet> {
    APPLETS.iter().find(|a| a.name == name)
}

/// Top-level usage text listing all applets.
#[must_use]
pub fn usage() -> String {
    use std::fmt::Write;
    let mut out =
        format!("Usage: {BIN_NAME} APPLET [OPTIONS]...\nApplets:\n");
    for a in APPLETS {
        let _ = writeln!(out, "  {:<8}{}", a.name, a.descr);
    }
    out
}

/// What `main` should do after inspecting `argv`.
#[derive(Debug, PartialEq, Eq)]
pub enum Selection<'a> {
    /// Run `applet` with the given argument slice.
    Run {
        applet: &'a Applet,
        args: &'a [OsString],
    },
    /// Print the top-level usage listing (stdout, exit 0).
    Help,
    /// Print the version (stdout, exit 0).
    Version,
    /// Dispatcher name invoked with no applet at all (usage to stderr, exit 1).
    NoApplet,
    /// Neither a known applet name nor the dispatcher name.
    UnknownApplet { name: String },
}

/// Multicall dispatch: decide what to run from `argv[0]` and the raw args.
#[must_use]
pub fn select<'a>(
    invoked_as: &'a OsStr,
    args: &'a [OsString],
) -> Selection<'a> {
    let basename = Path::new(invoked_as).file_name().map_or_else(
        || BIN_NAME.to_owned(),
        |n| n.to_string_lossy().into_owned(),
    );

    if basename == BIN_NAME {
        // Dispatcher name: the first argument selects the applet.
        let Some(first) = args.first() else {
            return Selection::NoApplet;
        };
        match first.to_str() {
            Some("-h" | "--help" | "help") => Selection::Help,
            Some("-V" | "--version" | "version") => Selection::Version,
            Some(name) => find_applet(name).map_or_else(
                || Selection::UnknownApplet {
                    name: name.to_owned(),
                },
                |applet| Selection::Run {
                    applet,
                    args: args.get(1..).unwrap_or(&[]),
                },
            ),
            None => Selection::UnknownApplet {
                name: first.to_string_lossy().into_owned(),
            },
        }
    } else {
        // Multicall name: `argv[0]` is the applet.
        find_applet(&basename).map_or_else(
            || Selection::UnknownApplet { name: basename },
            |applet| Selection::Run { applet, args },
        )
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    use super::*;

    fn args(items: &[&str]) -> Vec<OsString> {
        items.iter().map(OsString::from).collect()
    }

    #[test]
    fn applet_via_argv0() {
        let arg0 = args(&[]);
        let sel = select(&OsStr::new("quraa"), &arg0);
        match sel {
            Selection::Run { applet, args } => {
                assert_eq!(applet.name, "quraa");
                assert!(args.is_empty());
            }
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn applet_via_argv0_with_path() {
        let arg0 = args(&["-c"]);
        let sel = select(&OsStr::new("/usr/bin/quraa"), &arg0);
        match sel {
            Selection::Run { applet, args } => {
                assert_eq!(applet.name, "quraa");
                assert_eq!(args, &[OsString::from("-c")]);
            }
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn uuid7_via_argv0() {
        let arg0 = args(&[]);
        let sel = select(&OsStr::new("uuid7"), &arg0);
        match sel {
            Selection::Run { applet, .. } => {
                assert_eq!(applet.name, "uuid7")
            }
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn dispatcher_without_args_is_no_applet() {
        let arg0 = args(&[]);
        assert_eq!(
            select(&OsStr::new("derputils"), &arg0),
            Selection::NoApplet
        );
    }

    #[test]
    fn dispatcher_help_and_version() {
        let help = args(&["--help"]);
        assert_eq!(
            select(&OsStr::new("derputils"), &help),
            Selection::Help
        );
        let version = args(&["version"]);
        assert_eq!(
            select(&OsStr::new("derputils"), &version),
            Selection::Version
        );
    }

    #[test]
    fn dispatcher_subcommand() {
        let arg0 = args(&["quraa", "-s"]);
        let sel = select(&OsStr::new("derputils"), &arg0);
        match sel {
            Selection::Run { applet, args } => {
                assert_eq!(applet.name, "quraa");
                assert_eq!(args, &[OsString::from("-s")]);
            }
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn dispatcher_unknown_applet() {
        let arg0 = args(&["nope"]);
        assert_eq!(
            select(&OsStr::new("derputils"), &arg0),
            Selection::UnknownApplet {
                name: "nope".to_owned()
            }
        );
    }

    #[test]
    fn unknown_argv0() {
        let arg0 = args(&[]);
        assert_eq!(
            select(&OsStr::new("weird"), &arg0),
            Selection::UnknownApplet {
                name: "weird".to_owned()
            }
        );
    }
}
