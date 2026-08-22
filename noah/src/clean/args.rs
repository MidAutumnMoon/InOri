use std::path::PathBuf;

use clap::Args;
use clap::Subcommand;

// Needed a struct to have multiple sub-subcommands
#[derive(Debug, Clone, Args)]
pub struct CleanProxy {
    #[clap(subcommand)]
    pub command: CleanMode,
}

#[derive(Debug, Clone, Subcommand)]
/// Enhanced nix cleanup.
pub enum CleanMode {
    /// Clean all profiles.
    All(Clean),
    /// Clean the current user's profiles.
    User(Clean),
    /// Clean a specific profile.
    Profile(CleanProfile),
}

#[derive(Args, Clone, Debug)]
pub struct Clean {
    #[arg(long, short, default_value = "1")]
    /// At least keep this number of generations.
    pub keep: u32,

    #[arg(long, short = 'K', default_value = "0h")]
    /// At least keep gcroots and generations in this time range since now.
    ///
    /// See the documentation of humantime for possible formats: <https://docs.rs/humantime/latest/humantime/fn.parse_duration.html>.
    pub keep_since: humantime::Duration,

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

    /// Don't run nix store --gc.
    #[arg(long = "no-gc", alias = "nogc")]
    pub no_gc: bool,

    /// Don't clean gcroots.
    #[arg(long = "no-gcroots", alias = "nogcroots")]
    pub no_gcroots: bool,

    /// Don't clean direnv gcroots.
    #[arg(long = "no-direnv", alias = "nodirenv")]
    pub no_direnv: bool,

    /// Run nix-store --optimise after gc.
    #[arg(long)]
    pub optimise: bool,

    /// Pass --max to nix store gc.
    #[arg(long)]
    pub max: Option<String>,

    /// Keep at least one gcroot per direnv project.
    #[arg(long)]
    pub keep_one: bool,

    /// Cross filesystem boundaries when scanning gcroots.
    #[arg(long, short = 'x')]
    pub cross_filesystems: bool,
}

#[derive(Debug, Clone, Args)]
pub struct CleanProfile {
    #[command(flatten)]
    pub common: Clean,

    /// Which profile to clean.
    pub profile: PathBuf,
}
