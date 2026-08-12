use std::str::FromStr;

use color_eyre::Result;
use nh_core::command::{ElevationStrategy, ElevationStrategyArg};

use crate::nix_info::NixVariant;

mod checks;
mod interface;
mod nix_info;

const NH_VERSION: &str = env!("CARGO_PKG_VERSION");
const NH_REV: Option<&str> = option_env!("NH_REV");

pub struct Facts {
    envvars: Envvars,
    nix_variant: NixVariant,
}

pub struct Envvars {}

fn main() -> Result<()> {
    ino_tracing::init_tracing_subscriber();
    let mut args = <interface::Main as clap::Parser>::parse();

    color_eyre::config::HookBuilder::default()
        .display_location_section(true)
        .panic_section(
            "Please report the bug at https://github.com/nix-community/nh/issues",
        )
        .display_env_section(false)
        .install()?;

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

    // Check Nix version upfront
    checks::verify_nix_environment()?;

    // Once we assert required Nix features, validate NH environment checks
    // For now, this is just NH_* variables being set. More checks may be
    // added to setup_environment in the future.
    checks::verify_variables()?;

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

    args.command.run(elevation)
}
