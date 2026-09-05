//! Multicall applet implementations, registry, and dispatch.

mod completion;
mod hops;
mod qr;
mod upwards;
mod uuid7;

use std::ffi::OsStr;
use std::ffi::OsString;
use std::path::Path;
use std::process::ExitCode;

use crate::BIN_NAME;

/// How an applet run can fail; the dispatcher renders both.
#[derive(Debug)]
enum RunFailure {
    /// CLI parse exit, including `--help`/`--version`; bpaf renders it.
    Cli(bpaf::ParseFailure),
    /// Runtime failure; rendered as a report.
    Applet(rootcause::Report),
}

type AppletRunner = fn(args: &[OsString]) -> Result<ExitCode, RunFailure>;

/// One multicall applet.
#[derive(Debug, Clone, Copy)]
struct Applet {
    /// Selector matched against `argv[0]` and `{BIN_NAME} NAME`.
    name: &'static str,
    /// One-line summary shown in the applet listing and its help.
    summary: &'static str,
    /// Type-erased boundary around the applet's parser and implementation.
    runner: AppletRunner,
}

impl Applet {
    const fn new(
        name: &'static str,
        summary: &'static str,
        runner: AppletRunner,
    ) -> Self {
        Self {
            name,
            summary,
            runner,
        }
    }

    const fn name(&self) -> &'static str {
        self.name
    }

    const fn summary(&self) -> &'static str {
        self.summary
    }

    fn invoke(&self, args: &[OsString]) -> Result<ExitCode, RunFailure> {
        (self.runner)(args)
    }
}

/// A titled applet group in dispatcher help.
#[derive(Debug)]
struct AppletCategory {
    heading: &'static str,
    applets: &'static [Applet],
}

/// The complete applet registry, grouped in help order.
const APPLET_WITH_CATEGORIES: &[AppletCategory] = &[
    AppletCategory {
        heading: "Generators",
        applets: &[qr::APPLET, uuid7::APPLET],
    },
    AppletCategory {
        heading: "Paths",
        applets: &[upwards::APPLET, hops::APPLET],
    },
    AppletCategory {
        heading: "Shell integration",
        applets: &[completion::APPLET],
    },
];

fn all_applets() -> impl Iterator<Item = &'static Applet> {
    APPLET_WITH_CATEGORIES
        .iter()
        .flat_map(|category| category.applets.iter())
}

fn find_applet(name: &str) -> Option<&'static Applet> {
    all_applets().find(|applet| applet.name() == name)
}

/// Usage text listing all applets.
#[must_use]
fn usage() -> String {
    use std::fmt::Write as _;

    let width = all_applets()
        .map(|applet| applet.name.len())
        .max()
        .unwrap_or(0);
    let mut out =
        format!("Usage: {BIN_NAME} APPLET [OPTIONS]...\n\nApplets:\n");

    for (index, category) in APPLET_WITH_CATEGORIES.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        _ = writeln!(out, "  {}:", category.heading);
        for applet in category.applets {
            _ = writeln!(
                out,
                "    {:<width$}  {}",
                applet.name(),
                applet.summary()
            );
        }
    }

    out
}

/// A successfully resolved process invocation.
#[derive(Debug)]
enum ResolvedInvocation<'args> {
    /// Invoke one applet with its own argument slice.
    Applet {
        applet: &'static Applet,
        args: &'args [OsString],
    },
    /// Print dispatcher help.
    Help,
    /// Print the dispatcher version.
    Version,
}

/// Why an invocation could not be resolved.
#[derive(Debug)]
enum ResolutionError {
    /// The dispatcher was invoked without an applet.
    MissingApplet,
    /// The requested applet is not registered.
    UnknownApplet(String),
}

fn resolve_applet<'args>(
    name: &str,
    args: &'args [OsString],
) -> Result<ResolvedInvocation<'args>, ResolutionError> {
    let applet = find_applet(name)
        .ok_or_else(|| ResolutionError::UnknownApplet(name.to_owned()))?;
    Ok(ResolvedInvocation::Applet { applet, args })
}

