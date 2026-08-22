use std::path::PathBuf;

use crate::args::{DiffType, NixBuildPassthrough};
use crate::remote::Host;
use clap::Args;
use nh_installable::InstallableArgs;

use crate::nixos::generations::Field;

#[derive(Debug, Args)]
pub struct Rebuild {
    #[command(flatten)]
    pub common: CommonRebuild,

    #[command(flatten)]
    pub update_args: crate::update::Update,

    /// When using a flake installable, select this hostname from
    /// nixosConfigurations.
    ///
    /// When unspecified, defaults to the local hostname for local
    /// deployments, and hostname of the target machine for remote
    /// deployments (see --target-host).
    #[arg(long, short = 'H', global = true)]
    pub hostname: Option<String>,

    /// Explicitly select some specialisation.
    #[arg(long, short)]
    pub specialisation: Option<String>,

    /// Ignore specialisations.
    #[arg(long, short = 'S')]
    pub no_specialisation: bool,

    /// Install bootloader for switch and boot commands.
    #[arg(long)]
    pub install_bootloader: bool,

    /// Extra arguments passed to nix build.
    #[arg(last = true)]
    pub extra_args: Vec<String>,

    /// Don't panic if calling nh as root.
    #[arg(short = 'R', long, env = "NH_BYPASS_ROOT_CHECK")]
    pub bypass_root_check: bool,

    /// Deploy the built configuration to a different host over SSH.
    #[arg(long)]
    pub target_host: Option<Host>,

    /// Build the configuration on a different host over SSH.
    #[arg(long)]
    pub build_host: Option<Host>,

    /// Skip pre-activation system validation checks.
    #[arg(long, env = "NH_NO_VALIDATE")]
    pub no_validate: bool,
}

#[derive(Debug, Args)]
pub struct RebuildActivate {
    #[command(flatten)]
    pub rebuild: Rebuild,

    /// Show activation logs.
    #[arg(long, env = "NH_SHOW_ACTIVATION_LOGS", value_parser = clap::builder::BoolishValueParser::new())]
    pub show_activation_logs: bool,
}

#[derive(Debug, Args)]
pub struct Rollback {
    /// Only print actions, without performing them.
    #[arg(long, short = 'n')]
    pub dry: bool,

    /// Ask for confirmation.
    #[arg(
        long,
        short,
        env = "NH_ASK",
        value_parser = clap::builder::BoolishValueParser::new()
    )]
    pub ask: bool,

    /// Explicitly select some specialisation.
    #[arg(long, short)]
    pub specialisation: Option<String>,

    /// Ignore specialisations.
    #[arg(long, short = 'S')]
    pub no_specialisation: bool,

    /// Rollback to a specific generation number (defaults to previous
    /// generation).
    #[arg(long, short)]
    pub to: Option<u64>,

    /// Don't panic if calling nh as root.
    #[arg(short = 'R', long, env = "NH_BYPASS_ROOT_CHECK")]
    pub bypass_root_check: bool,

    /// Whether to display a package diff.
    #[arg(long, short, value_enum, default_value_t = DiffType::Auto)]
    pub diff: DiffType,
}

#[derive(Debug, Args)]
pub struct CommonRebuild {
    /// Only print actions, without performing them.
    #[arg(long, short = 'n')]
    pub dry: bool,

    /// Ask for confirmation.
    #[arg(
        long,
        short,
        env = "NH_ASK",
        value_parser = clap::builder::BoolishValueParser::new()
    )]
    pub ask: bool,

    #[command(flatten)]
    pub installable: InstallableArgs,

    /// Don't use nix-output-monitor for the build process.
    #[arg(long)]
    pub no_nom: bool,

    /// Path to save the result link, defaults to using a temporary directory.
    #[arg(long, short)]
    pub out_link: Option<PathBuf>,

    /// Whether to display a package diff.
    #[arg(long, short, value_enum, default_value_t = DiffType::Auto)]
    pub diff: DiffType,

    #[command(flatten)]
    pub passthrough: NixBuildPassthrough,
}

#[derive(Debug, Args)]
pub struct Repl {
    #[command(flatten)]
    pub installable: InstallableArgs,

    /// When using a flake installable, select this hostname from
    /// nixosConfigurations.
    #[arg(long, short = 'H', global = true)]
    pub hostname: Option<String>,
}

#[derive(Debug, Args)]
pub struct Generations {
    /// Path to Nix' profiles directory.
    #[arg(
        long,
        short = 'P',
        default_value = "/nix/var/nix/profiles/system"
    )]
    pub profile: Option<String>,

    /// Comma-delimited list of field(s) to display.
    #[arg(long, value_delimiter = ',')]
    pub fields: Option<Vec<Field>>,
}
