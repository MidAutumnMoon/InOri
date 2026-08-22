use anstyle::Style;
use clap::{Parser, Subcommand, builder::Styles};
use nh::command::ElevationStrategy;

use crate::Result;
use crate::RuntimeConfig;

const fn make_style() -> Styles {
    Styles::plain().header(Style::new().bold()).literal(
        Style::new().bold().fg_color(Some(anstyle::Color::Ansi(
            anstyle::AnsiColor::Yellow,
        ))),
    )
}

#[derive(Parser, Debug)]
#[command(
    version,
    about,
    long_about = None,
    styles=make_style(),
    propagate_version = false,
    help_template = "
{name} {version}
{about-with-newline}
{usage-heading} {usage}

{all-args}{after-help}
"
)]
/// Yet another nix helper
pub struct Main {
    #[arg(
        short,
        long,
        global = true,
        env = "NH_ELEVATION_STRATEGY",
        value_hint = clap::ValueHint::CommandName,
        alias = "elevation-program"
    )]
    /// Choose the privilege elevation strategy.
    ///
    /// Can be a path to an elevation program (e.g., /usr/bin/sudo),
    /// or one of: 'none' (no elevation),
    /// 'passwordless' (use elevation without password prompt for remote hosts
    /// with NOPASSWD configured), or 'auto' (automatically detect available
    /// elevation programs in order: doas, sudo, run0, pkexec)
    pub elevation_strategy: Option<nh::command::ElevationStrategyArg>,

    #[command(subcommand)]
    pub command: NHCommand,
}

#[derive(Subcommand, Debug)]
#[command(disable_help_subcommand = true)]
pub enum NHCommand {
    /// Build and activate the new configuration, and make it the boot default
    Switch(nh::nixos::args::RebuildActivateArgs),
    /// Build the new configuration and make it the boot default
    Boot(nh::nixos::args::RebuildActivateArgs),
    /// Build and activate the new configuration
    Test(nh::nixos::args::RebuildActivateArgs),
    /// Build the new configuration
    Build(nh::nixos::args::RebuildArgs),
    /// Load system in a repl
    Repl(nh::nixos::args::ReplArgs),
    /// List available generations from profile path
    Info(nh::nixos::args::GenerationsArgs),
    /// Rollback to a previous generation
    Rollback(nh::nixos::args::RollbackArgs),
    /// Searches packages or NixOS options via search.nixos.org
    Search(nh::search::args::SearchArgs),
    /// Enhanced nix cleanup
    Clean(nh::clean::args::CleanProxy),
}

impl NHCommand {
    /// Run the selected subcommand.
    pub fn run(
        self,
        env: &RuntimeConfig,
        elevation: ElevationStrategy,
    ) -> Result<()> {
        use nh::nixos::nixos::ActivationAction::{Boot, Switch, Test};
        match self {
            Self::Switch(args) => args.build_and_activate(
                Switch,
                elevation,
                &env.process,
                &env.sudo,
                &env.flake,
                &env.ssh,
            ),
            Self::Boot(args) => args.build_and_activate(
                Boot,
                elevation,
                &env.process,
                &env.sudo,
                &env.flake,
                &env.ssh,
            ),
            Self::Test(args) => args.build_and_activate(
                Test,
                elevation,
                &env.process,
                &env.sudo,
                &env.flake,
                &env.ssh,
            ),
            Self::Build(args) => {
                if args.common.ask || args.common.dry {
                    tracing::warn!(
                        "`--ask` and `--dry` have no effect for `nh build`"
                    );
                }
                args.build_only(&elevation, &env.flake, &env.ssh)
            }
            Self::Repl(args) => args.run(&env.process, &env.flake),
            Self::Info(args) => args.info(),
            Self::Rollback(args) => {
                args.rollback(elevation, &env.process, &env.sudo)
            }
            Self::Search(args) => args.run(),
            Self::Clean(proxy) => {
                proxy.command.run(elevation, &env.process, &env.sudo)
            }
        }
    }
}