/// Resolve the dispatcher mode and applet without performing any effects.
fn resolve<'args>(
    invoked_as: &OsStr,
    args: &'args [OsString],
) -> Result<ResolvedInvocation<'args>, ResolutionError> {
    let basename = Path::new(invoked_as)
        .file_name()
        .unwrap_or_else(|| OsStr::new(BIN_NAME))
        .to_string_lossy();

    if basename != BIN_NAME {
        // Multicall name: `argv[0]` selects the applet.
        return resolve_applet(&basename, args);
    }

    // Dispatcher name: the first argument selects the applet.
    let Some((first, rest)) = args.split_first() else {
        return Err(ResolutionError::MissingApplet);
    };
    match first.to_str() {
        Some("-h" | "--help" | "help") => Ok(ResolvedInvocation::Help),
        Some("-V" | "--version" | "version") => {
            Ok(ResolvedInvocation::Version)
        }
        Some(name) => resolve_applet(name, rest),
        None => Err(ResolutionError::UnknownApplet(
            first.to_string_lossy().into_owned(),
        )),
    }
}

fn report_resolution_error(error: ResolutionError) -> ExitCode {
    match error {
        ResolutionError::MissingApplet => {}
        ResolutionError::UnknownApplet(name) => {
            eprintln!("{BIN_NAME}: unknown applet '{name}'");
        }
    }
    eprint!("{}", usage());
    ExitCode::FAILURE
}

/// Resolve and execute one multicall process invocation.
#[must_use]
pub fn dispatch(invoked_as: &OsStr, args: &[OsString]) -> ExitCode {
    match resolve(invoked_as, args) {
        Ok(ResolvedInvocation::Applet { applet, args }) => {
            match applet.invoke(args) {
                Ok(exit_code) => exit_code,
                Err(RunFailure::Cli(failure)) => {
                    // 100 is bpaf's own default max width.
                    failure.print_message(100);
                    ExitCode::from(
                        u8::try_from(failure.exit_code()).unwrap_or(1),
                    )
                }
                Err(RunFailure::Applet(report)) => {
                    eprintln!("{report}");
                    ExitCode::FAILURE
                }
            }
        }
        Ok(ResolvedInvocation::Help) => {
            print!("{}", usage());
            ExitCode::SUCCESS
        }
        Ok(ResolvedInvocation::Version) => {
            let version = env!("CARGO_PKG_VERSION");
            println!("{BIN_NAME} {version}");
            ExitCode::SUCCESS
        }
        Err(error) => report_resolution_error(error),
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

    fn resolved_applet<'args>(
        invoked_as: &OsStr,
        args: &'args [OsString],
    ) -> (&'static Applet, &'args [OsString]) {
        match resolve(invoked_as, args) {
            Ok(ResolvedInvocation::Applet { applet, args }) => {
                (applet, args)
            }
            other => panic!("expected applet invocation, got {other:?}"),
        }
    }

    #[test]
    fn applet_via_argv0() {
        let argv = args(&["-c"]);
        let (applet, applet_args) =
            resolved_applet(OsStr::new("/usr/bin/qr"), &argv);
        assert_eq!(applet.name(), "qr");
        assert_eq!(applet_args, &[OsString::from("-c")]);
    }

    #[test]
    fn dispatcher_subcommand() {
        let argv = args(&["uuid7"]);
        let (applet, applet_args) =
            resolved_applet(OsStr::new(BIN_NAME), &argv);
        assert_eq!(applet.name(), "uuid7");
        assert!(applet_args.is_empty());
    }

    #[test]
    fn dispatcher_without_args_is_missing_applet() {
        assert_matches!(
            resolve(OsStr::new(BIN_NAME), &[]),
            Err(ResolutionError::MissingApplet)
        );
    }

    #[test]
    fn dispatcher_help_and_version() {
        let help = args(&["--help"]);
        assert_matches!(
            resolve(OsStr::new(BIN_NAME), &help),
            Ok(ResolvedInvocation::Help)
        );
        let version = args(&["version"]);
        assert_matches!(
            resolve(OsStr::new(BIN_NAME), &version),
            Ok(ResolvedInvocation::Version)
        );
    }

    #[test]
    fn dispatcher_unknown_applet() {
        let argv = args(&["nope"]);
        assert_matches!(
            resolve(OsStr::new(BIN_NAME), &argv),
            Err(ResolutionError::UnknownApplet(name)) if name == "nope"
        );
    }
}
