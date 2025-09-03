mod clean;
mod commands;
mod deploy;
mod generations;
mod handy;
mod logging;
mod nixos;

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

    /// The flake to use. Can be anything that is considered as
    /// "installable" by Nix.
    #[arg(global = true)]
    #[arg(required = false)]
    #[arg(long, short = 'F')]
    #[arg(env = "NH_FLAKE")]
    pub flake: String,

    /// Allow noah to be executed as root.
    #[arg(global = true)]
    #[arg(required = false)]
    #[arg(long)]
    #[arg(env = "NH_NO_ROOT_CHECK")]
    #[arg(default_value_t = false)]
    pub no_root_check: bool,

    #[command(subcommand)]
    pub command: CliCmd,
}

#[derive(clap::Subcommand, Debug)]
#[command(disable_help_subcommand = true)]
pub enum CliCmd {
    // Flatten the subcommands so that they are not prefixed.
    #[command(flatten)]
    NixOS(Box<crate::nixos::OsSubcmd>),

    Deploy(Box<crate::deploy::Deploy>),

    #[command(subcommand)]
    Clean(Box<crate::clean::CleanMode>),

    /// Generate completions for shells.
    Complete {
        shell: clap_complete::Shell,
    },
}

#[derive(Debug)]
pub struct Runtime {
    flake: String,
    no_root_check: bool,
}

fn main() -> Result<()> {
    let cliopts = <CliOpts as clap::Parser>::parse();

    startup_check().context("Failed to run startup checks")?;

    // Set up logging
    crate::logging::setup_logging(cliopts.verbosity)?;
    tracing::debug!("{cliopts:#?}");

    let runtime = Runtime {
        flake: cliopts.flake,
        no_root_check: cliopts.no_root_check,
    };

    match cliopts.command {
        CliCmd::NixOS(cmd) => {
            // TODO: get rid of envvar
            unsafe {
                std::env::set_var("NH_CURRENT_COMMAND", "os");
            }
            cmd.run(runtime)
        }
        CliCmd::Deploy(..) => todo!(),
        CliCmd::Clean(clean) => clean.run(),
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
        "Experimental feature flakes or nix-command not enabled. Noah is built to be flake-only."
    };

    // lol lix
    ensure! {
        features.contains(&"pipe-operator".to_string())
        || features.contains(&"pipe-operators".to_string()),
        "Experimental feature pipe-operator (or pipe-operators if stock nix) not enabled"
    }

    Ok(())
}
