use clap_verbosity_flag::InfoLevel;

use crate::Result;

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
