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
use color_eyre::eyre::bail;
use color_eyre::eyre::ensure;
// TODO: get rid of eyre
use color_eyre::Result;
use color_eyre::Result as EyreResult;
use semver::Version;

use crate::util::NixVariant;

// const MINIMUM_NIX_VERSION: Version = Version::new(2, 28, 4);
const MINIMUM_LIX_VERSION: Version = Version::new(2, 93, 3);

fn main() -> Result<()> {
    startup_check().context("Failed to run startup checks")?;

    let args = <crate::cli::CliOpts as clap::Parser>::parse();

    // Set up logging
    crate::logging::setup_logging(args.verbosity)?;
    tracing::debug!("{args:#?}");

    // Once we assert required Nix features, validate NH environment checks
    // For now, this is just NH_* variables being set. More checks may be
    // added to setup_environment in the future.
    checks::verify_variables()?;

    args.command.run()
}

fn startup_check() -> EyreResult<()> {
    let (variant, version, features) =
        util::nix_info().context("Failed to fetch nix information")?;

    if matches!(variant, NixVariant::DetSys | NixVariant::Nix) {
        bail!(
            "Noah don't like stock nix or DetSys nix. This is currently lix only."
        );
    }

    if version < MINIMUM_LIX_VERSION {
        bail!(
            r#"Nix version "{}" is below minimum supported version "{}""#,
            version,
            MINIMUM_LIX_VERSION
        )
    }

    ensure! {
        features.contains(&"flakes".to_string())
        && features.contains(&"nix-command".to_string()),
        "Experimental feature flakes or nix-command not enabled"
    };

    // lol lix
    ensure! {
        features.contains(&"pipe-operator".to_string())
        || features.contains(&"pipe-operators".to_string()),
        "Experimental feature pipe-operator (or pipe-operators if stock nix) not enabled"
    }

    Ok(())
}
