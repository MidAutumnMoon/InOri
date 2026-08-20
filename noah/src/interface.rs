use anstyle::Style;
use clap::{Parser, Subcommand, builder::Styles};
use nh::command::ElevationStrategy;

use crate::RuntimeEnv;
use crate::Result;

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
    Switch(nh::nixos::args::OsRebuildActivateArgs),
    /// Build the new configuration and make it the boot default
    Boot(nh::nixos::args::OsRebuildActivateArgs),
    /// Build and activate the new configuration
    Test(nh::nixos::args::OsRebuildActivateArgs),
    /// Build the new configuration
    Build(nh::nixos::args::RebuildArgs),
    /// Load system in a repl
    Repl(nh::nixos::args::OsReplArgs),
    /// List available generations from profile path
    Info(nh::nixos::args::OsGenerationsArgs),
    /// Rollback to a previous generation
    Rollback(nh::nixos::args::OsRollbackArgs),
    /// Build a `NixOS` VM image
    BuildVm(nh::nixos::args::RebuildVmArgs),
    /// Build a `NixOS` disk-image variant
    BuildImage(nh::nixos::args::OsBuildImageArgs),
    /// Searches packages or NixOS/home-manager options via search.nixos.org
    Search(nh::search::args::SearchArgs),
    /// Enhanced nix cleanup
    Clean(nh::clean::args::CleanProxy),
}

impl NHCommand {
    /// Run the selected subcommand.
    ///
    /// # Errors
    ///
    /// Returns an error if required Nix features are unavailable or if the
    pub fn run(
        self,
        env: &RuntimeEnv,
        elevation: ElevationStrategy,
    ) -> Result<()> {
        use nh::nixos::nixos::ActivationAction::{Boot, Switch, Test};

        match self {
            Self::Switch(args) => args.build_and_activate(
                Switch,
                elevation,
                &env.subprocess,
                &env.sudo,
                &env.flake,
                &env.ssh,
            ),
            Self::Boot(args) => args.build_and_activate(
                Boot,
                elevation,
                &env.subprocess,
                &env.sudo,
                &env.flake,
                &env.ssh,
            ),
            Self::Test(args) => args.build_and_activate(
                Test,
                elevation,
                &env.subprocess,
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
                args.build_only(
                    &elevation,
                    &env.subprocess,
                    &env.sudo,
                    &env.flake,
                    &env.ssh,
                )
            }
            Self::Repl(args) => args.run(
                &env.subprocess,
                &env.sudo,
                &env.flake,
            ),
            Self::Info(args) => {
                args.info(&env.subprocess, &env.sudo)
            }
            Self::Rollback(args) => args.rollback(
                elevation,
                &env.subprocess,
                &env.sudo,
                &env.flake,
            ),
            Self::BuildVm(args) => args.build_vm(
                &elevation,
                &env.subprocess,
                &env.sudo,
                &env.flake,
                &env.ssh,
            ),
            Self::BuildImage(args) => args.build_image(
                &elevation,
                &env.subprocess,
                &env.sudo,
                &env.flake,
                &env.ssh,
            ),
            Self::Search(args) => args.run(&env.github),
            Self::Clean(proxy) => proxy.command.run(
                elevation,
                &env.subprocess,
                &env.sudo,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    // TODO: Rewrite this test. The old test used serial_test + env::set_var to
    // test clap's `#[arg(env = "NH_ASK")]` boolish parsing. clap reads env
    // vars at parse time, so this is inherently env-var-based. To test without
    // serial_test, use `Main::try_parse_from` with explicit `--ask`/`--no-ask`
    // flags instead of relying on the NH_ASK env var.
    /*
    use std::{env, ffi::OsString};

    use clap::{Parser, error::ErrorKind};
    use nh::clean::args::CleanMode;

    use super::{Main, NHCommand};

    struct EnvGuard(Option<OsString>);

    impl EnvGuard {
        fn new() -> Self {
            Self(env::var_os("NH_ASK"))
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.0 {
                    Some(value) => env::set_var("NH_ASK", value),
                    None => env::remove_var("NH_ASK"),
                }
            }
        }
    }

    #[test]
    fn nh_ask_parses_boolish_environment_values() -> clap::error::Result<()>
    {
        let _guard = EnvGuard::new();

        for (value, expected) in
            [("1", true), ("true", true), ("0", false), ("false", false)]
        {
            unsafe {
                env::set_var("NH_ASK", value);
            }
            let parsed = Main::try_parse_from(["nh", "clean", "all"])?;
            let ask = match parsed.command {
                NHCommand::Clean(proxy) => match proxy.command {
                    CleanMode::All(args) => Some(args.ask),
                    _ => None,
                },
                _ => None,
            };
            assert_eq!(ask, Some(expected));
        }

        unsafe {
            env::set_var("NH_ASK", "invalid");
        }
        assert!(matches!(
          Main::try_parse_from(["nh", "clean", "all"]),
          Err(error) if error.kind() == ErrorKind::ValueValidation
        ));

        Ok(())
    }
    */
}
