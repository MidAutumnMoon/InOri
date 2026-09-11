//! Multicall applet implementations, registry, and dispatch.
//!
//! Every applet owns its bpaf parser, so the same CLI serves both the
//! dispatcher (`{BIN_NAME} APPLET ...`) and the `argv[0]`-selected
//! invocation a multicall symlink performs. The dispatcher is built from the
//! applet registry below, which also makes bpaf render the dispatcher help
//! and report unknown applets.

mod completion;
mod hops;
mod qr;
mod upwards;
mod uuid7;

use std::borrow::Cow;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::path::Path;
use std::process::ExitCode;

use bpaf::Args;
use bpaf::OptionParser;
use bpaf::Parser;
use bpaf::choice;

use crate::BIN_NAME;

/// A parsed applet invocation: the applet's effect, with the arguments it
/// will run with already captured.
struct Invocation(Box<dyn FnOnce() -> rootcause::Result<ExitCode>>);

impl Invocation {
    /// Build one applet's CLI from its arguments and its effect.
    ///
    /// The result is both the applet's dispatcher command and the entry
    /// point of its `argv[0]`-selected invocation.
    #[must_use]
    fn cli<A>(
        args: impl Parser<A> + 'static,
        summary: &'static str,
        effect: fn(A) -> rootcause::Result<ExitCode>,
    ) -> OptionParser<Self>
    where
        A: 'static,
    {
        args.map(move |args| Self(Box::new(move || effect(args))))
            .to_options()
            .descr(summary)
    }

    /// Run the applet this invocation was parsed for.
    fn run(self) -> rootcause::Result<ExitCode> {
        (self.0)()
    }
}

/// One multicall applet.
#[derive(Debug, Clone, Copy)]
struct Applet {
    /// Selector matched against `argv[0]`; also the command word.
    name: &'static str,
    /// Type-erased boundary around the applet's parser and implementation.
    /// Its description doubles as the one-line summary in the applet listing.
    cli: fn() -> OptionParser<Invocation>,
}

impl Applet {
    const fn new(
        name: &'static str,
        cli: fn() -> OptionParser<Invocation>,
    ) -> Self {
        Self { name, cli }
    }

    const fn name(&self) -> &'static str {
        self.name
    }

    /// The applet as a command of the dispatcher CLI.
    fn command(&self) -> impl Parser<Invocation> + use<> {
        (self.cli)().command(self.name())
    }
}

/// The complete applet registry, grouped as `--help` lists them.
///
/// This is the single source for the applet set: the dispatcher CLI is built
/// from the groups, and `dispatch` and the `completion` applet iterate the
/// applets inside them (bpaf exposes no way to enumerate a parser's commands).
const APPLET_GROUPS: &[(&str, &[Applet])] = &[
    ("Generators", &[qr::APPLET, uuid7::APPLET]),
    ("Paths", &[upwards::APPLET, hops::APPLET]),
    ("Shell integration", &[completion::APPLET]),
];

/// Every applet, in registry order.
fn applets() -> impl Iterator<Item = &'static Applet> {
    APPLET_GROUPS.iter().flat_map(|(_, applets)| *applets)
}

fn find_applet(name: &str) -> Option<&'static Applet> {
    applets().find(|applet| applet.name() == name)
}

/// The dispatcher CLI: one command per applet, grouped for `--help`.
#[must_use]
fn dispatcher_cli() -> OptionParser<Invocation> {
    let groups = APPLET_GROUPS.iter().map(|(heading, applets)| {
        choice(applets.iter().map(|applet| applet.command().boxed()))
            .group_help(*heading)
            .boxed()
    });
    choice(groups)
        .to_options()
        .version(env!("CARGO_PKG_VERSION"))
        .fallback_to_usage()
}

/// Parse one invocation and run the applet it resolved to.
fn run(
    parser: &OptionParser<Invocation>,
    name: &str,
    args: &[OsString],
) -> ExitCode {
    match parser.run_inner(Args::from(args).set_name(name)) {
        Ok(invocation) => match invocation.run() {
            Ok(exit_code) => exit_code,
            Err(report) => {
                eprintln!("{report}");
                ExitCode::FAILURE
            }
        },
        Err(failure) => {
            // 100 is bpaf's own default max width.
            failure.print_message(100);
            ExitCode::from(u8::try_from(failure.exit_code()).unwrap_or(1))
        }
    }
}

/// Dispatcher arguments, with the `help` and `version` applet spellings
/// rewritten to the flags bpaf handles natively.
fn dispatcher_args(args: &[OsString]) -> Cow<'_, [OsString]> {
    let flag = match args.first().and_then(|arg| arg.to_str()) {
        Some("help") => "--help",
        Some("version") => "--version",
        _ => return Cow::Borrowed(args),
    };

    let rewritten = std::iter::once(OsString::from(flag))
        .chain(args.iter().skip(1).cloned())
        .collect();
    Cow::Owned(rewritten)
}

