use std::str::FromStr;

use color_eyre::Result;
use ino_shell::{Shell, cmd};
use nh::EyreRootcauseBridge;
use nh_core::command::{
    ElevationStrategy, ElevationStrategyArg, SudoConfig, SubprocessEnv,
};
use nh_installable::FlakeConfig;
use nh_remote::SshConfig;
use nh_search::GithubConfig;
use rootcause::prelude::ResultExt;
use tracing::debug;

mod interface;

const NH_VERSION: &str = env!("CARGO_PKG_VERSION");
const NH_REV: Option<&str> = option_env!("NH_REV");

/// All runtime configuration captured from the environment at startup.
///
/// Assembled once in `main()`, passed by reference to subcommands.
/// Each subsystem receives only the slice it needs.
struct RuntimeEnv {
    subprocess: SubprocessEnv,
    sudo: SudoConfig,
    flake: FlakeConfig,
    ssh: SshConfig,
    github: GithubConfig,
}

/// Variant of the system Nix. Determinate Nix is not supported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NixVariant {
    Nix,
    Lix,
}

fn main() -> rootcause::Result<()> {
    ino_tracing::init_tracing_subscriber();
    let mut args = <interface::Main as clap::Parser>::parse();

    color_eyre::config::HookBuilder::default()
        .display_location_section(true)
        .panic_section(
            "Please report the bug at https://github.com/nix-community/nh/issues",
        )
        .display_env_section(false)
        .install().into_rootcause()?;

    // Capture all environment-derived configuration once at startup.
    // Subcommands receive only the slices they need.
    // Validate the Nix environment before doing anything else.
    nix_variant()?;
    let env = RuntimeEnv {
        subprocess: SubprocessEnv::from_env(),
        sudo: SudoConfig::from_env(),
        flake: FlakeConfig::from_env(),
        ssh: SshConfig::from_env(),
        github: GithubConfig::from_env(),
    };

    // Backward compatibility: support NH_ELEVATION_PROGRAM env var if
    // NH_ELEVATION_STRATEGY is not set.
    // TODO: Remove this fallback in a future version
    if args.elevation_strategy.is_none()
        && let Some(old_value) = std::env::var("NH_ELEVATION_PROGRAM")
            .ok()
            .filter(|v| !v.is_empty())
    {
        tracing::warn!(
            "NH_ELEVATION_PROGRAM is deprecated, use NH_ELEVATION_STRATEGY instead. \
            Falling back to NH_ELEVATION_PROGRAM for backward compatibility. \
            Accepted values: none, passwordless, program:<path>"
        );
        match ElevationStrategyArg::from_str(&old_value) {
            Ok(strategy) => args.elevation_strategy = Some(strategy),
            Err(e) => {
                tracing::warn!(
                    "Failed to parse NH_ELEVATION_PROGRAM value '{}': {}. Falling back \
                    to none.",
                    old_value,
                    e
                );
            }
        }
    }

    tracing::debug!("{args:#?}");
    tracing::debug!(%NH_VERSION, ?NH_REV);

    let elevation = args.elevation_strategy.as_ref().map_or(
        ElevationStrategy::Auto,
        |arg| match arg {
            ElevationStrategyArg::Auto => ElevationStrategy::Auto,
            ElevationStrategyArg::None => ElevationStrategy::None,
            ElevationStrategyArg::Passwordless => {
                ElevationStrategy::Passwordless
            }
            ElevationStrategyArg::Program(path) => {
                ElevationStrategy::Prefer(path.clone())
            }
        },
    );

    args.command.run(&env, elevation).into_rootcause()
}

fn nix_variant() -> rootcause::Result<NixVariant> {
    let variant = guess_nix_variant_from_version_output()?;
    ensure_features_needed_are_set()?;
    Ok(variant)
}

fn guess_nix_variant_from_version_output() -> rootcause::Result<NixVariant>
{
    let shell = Shell::new()?;
    let version_output = cmd!(shell, "nix --version")
        .read()
        .context("Failed to run `nix --version`")?;

    if version_output.to_lowercase().contains("lix") {
        Ok(NixVariant::Lix)
    } else {
        Ok(NixVariant::Nix)
    }
}

fn ensure_features_needed_are_set() -> rootcause::Result<()> {
    let shell = Shell::new()?;
    let expr_features =
        cmd!(shell, "nix config show experimental-features")
            .read()
            .context("Failed to read enabled experimental features")?;

    debug!(expr_features);

    if expr_features.contains("flakes")
        && expr_features.contains("nix-command")
    {
        Ok(())
    } else {
        rootcause::bail!(
            "Required flake features (nix-command, flakes) are not enabled"
        )
    }
}

