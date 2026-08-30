mod clean;
mod cli;
mod command;
mod diff;
mod nix_command;
mod nix_options;
mod nixos;
mod runtime;
mod search;
mod target;
mod update;
mod weather;

use crate::cli::CliCommand;
use crate::runtime::Config;
use crate::runtime::Env;

const NH_VERSION: &str = env!("CARGO_PKG_VERSION");
const NH_REV: Option<&str> = option_env!("NH_REV");

/// Converts an error produced by an external crate into a
/// [`rootcause::Report`].
///
/// Dependencies that don't speak rootcause (e.g. `dix`) report their failures
/// through the boxed-error representation; this is the seam where such
/// errors enter rootcause.
pub(crate) fn external_report<E>(err: E) -> rootcause::Report
where
    Box<dyn std::error::Error + Send + Sync>: From<E>,
{
    rootcause::report!(Box::<dyn std::error::Error + Send + Sync>::from(
        err
    ))
    .into()
}

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
        CliCommand::Rebuild(command) => {
            nixos::run_rebuild(*command, config)
        }
        CliCommand::Repl(request) => nixos::run_repl(request, &config.env),
        CliCommand::Info(request) => nixos::run_info(&request),
        CliCommand::Rollback(request) => {
            nixos::run_rollback(request, config)
        }
        CliCommand::Search(request) => search::run(&request),
        CliCommand::Weather(request) => weather::run(&request, config),
        CliCommand::Clean(request) => clean::run(&request, config),
    }
}
