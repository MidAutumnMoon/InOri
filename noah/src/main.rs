mod clean;
mod commands;
mod completion;
mod generations;
mod handy;
mod installable;
mod logging;
mod nixos;
mod update;

use color_eyre::eyre::Context;
use color_eyre::eyre::bail;
use color_eyre::eyre::ensure;
// TODO: get rid of eyre
use color_eyre::Result;
use color_eyre::Result as EyreResult;
use semver::Version;

use crate::handy::NixVariant;
use crate::handy::nix_info;

// const MINIMUM_NIX_VERSION: Version = Version::new(2, 28, 4);
const MINIMUM_LIX_VERSION: Version = Version::new(2, 93, 3);

use clap_verbosity_flag::InfoLevel;

#[derive(clap::Parser, Debug)]
/// A tailored nix helper.
pub struct CliOpts {
    #[command(flatten)]
    /// Increase logging verbosity, can be passed multiple times for
    /// more detailed logs.
    pub verbosity: clap_verbosity_flag::Verbosity<InfoLevel>,

    #[command(subcommand)]
    pub command: CliCmd,
}

#[derive(clap::Subcommand, Debug)]
#[command(disable_help_subcommand = true)]
pub enum CliCmd {
    NixOS(crate::nixos::OsArgs),
    // Deploy,
    Clean(crate::clean::CleanProxy),
    Completion(CompletionArgs),
}

impl CliCmd {
    pub fn run(self) -> Result<()> {
        match self {
            Self::NixOS(args) => {
                // TODO: get rid of envvar
                unsafe {
                    std::env::set_var("NH_CURRENT_COMMAND", "os");
                }
                args.run()
            }
            Self::Clean(proxy) => proxy.command.run(),
            Self::Completion(args) => args.run(),
        }
    }
}

#[derive(Debug, clap::Parser)]
/// Generate shell completions.
pub struct CompletionArgs {
    /// Name of the shell
    pub shell: clap_complete::Shell,
}

fn main() -> Result<()> {
    startup_check().context("Failed to run startup checks")?;

    let args = <crate::CliOpts as clap::Parser>::parse();

    // Set up logging
    crate::logging::setup_logging(args.verbosity)?;
    tracing::debug!("{args:#?}");

    args.command.run()
}

fn startup_check() -> EyreResult<()> {
    let (variant, version, features) =
        nix_info().context("Failed to fetch nix information")?;

    if matches!(variant, NixVariant::DetSys | NixVariant::Nix) {
        bail!(
            "Noah don't like stock nix or DetSys nix. It is currently lix only."
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
