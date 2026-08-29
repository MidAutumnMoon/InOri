use bpaf::construct;
use bpaf::long;
use bpaf::Parser;
use nh::clean::args::CleanProxy;
use nh::clean::args::clean_cli;
use nh::command::ElevationStrategy;
use nh::command::ElevationStrategyArg;
use nh::nixos::args::Generations;
use nh::nixos::args::Rebuild;
use nh::nixos::args::RebuildActivate;
use nh::nixos::args::generations_cli;
use nh::nixos::args::rebuild_activate_cli;
use nh::nixos::args::rebuild_cli;
use nh::nixos::args::repl_cli;
use nh::nixos::args::rollback_cli;
use nh::nixos::args::Rollback;
use nh::nixos::args::Repl;
use nh::search::args::Search;
use nh::search::args::search_cli;

use crate::Result;
use crate::RuntimeConfig;

/// Yet another nix helper.
#[derive(Debug)]
pub struct Main {
    /// Choose the privilege elevation strategy.
    ///
    /// Can be a path to an elevation program (e.g., /usr/bin/sudo),
    /// or one of: 'none' (no elevation),
    /// 'passwordless' (use elevation without password prompt for remote hosts
    /// with NOPASSWD configured), or 'auto' (automatically detect available
    /// elevation programs in order: doas, sudo, run0, pkexec).
    pub elevation_strategy: Option<ElevationStrategyArg>,

    pub command: NHCommand,
}

#[derive(Debug)]
pub enum NHCommand {
    /// Build and activate the new configuration, and make it the boot default.
    Switch(RebuildActivate),
    /// Build the new configuration and make it the boot default.
    Boot(RebuildActivate),
    /// Build and activate the new configuration.
    Test(RebuildActivate),
    /// Build the new configuration.
    Build(Rebuild),
    /// Load system in a repl.
    Repl(Repl),
    /// List available generations from profile path.
    Info(Generations),
    /// Rollback to a previous generation.
    Rollback(Rollback),
    /// Searches packages or NixOS options via search.nixos.org.
    Search(Search),
    /// Enhanced nix cleanup.
    Clean(CleanProxy),
}

