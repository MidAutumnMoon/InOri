mod checks;
mod clean;
mod commands;
mod completion;
mod generations;
mod installable;
mod cli;
mod json;
mod logging;
mod nixos;
mod update;
mod util;

// TODO: get rid of eyre
use color_eyre::Result;

fn main() -> Result<()> {
    let args = <crate::cli::CliOpts as clap::Parser>::parse();

    // Set up logging
    crate::logging::setup_logging(args.verbosity)?;
    tracing::debug!("{args:#?}");

    // Check Nix version upfront
    checks::verify_nix_environment()?;

    // Once we assert required Nix features, validate NH environment checks
    // For now, this is just NH_* variables being set. More checks may be
    // added to setup_environment in the future.
    checks::verify_variables()?;

    args.command.run()
}
