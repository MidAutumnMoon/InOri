use std::path::PathBuf;

use clap::ValueEnum;
use clap::{Args, Parser, Subcommand};
use clap_verbosity_flag::InfoLevel;

use crate::Result;

#[derive(Parser, Debug)]
/// A tailored nix helper.
pub struct CliOpts {
    #[command(flatten)]
    /// Increase logging verbosity, can be passed multiple times for
    /// more detailed logs.
    pub verbosity: clap_verbosity_flag::Verbosity<InfoLevel>,

    #[command(subcommand)]
    pub command: CliCmd,
}

#[derive(Subcommand, Debug)]
#[command(disable_help_subcommand = true)]
pub enum CliCmd {
    NixOS(crate::nixos::OsArgs),
    // Deploy,
    Clean(CleanProxy),
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

#[derive(ValueEnum, Clone, Default, Debug)]
pub enum DiffType {
    /// Display package diff only if the of the
    /// current and the deployed configuration matches
    #[default]
    Auto,
    /// Always display package diff
    Always,
    /// Never display package diff
    Never,
}

// Needed a struct to have multiple sub-subcommands
#[derive(Debug, Clone, Args)]
pub struct CleanProxy {
    #[clap(subcommand)]
    command: CleanMode,
}

#[derive(Debug, Clone, Subcommand)]
/// Enhanced nix cleanup
pub enum CleanMode {
    /// Clean all profiles
    All(CleanArgs),
    /// Clean the current user's profiles
    User(CleanArgs),
    /// Clean a specific profile
    Profile(CleanProfileArgs),
}

#[derive(Args, Clone, Debug)]
#[clap(verbatim_doc_comment)]
/// Enhanced nix cleanup
///
/// For --keep-since, see the documentation of humantime for possible formats: <https://docs.rs/humantime/latest/humantime/fn.parse_duration.html>
pub struct CleanArgs {
    #[arg(long, short, default_value = "1")]
    /// At least keep this number of generations
    pub keep: u32,

    #[arg(long, short = 'K', default_value = "0h")]
    /// At least keep gcroots and generations in this time range since now.
    pub keep_since: humantime::Duration,

    /// Only print actions, without performing them
    #[arg(long, short = 'n')]
    pub dry: bool,

    /// Ask for confirmation
    #[arg(long, short)]
    pub ask: bool,

    /// Don't run nix store --gc
    #[arg(long = "no-gc", alias = "nogc")]
    pub no_gc: bool,

    /// Don't clean gcroots
    #[arg(long = "no-gcroots", alias = "nogcroots")]
    pub no_gcroots: bool,

    /// Run nix-store --optimise after gc
    #[arg(long)]
    pub optimise: bool,

    /// Pass --max to nix store gc
    #[arg(long)]
    pub max: Option<String>,
}

#[derive(Debug, Clone, Args)]
pub struct CleanProfileArgs {
    #[command(flatten)]
    pub common: CleanArgs,

    /// Which profile to clean
    pub profile: PathBuf,
}

#[derive(Debug, Parser)]
/// Generate shell completions.
pub struct CompletionArgs {
    /// Name of the shell
    pub shell: clap_complete::Shell,
}
