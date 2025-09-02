mod checks;
mod clean;
mod cli;
mod commands;
mod completion;
mod generations;
mod installable;
mod json;
mod logging;
mod nixos;
mod update;
mod util;

use color_eyre::eyre::Context;
// TODO: get rid of eyre
use color_eyre::Result;
use color_eyre::Result as EyreResult;
use semver::Version;

// const MINIMUM_NIX_VERSION: Version = Version::new(2, 28, 4);
const MINIMUM_LIX_VERSION: Version = Version::new(2, 93, 3);

fn main() -> Result<()> {
    let args = <crate::cli::CliOpts as clap::Parser>::parse();

    // Set up logging
    crate::logging::setup_logging(args.verbosity)?;
    tracing::debug!("{args:#?}");

    startup_check().context("Failed to run startup checks")?;

    // Check Nix version upfront
    checks::verify_nix_environment()?;

    // Once we assert required Nix features, validate NH environment checks
    // For now, this is just NH_* variables being set. More checks may be
    // added to setup_environment in the future.
    checks::verify_variables()?;

    args.command.run()
}

fn startup_check() -> EyreResult<()> {
    todo!()
}