/// Resolve and execute one multicall process invocation.
#[must_use]
pub fn dispatch(invoked_as: &OsStr, args: &[OsString]) -> ExitCode {
    let name = Path::new(invoked_as)
        .file_name()
        .unwrap_or_else(|| OsStr::new(BIN_NAME));

    // Multicall name: `argv[0]` selects the applet.
    if let Some(applet) = name.to_str().and_then(find_applet) {
        let applet_cli = (applet.cli)();
        return run(&applet_cli, applet.name(), args);
    }

    // Dispatcher name: the first argument selects the applet.
    if name.to_str() == Some(BIN_NAME) {
        return run(&dispatcher_cli(), BIN_NAME, &dispatcher_args(args));
    }

    // Some other name: report it as an unknown applet, the same way its
    // `{BIN_NAME} NAME` spelling does.
    let mut with_name = Vec::with_capacity(args.len() + 1);
    with_name.push(name.to_owned());
    with_name.extend_from_slice(args);
    run(&dispatcher_cli(), BIN_NAME, &with_name)
}

#[cfg(test)]
#[expect(clippy::panic, reason = "in tests")]
mod test {
    use super::*;

    fn args(items: &[&str]) -> Vec<OsString> {
        items.iter().map(OsString::from).collect()
    }

    fn parse(items: &[&str]) -> Result<Invocation, bpaf::ParseFailure> {
        dispatcher_cli().run_inner(Args::from(items).set_name(BIN_NAME))
    }

    /// Help text rendered for a command line, requiring it to render some.
    fn help_text(items: &[&str]) -> String {
        match parse(items) {
            Err(bpaf::ParseFailure::Stdout(help, _)) => help.monochrome(true),
            Err(bpaf::ParseFailure::Stderr(error)) => panic!(
                "expected help for {items:?}, got error: {}",
                error.monochrome(true)
            ),
            Err(bpaf::ParseFailure::Completion(_)) => {
                panic!("expected help for {items:?}, got completion")
            }
            Ok(_) => panic!("expected help for {items:?}, got an invocation"),
        }
    }

    /// The registry drives the CLI, so this only has to catch rendering
    /// breakage: every applet and heading must reach `--help` and a bare
    /// invocation (which falls back to the same listing).
    #[test]
    fn bare_invocation_and_help_render_the_registry() {
        for items in [&[][..], &["--help"][..]] {
            let help = help_text(items);
            for applet in applets() {
                assert!(
                    help.contains(applet.name()),
                    "missing applet {:?} in:\n{help}",
                    applet.name()
                );
            }
            for (heading, _) in APPLET_GROUPS {
                assert!(help.contains(heading), "missing {heading:?} in:\n{help}");
            }
        }
    }

    /// Each registry applet must come out of the group builder as a dispatcher
    /// command: sharing the registry does not by itself wire one up.
    /// Parser identity is covered by `applet_commands_parse_applet_arguments`.
    #[test]
    fn every_applet_command_renders_help() {
        for applet in applets() {
            let name = applet.name();
            let help = help_text(&[name, "--help"]);
            assert!(
                help.contains(&format!("Usage: derputils {name}")),
                "{name} is missing from the dispatcher CLI:\n{help}"
            );
        }
    }

    /// `Applet::cli` is type-erased, so a command stays compiling when it is
    /// wired to another applet's parser or a placeholder; parse one real
    /// invocation per applet rather than trusting the help text.
    #[test]
    fn applet_commands_parse_applet_arguments() {
        for items in [
            &["qr", "-c"][..],
            &["uuid7"][..],
            &["upwards"][..],
            &["hops", "/bin/sh"][..],
            &["completion", "bash"][..],
        ] {
            assert!(parse(items).is_ok(), "{items:?} did not parse");
        }

        // `qr` insists on one source; a placeholder parser would accept this.
        assert!(matches!(
            parse(&["qr"]),
            Err(bpaf::ParseFailure::Stderr(_))
        ));
    }

    #[test]
    fn help_and_version_words_map_to_their_flags() {
        let spelled = args(&["help", "qr"]);
        assert_eq!(
            dispatcher_args(&spelled).as_ref(),
            &[OsString::from("--help"), OsString::from("qr")]
        );

        assert_eq!(
            dispatcher_args(&args(&["version"])).as_ref(),
            &[OsString::from("--version")]
        );

        // Everything else reaches the parser as spelled.
        let applet = args(&["qr", "-c"]);
        assert_eq!(dispatcher_args(&applet).as_ref(), applet.as_slice());
    }
}