/// CLI parser for the global `--elevation-strategy` flag.
///
/// Declared once at the top level: bpaf named parsers consume their items
/// before a command narrows the argument scope, so the flag is accepted
/// before and after the subcommand word, like clap's `global = true`.
#[must_use]
fn elevation_cli() -> impl Parser<Option<ElevationStrategyArg>> {
    long("elevation-strategy")
        .short('e')
        .long("elevation-program")
        .env("NH_ELEVATION_STRATEGY")
        .argument::<ElevationStrategyArg>("STRATEGY")
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
pub fn cli() -> bpaf::OptionParser<Main> {
    let switch = rebuild_activate_cli()
        .to_options()
        .descr(
            "Build and activate the new configuration, and make it the boot \
             default.",
        )
        .command("switch")
        .map(NHCommand::Switch);
    let boot = rebuild_activate_cli()
        .to_options()
        .descr("Build the new configuration and make it the boot default.")
        .command("boot")
        .map(NHCommand::Boot);
    let test = rebuild_activate_cli()
        .to_options()
        .descr("Build and activate the new configuration.")
        .command("test")
        .map(NHCommand::Test);
    let build = rebuild_cli()
        .to_options()
        .descr("Build the new configuration.")
        .command("build")
        .map(NHCommand::Build);
    let repl = repl_cli()
        .to_options()
        .descr("Load system in a repl.")
        .command("repl")
        .map(NHCommand::Repl);
    let info = generations_cli()
        .to_options()
        .descr("List available generations from profile path.")
        .command("info")
        .map(NHCommand::Info);
    let rollback = rollback_cli()
        .to_options()
        .descr("Rollback to a previous generation.")
        .command("rollback")
        .map(NHCommand::Rollback);
    let search = search_cli()
        .to_options()
        .descr("Searches packages or NixOS options via search.nixos.org.")
        .command("search")
        .map(NHCommand::Search);
    let clean = clean_cli()
        .to_options()
        .descr("Enhanced nix cleanup.")
        .command("clean")
        .map(NHCommand::Clean);

    let command = construct!([
        switch, boot, test, build, repl, info, rollback, search, clean
    ]);

    let elevation_strategy = elevation_cli();
    construct!(Main {
        elevation_strategy,
        command,
    })
    .to_options()
    .descr("Yet another nix helper.")
    .version(env!("CARGO_PKG_VERSION"))
}

impl NHCommand {
    /// Run the selected subcommand.
    pub fn run(
        self,
        env: &RuntimeConfig,
        elevation: ElevationStrategy,
    ) -> Result<()> {
        use nh::nixos::ActivationAction::Boot;
        use nh::nixos::ActivationAction::Switch;
        use nh::nixos::ActivationAction::Test;

        match self {
            Self::Switch(args) => args.build_and_activate(
                Switch,
                elevation,
                &env.process,
                &env.sudo,
                &env.flake,
                &env.ssh,
            ),
            Self::Boot(args) => args.build_and_activate(
                Boot,
                elevation,
                &env.process,
                &env.sudo,
                &env.flake,
                &env.ssh,
            ),
            Self::Test(args) => args.build_and_activate(
                Test,
                elevation,
                &env.process,
                &env.sudo,
                &env.flake,
                &env.ssh,
            ),
            Self::Build(args) => {
                if args.common.ask || args.common.dry {
                    tracing::warn!(
                        "`--ask` and `--dry` have no effect for `nh build`"
                    );
                }
                args.build_only(&elevation, &env.flake, &env.ssh)
            }
            Self::Repl(args) => args.run(&env.process, &env.flake),
            Self::Info(args) => args.info(),
            Self::Rollback(args) => {
                args.rollback(elevation, &env.process, &env.sudo)
            }
            Self::Search(args) => args.run(),
            Self::Clean(proxy) => {
                proxy.command.run(elevation, &env.process, &env.sudo)
            }
        }
    }
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "Test assertions")]
mod tests {
    use bpaf::{Args, ParseFailure};

    use super::cli;
    use super::NHCommand;
    use nh::command::ElevationStrategyArg;

    fn parse(args: &[&str]) -> std::result::Result<super::Main, String> {
        let options = cli();
        options.check_invariants(false);
        options
            .run_inner(Args::from(args).set_name("nh"))
            .map_err(ParseFailure::unwrap_stderr)
    }

    #[test]
    fn elevation_strategy_accepted_before_subcommand() {
        let args = parse(&["--elevation-strategy", "none", "info"]).unwrap();

        assert!(matches!(
            args.elevation_strategy,
            Some(ElevationStrategyArg::None)
        ));
        assert!(matches!(args.command, NHCommand::Info(_)));
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
            Some(ElevationStrategyArg::Passwordless)
        ));
        assert!(matches!(args.command, NHCommand::Search(_)));
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
            Some(ElevationStrategyArg::Program(_))
        ));
        assert!(matches!(args.command, NHCommand::Clean(_)));
    }

    #[test]
    fn elevation_strategy_deprecated_alias_works() {
        let args = parse(&["--elevation-program", "auto", "info"]).unwrap();

        assert!(matches!(
            args.elevation_strategy,
            Some(ElevationStrategyArg::Auto)
        ));
    }

    #[test]
    fn elevation_strategy_env_fallback() {
        // SAFETY: single-threaded env manipulation with a name no other test
        // reads; the variable is removed again immediately after parsing.
        unsafe {
            std::env::set_var("NH_ELEVATION_STRATEGY", "none");
        }
        let args = parse(&["info"]).unwrap();
        // SAFETY: see above.
        unsafe {
            std::env::remove_var("NH_ELEVATION_STRATEGY");
        }

        assert!(matches!(
            args.elevation_strategy,
            Some(ElevationStrategyArg::None)
        ));
    }

    #[test]
    fn missing_subcommand_is_rejected() {
        let err = parse(&[]).unwrap_err();
        assert!(!err.is_empty());
    }
}
