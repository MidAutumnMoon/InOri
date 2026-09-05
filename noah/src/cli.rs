use bpaf::Parser;
use bpaf::construct;
use bpaf::long;

use crate::clean;
use crate::clean::clean_cli;
use crate::elevation::ElevationStrategy;
use crate::os::cli::{boot_cli, build_cli, switch_cli, test_cli};
use crate::os::info;
use crate::os::rebuild::RebuildCommand;
use crate::os::repl;
use crate::os::rollback;
use crate::search;
use crate::search::search_cli;

/// Yet another nix helper.
#[derive(Debug)]
pub struct Cli {
    /// Choose the privilege elevation strategy.
    ///
    /// Can be a path to an elevation program (e.g., /usr/bin/sudo),
    /// or one of: 'none' (no elevation),
    /// 'passwordless' (use elevation without password prompt for remote hosts
    /// with NOPASSWD configured), or 'auto' (automatically detect available
    /// elevation programs in order: doas, sudo, run0, pkexec).
    pub elevation_strategy: Option<ElevationStrategy>,

    pub command: CliCommand,
}

#[derive(Debug)]
pub enum CliCommand {
    /// Build or activate a NixOS configuration.
    Rebuild(Box<RebuildCommand>),
    /// Load system in a repl.
    Repl(repl::CliOpts),
    /// List available generations from profile path.
    Info(info::CliOpts),
    /// Rollback to a previous generation.
    Rollback(rollback::CliOpts),
    /// Searches packages or NixOS options via search.nixos.org.
    Search(search::CliOpts),
    /// Enhanced nix cleanup.
    Clean(clean::CliOpts),
}

/// CLI parser for the global `--elevation-strategy` flag.
///
/// Declared once at the top level: bpaf named parsers consume their items
/// before a command narrows the argument scope, so the flag is accepted
/// before and after the subcommand word.
#[must_use]
fn elevation_cli() -> impl Parser<Option<ElevationStrategy>> {
    long("elevation-strategy")
        .short('e')
        .long("elevation-program")
        .argument::<ElevationStrategy>("STRATEGY")
        .help(
            "Choose the privilege elevation strategy.\n\nCan be a path to an \
             elevation program (e.g., /usr/bin/sudo), or one of: 'none' (no \
             elevation), 'passwordless' (use elevation without password \
             prompt for remote hosts with NOPASSWD configured), or 'auto' \
             (automatically detect available elevation programs in order: \
             doas, sudo, run0, pkexec)",
        )
        .optional()
}

/// Assemble the full `nh` command line parser.
#[must_use]
pub fn cli() -> bpaf::OptionParser<Cli> {
    let switch = switch_cli()
        .to_options()
        .descr(
            "Build and activate the new configuration, and make it the boot \
             default.",
        )
        .command("switch")
        .map(|command| CliCommand::Rebuild(Box::new(command)));
    let boot = boot_cli()
        .to_options()
        .descr("Build the new configuration and make it the boot default.")
        .command("boot")
        .map(|command| CliCommand::Rebuild(Box::new(command)));
    let test = test_cli()
        .to_options()
        .descr("Build and activate the new configuration.")
        .command("test")
        .map(|command| CliCommand::Rebuild(Box::new(command)));
    let build = build_cli()
        .to_options()
        .descr("Build the new configuration.")
        .command("build")
        .map(|command| CliCommand::Rebuild(Box::new(command)));
    let repl = repl::cli()
        .to_options()
        .descr("Load system in a repl.")
        .command("repl")
        .map(CliCommand::Repl);
    let info = info::cli()
        .to_options()
        .descr("List available generations from profile path.")
        .command("info")
        .map(CliCommand::Info);
    let rollback = rollback::cli()
        .to_options()
        .descr("Rollback to a previous generation.")
        .command("rollback")
        .map(CliCommand::Rollback);
    let search = search_cli()
        .to_options()
        .descr("Searches packages or NixOS options via search.nixos.org.")
        .command("search")
        .map(CliCommand::Search);
    let clean = clean_cli()
        .to_options()
        .descr("Enhanced nix cleanup.")
        .command("clean")
        .map(CliCommand::Clean);

    let command = construct!([
        switch, boot, test, build, repl, info, rollback, search, clean
    ]);

    let elevation_strategy = elevation_cli();
    construct!(Cli {
        elevation_strategy,
        command,
    })
    .to_options()
    .descr("Yet another nix helper.")
    .version(env!("CARGO_PKG_VERSION"))
    .fallback_to_usage()
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "Test assertions")]
mod tests {
    use bpaf::{Args, ParseFailure};

    use super::CliCommand;
    use super::cli;
    use crate::elevation::ElevationStrategy;

    fn parse(args: &[&str]) -> std::result::Result<super::Cli, String> {
        let options = cli();
        options.check_invariants(false);
        options
            .run_inner(Args::from(args).set_name("nh"))
            .map_err(ParseFailure::unwrap_stderr)
    }

    #[test]
    fn elevation_strategy_accepted_before_subcommand() {
        let args =
            parse(&["--elevation-strategy", "none", "info"]).unwrap();

        assert!(matches!(
            args.elevation_strategy,
            Some(ElevationStrategy::None)
        ));
        assert!(matches!(args.command, CliCommand::Info(_)));
    }

    #[test]
    fn elevation_strategy_accepted_after_subcommand() {
        let args = parse(&[
            "search",
            "hello",
            "--elevation-strategy",
            "passwordless",
        ])
        .unwrap();

        assert!(matches!(
            args.elevation_strategy,
            Some(ElevationStrategy::Passwordless)
        ));
        assert!(matches!(args.command, CliCommand::Search(_)));
    }

    #[test]
    fn elevation_strategy_accepted_after_nested_subcommand() {
        let args = parse(&[
            "clean",
            "all",
            "--elevation-strategy",
            "program:/usr/bin/doas",
        ])
        .unwrap();

        assert!(matches!(
            args.elevation_strategy,
            Some(ElevationStrategy::Program(_))
        ));
        assert!(matches!(args.command, CliCommand::Clean(_)));
    }

    #[test]
    fn elevation_strategy_deprecated_alias_works() {
        let args =
            parse(&["--elevation-program", "auto", "info"]).unwrap();

        assert!(matches!(
            args.elevation_strategy,
            Some(ElevationStrategy::Auto)
        ));
    }
}
