mod clean;
mod commands;
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
use tracing::debug;

use crate::handy::NixVariant;
use crate::handy::nix_info;

// const MINIMUM_NIX_VERSION: Version = Version::new(2, 28, 4);
const MINIMUM_LIX_VERSION: Version = Version::new(2, 93, 3);

use clap_verbosity_flag::InfoLevel;

#[derive(Debug)]
#[derive(clap::Parser)]
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
    Complete { shell: clap_complete::Shell },
}

fn main() -> Result<()> {
    let args = <CliOpts as clap::Parser>::parse();

    startup_check().context("Failed to run startup checks")?;

    // Set up logging
    crate::logging::setup_logging(args.verbosity)?;
    tracing::debug!("{args:#?}");

    match args.command {
        CliCmd::NixOS(args) => {
            // TODO: get rid of envvar
            unsafe {
                std::env::set_var("NH_CURRENT_COMMAND", "os");
            }
            args.run()
        }
        CliCmd::Clean(proxy) => proxy.command.run(),
        CliCmd::Complete { shell } => {
            use clap::CommandFactory;
            use clap_complete::generate;
            debug!("generate shell completion");
            let mut cmd = CliOpts::command();
            let mut out = std::io::stdout();
            generate(shell, &mut cmd, "nh", &mut out);
            Ok(())
        }
    }
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
