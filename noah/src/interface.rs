use anstyle::Style;
use clap::{Parser, Subcommand, builder::Styles};
use clap_verbosity_flag::InfoLevel;
use nh_core::command::ElevationStrategy;

use crate::checks::{
    FeatureRequirements, FlakeFeatures, LegacyFeatures, NoFeatures,
    OsReplFeatures,
};

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
    #[command(flatten)]
    /// Increase logging verbosity, can be passed multiple times for
    /// more detailed logs.
    pub verbosity: clap_verbosity_flag::Verbosity<InfoLevel>,

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
    pub elevation_strategy: Option<nh_core::command::ElevationStrategyArg>,

    #[command(subcommand)]
    pub command: NHCommand,
}

#[derive(Subcommand, Debug)]
#[command(disable_help_subcommand = true)]
pub enum NHCommand {
    /// Build and activate the new configuration, and make it the boot default
    Switch(nh_nixos::args::OsRebuildActivateArgs),
    /// Build the new configuration and make it the boot default
    Boot(nh_nixos::args::OsRebuildActivateArgs),
    /// Build and activate the new configuration
    Test(nh_nixos::args::OsRebuildActivateArgs),
    /// Build the new configuration
    Build(nh_nixos::args::RebuildArgs),
    /// Load system in a repl
    Repl(nh_nixos::args::OsReplArgs),
    /// List available generations from profile path
    Info(nh_nixos::args::OsGenerationsArgs),
    /// Rollback to a previous generation
    Rollback(nh_nixos::args::OsRollbackArgs),
    /// Build a `NixOS` VM image
    BuildVm(nh_nixos::args::RebuildVmArgs),
    /// Build a `NixOS` disk-image variant
    BuildImage(nh_nixos::args::OsBuildImageArgs),
    /// Searches packages or NixOS/home-manager options via search.nixos.org,
    /// or a local SPAM database
    Search(nh_search::args::SearchArgs),
    /// Enhanced nix cleanup
    Clean(nh_clean::args::CleanProxy),
}

impl NHCommand {
    #[must_use]
    pub fn get_feature_requirements(
        &self,
    ) -> Box<dyn FeatureRequirements> {
        match self {
            Self::Repl(args) => {
                let is_flake = args.uses_flakes();
                Box::new(OsReplFeatures { is_flake })
            }
            Self::Switch(args) | Self::Boot(args) | Self::Test(args) => {
                if args.rebuild.uses_flakes() {
                    Box::new(FlakeFeatures)
                } else {
                    Box::new(LegacyFeatures)
                }
            }
            Self::Build(args) => {
                if args.uses_flakes() {
                    Box::new(FlakeFeatures)
                } else {
                    Box::new(LegacyFeatures)
                }
            }
            Self::BuildVm(args) => {
                if args.common.uses_flakes() {
                    Box::new(FlakeFeatures)
                } else {
                    Box::new(LegacyFeatures)
                }
            }
            Self::Info(_) | Self::Rollback(_) => Box::new(LegacyFeatures),
            Self::BuildImage(args) => {
                if args.common.uses_flakes() {
                    Box::new(FlakeFeatures)
                } else {
                    Box::new(LegacyFeatures)
                }
            }
            Self::Search(..) | Self::Clean(..) => Box::new(NoFeatures),
        }
    }

    /// Run the selected subcommand.
    ///
    /// # Errors
    ///
    /// Returns an error if required Nix features are unavailable or if the
    /// selected subcommand fails.
    pub fn run(self, elevation: ElevationStrategy) -> Result<()> {
        use nh_nixos::nixos::OsRebuildVariant::{
            Boot, Build, Switch, Test,
        };

        // Check features specific to this command
        let requirements = self.get_feature_requirements();
        requirements.check_features()?;

        match self {
            Self::Switch(args) => {
                args.rebuild_and_activate(&Switch, None, elevation)
            }
            Self::Boot(args) => {
                args.rebuild_and_activate(&Boot, None, elevation)
            }
            Self::Test(args) => {
                args.rebuild_and_activate(&Test, None, elevation)
            }
            Self::Build(args) => {
                if args.common.ask || args.common.dry {
                    tracing::warn!(
                        "`--ask` and `--dry` have no effect for `nh build`"
                    );
                }
                args.build_only(&Build, None, &elevation)
            }
            Self::Repl(args) => args.run(),
            Self::Info(args) => args.info(),
            Self::Rollback(args) => args.rollback(elevation),
            Self::BuildVm(args) => args.build_vm(&elevation),
            Self::BuildImage(args) => args.build_image(&elevation),
            Self::Search(args) => args.run(),
            Self::Clean(proxy) => proxy.command.run(elevation),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{env, ffi::OsString};

    use clap::{Parser, error::ErrorKind};
    use nh_clean::args::CleanMode;
    use serial_test::serial;

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
    #[serial]
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
}
