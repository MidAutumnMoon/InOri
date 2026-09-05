mod clean;
mod cli;
mod command;
mod elevation;
mod nix;
mod os;
mod runtime;
mod search;
mod target;

use crate::cli::CliCommand;
use crate::runtime::Config;
use crate::runtime::Env;

const NH_VERSION: &str = env!("CARGO_PKG_VERSION");
const NH_REV: Option<&str> = option_env!("NH_REV");

fn main() -> rootcause::Result<()> {
    let _log_guard = ino_tracing::init_tracing_subscriber();

    let args = cli::cli().run();

    tracing::debug!("{args:#?}");
    tracing::debug!(%NH_VERSION, ?NH_REV);

    // Capture environment-derived configuration once, after bpaf has handled
    // early exits such as --help and --version.
    let env = Env::capture()?;
    let config = Config::from_env(env, args.elevation_strategy)?;

    run(args.command, &config)
}

fn run(command: CliCommand, config: &Config) -> rootcause::Result<()> {
    match command {
        CliCommand::Rebuild(command) => os::rebuild::run(*command, config),
        CliCommand::Repl(opts) => os::repl::run(opts, &config.env),
        CliCommand::Info(opts) => os::info::run(&opts),
        CliCommand::Rollback(opts) => os::rollback::run(&opts, config),
        CliCommand::Search(opts) => search::run(&opts),
        CliCommand::Clean(opts) => clean::run(&opts, config),
    }
}
