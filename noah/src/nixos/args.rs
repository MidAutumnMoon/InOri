use std::path::PathBuf;

use crate::args::{DiffType, NixBuildPassthroughArgs};
use crate::remote::RemoteHost;
use clap::Args;
use nh_installable::{FlakeConfig, InstallableArgs};

use crate::nixos::generations::Field;

#[derive(Debug, Args)]
pub struct BuildImageArgs {
    #[command(flatten)]
    pub common: RebuildArgs,

    /// Image variant
    #[arg(long)]
    pub image_variant: String,
}

#[derive(Debug, Args)]
pub struct RebuildVmArgs {
    #[command(flatten)]
    pub common: RebuildArgs,

    /// Build with bootloader. Bootloader is bypassed by default.
    #[arg(long, short = 'B')]
    pub with_bootloader: bool,

    /// Run the VM immediately after building
    #[arg(long, short = 'r')]
    pub run: bool,
}

#[derive(Debug, Args)]
pub struct RebuildArgs {
    #[command(flatten)]
    pub common: CommonRebuildArgs,

    #[command(flatten)]
    pub update_args: crate::update::UpdateArgs,

    /// When using a flake installable, select this hostname from
    /// nixosConfigurations
    ///
    /// When unspecified, defaults to the local hostname for local
    /// deployments, and hostname of the target machine for remote
    /// deployments (see --target-host).
    #[arg(long, short = 'H', global = true)]
    pub hostname: Option<String>,

    /// Explicitly select some specialisation
    #[arg(long, short)]
    pub specialisation: Option<String>,

    /// Ignore specialisations
    #[arg(long, short = 'S')]
    pub no_specialisation: bool,

    /// Install bootloader for switch and boot commands
    #[arg(long)]
    pub install_bootloader: bool,

    /// Extra arguments passed to nix build
    #[arg(last = true)]
    pub extra_args: Vec<String>,

    /// Don't panic if calling nh as root
    #[arg(short = 'R', long, env = "NH_BYPASS_ROOT_CHECK")]
    pub bypass_root_check: bool,

    /// Deploy the built configuration to a different host over SSH
    #[arg(long)]
    pub target_host: Option<RemoteHost>,

    /// Build the configuration on a different host over SSH
    #[arg(long)]
    pub build_host: Option<RemoteHost>,

    /// Skip pre-activation system validation checks
    #[arg(long, env = "NH_NO_VALIDATE")]
    pub no_validate: bool,
}

#[derive(Debug, Args)]
pub struct RebuildActivateArgs {
    #[command(flatten)]
    pub rebuild: RebuildArgs,

    /// Show activation logs
    #[arg(long, env = "NH_SHOW_ACTIVATION_LOGS", value_parser = clap::builder::BoolishValueParser::new())]
    pub show_activation_logs: bool,
}

impl RebuildArgs {
    #[must_use]
    pub fn uses_flakes(&self, config: &FlakeConfig) -> bool {
        self.common.installable.uses_flakes(config)
    }
}

#[derive(Debug, Args)]
pub struct RollbackArgs {
    /// Only print actions, without performing them
    #[arg(long, short = 'n')]
    pub dry: bool,

    /// Ask for confirmation
    #[arg(
        long,
        short,
        env = "NH_ASK",
        value_parser = clap::builder::BoolishValueParser::new()
    )]
    pub ask: bool,

    /// Explicitly select some specialisation
    #[arg(long, short)]
    pub specialisation: Option<String>,

    /// Ignore specialisations
    #[arg(long, short = 'S')]
    pub no_specialisation: bool,

    /// Rollback to a specific generation number (defaults to previous
    /// generation)
    #[arg(long, short)]
    pub to: Option<u64>,

    /// Don't panic if calling nh as root
    #[arg(short = 'R', long, env = "NH_BYPASS_ROOT_CHECK")]
    pub bypass_root_check: bool,

    /// Whether to display a package diff
    #[arg(long, short, value_enum, default_value_t = DiffType::Auto)]
    pub diff: DiffType,
}

#[derive(Debug, Args)]
pub struct CommonRebuildArgs {
    /// Only print actions, without performing them
    #[arg(long, short = 'n')]
    pub dry: bool,

    /// Ask for confirmation
    #[arg(
        long,
        short,
        env = "NH_ASK",
        value_parser = clap::builder::BoolishValueParser::new()
    )]
    pub ask: bool,

    #[command(flatten)]
    pub installable: InstallableArgs,

    /// Don't use nix-output-monitor for the build process
    #[arg(long)]
    pub no_nom: bool,

    /// Path to save the result link, defaults to using a temporary directory
    #[arg(long, short)]
    pub out_link: Option<PathBuf>,

    /// Whether to display a package diff
    #[arg(long, short, value_enum, default_value_t = DiffType::Auto)]
    pub diff: DiffType,

    #[command(flatten)]
    pub passthrough: NixBuildPassthroughArgs,
}

#[derive(Debug, Args)]
pub struct ReplArgs {
    #[command(flatten)]
    pub installable: InstallableArgs,

    /// When using a flake installable, select this hostname from
    /// nixosConfigurations
    #[arg(long, short = 'H', global = true)]
    pub hostname: Option<String>,
}

impl ReplArgs {
    #[must_use]
    pub fn uses_flakes(&self, config: &FlakeConfig) -> bool {
        self.installable.uses_flakes(config)
    }
}

#[derive(Debug, Args)]
pub struct GenerationsArgs {
    /// Path to Nix' profiles directory
    #[arg(
        long,
        short = 'P',
        default_value = "/nix/var/nix/profiles/system"
    )]
    pub profile: Option<String>,

    /// Comma-delimited list of field(s) to display
    #[arg(long, value_delimiter = ',')]
    pub fields: Option<Vec<Field>>,
}
