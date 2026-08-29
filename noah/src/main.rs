#![expect(
    clippy::exhaustive_structs,
    clippy::exhaustive_enums,
    reason = "App only, not published"
)]

mod app;
pub mod clean;
mod cli;
pub mod command;
pub mod diff;
mod nix_options;
pub mod nixos;
pub mod progress;
pub mod remote;
pub mod runtime;
pub mod search;
pub mod update;
pub mod util;

use crate::command::ElevationStrategy;
use crate::command::ElevationStrategyArg;
use crate::runtime::RuntimeEnv;

use ino_shell::{Shell, cmd};
use rootcause::prelude::ResultExt as _;
use tracing::debug;

const NH_VERSION: &str = env!("CARGO_PKG_VERSION");
const NH_REV: Option<&str> = option_env!("NH_REV");

/// Converts an error produced by an external crate into a
/// [`rootcause::Report`].
///
/// Dependencies that don't speak rootcause (e.g. `dix`) report their failures
/// through the boxed-error representation; this is the seam where such
/// errors enter rootcause.
pub(crate) fn external_report<E>(err: E) -> rootcause::Report
where
    Box<dyn std::error::Error + Send + Sync>: From<E>,
{
    rootcause::report!(Box::<dyn std::error::Error + Send + Sync>::from(
        err
    ))
    .into()
}

/// Variant of the system Nix. Determinate Nix is not supported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NixVariant {
    Nix,
    Lix,
}

fn main() -> rootcause::Result<()> {
    let _log_guard = ino_tracing::init_tracing_subscriber();

    // Panic diagnostics: point users at the issue tracker for bug reports.
    std::panic::set_hook(Box::new(|info| {
        eprintln!("{info}");
        eprintln!(
            "Please report the bug at https://github.com/nix-community/nh/issues"
        );
    }));

    let mut args = cli::cli().run();

    // Capture environment-derived configuration once, after bpaf has handled
    // early exits such as --help and --version.
    let process = RuntimeEnv::capture()?;
    let env = app::RuntimeConfig::from_env(process)?;

    // Validate the Nix environment before dispatching the command.
    nix_variant()?;

    // Backward compatibility: support NH_ELEVATION_PROGRAM env var if
    // NH_ELEVATION_STRATEGY is not set.
    // TODO: Remove this fallback in a future version
    if args.elevation_strategy.is_none()
        && let Some(old_value) =
            env.process.non_empty_var("NH_ELEVATION_PROGRAM")
    {
        tracing::warn!(
            "NH_ELEVATION_PROGRAM is deprecated, use NH_ELEVATION_STRATEGY instead. \
            Falling back to NH_ELEVATION_PROGRAM for backward compatibility. \
            Accepted values: none, passwordless, program:<path>"
        );
        args.elevation_strategy = Some(old_value.into());
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

    app::run(args.command, &env, elevation)
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
