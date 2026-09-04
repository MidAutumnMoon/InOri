use std::ffi::OsStr;

use bpaf::Parser;
use bpaf::construct;
use bpaf::long;
use bpaf::parsers::ParseFlag;

use crate::clean::Request as CleanRequest;
use crate::clean::clean_cli;
use crate::command::ElevationStrategy;
use crate::nixos::{
    GenerationsRequest, RebuildCommand, ReplRequest, RollbackRequest,
    boot_cli, build_cli, generations_cli, repl_cli, rollback_cli,
    switch_cli, test_cli,
};
use crate::search::Request as SearchRequest;
use crate::search::search_cli;
use crate::weather::WeatherRequest;
use crate::weather::weather_cli;

/// Environment fallback accepting only literal `true` and `false`, matching
/// clap's default bool value parser.
#[must_use]
pub fn env_bool_strict(name: &'static str) -> impl Parser<bool> {
    bpaf::pure(()).parse(move |()| -> std::result::Result<bool, String> {
        match std::env::var_os(name) {
            None => Ok(false),
            Some(value) if value.as_os_str() == OsStr::new("true") => {
                Ok(true)
            }
            Some(value) if value.as_os_str() == OsStr::new("false") => {
                Ok(false)
            }
            Some(value) => Err(format!(
                "{name} is set to `{}`, which is not `true` or `false`",
                value.to_string_lossy()
            )),
        }
    })
}

struct CliAndEnvBool {
    cli: bool,
    from_env: bool,
}

/// Combine a CLI switch with a boolean environment fallback; the CLI switch
/// wins.
#[must_use]
pub fn switch_or_env(
    flag: ParseFlag<bool>,
    from_env: impl Parser<bool>,
) -> impl Parser<bool> {
    let cli = flag;
    construct!(CliAndEnvBool { cli, from_env })
        .map(|parsed| parsed.cli || parsed.from_env)
}

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
    Repl(ReplRequest),
    /// List available generations from profile path.
    Info(GenerationsRequest),
    /// Rollback to a previous generation.
    Rollback(RollbackRequest),
    /// Searches packages or NixOS options via search.nixos.org.
    Search(SearchRequest),
    /// Report which parts of a closure the substituters can supply.
    Weather(WeatherRequest),
    /// Enhanced nix cleanup.
    Clean(CleanRequest),
}

/// CLI parser for the global `--elevation-strategy` flag.
///
/// Declared once at the top level: bpaf named parsers consume their items
/// before a command narrows the argument scope, so the flag is accepted
/// before and after the subcommand word, like clap's `global = true`.
#[must_use]
fn elevation_cli() -> impl Parser<Option<ElevationStrategy>> {
    long("elevation-strategy")
        .short('e')
        .long("elevation-program")
        .env("NH_ELEVATION_STRATEGY")
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
    let repl = repl_cli()
        .to_options()
        .descr("Load system in a repl.")
        .command("repl")
        .map(CliCommand::Repl);
    let info = generations_cli()
        .to_options()
        .descr("List available generations from profile path.")
        .command("info")
        .map(CliCommand::Info);
    let rollback = rollback_cli()
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
    let weather = weather_cli()
        .to_options()
        .descr(
            "Report which parts of a closure the configured substituters \
             can supply.",
        )
        .command("weather")
        .map(CliCommand::Weather);

    let command = construct!([
        switch, boot, test, build, repl, info, rollback, search, clean,
        weather
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
    use crate::command::ElevationStrategy;

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
            Some(ElevationStrategy::None)
        ));
    }
}
