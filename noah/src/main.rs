#![expect(clippy::exhaustive_enums, reason = "App only, not published")]

mod clean;
mod cli;
mod command;
mod diff;
mod nix_options;
mod nixos;
mod progress;
mod remote;
mod runtime;
mod search;
mod update;
mod util;
mod weather;

use crate::cli::CliCommand;
use crate::runtime::Config;
use crate::runtime::Env;

use ino_shell::Shell;
use ino_shell::cmd;
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

    let args = cli::cli().run();

    tracing::debug!("{args:#?}");
    tracing::debug!(%NH_VERSION, ?NH_REV);

    // Capture environment-derived configuration once, after bpaf has handled
    // early exits such as --help and --version.
    let env = Env::capture()?;
    let config = Config::from_env(env, args.elevation_strategy)?;

    // Validate the Nix environment before dispatching the command.
    nix_variant()?;

    run(args.command, &config)
}

fn run(command: CliCommand, config: &Config) -> rootcause::Result<()> {
    match command {
        CliCommand::Rebuild(command) => {
            nixos::run_rebuild(*command, config)
        }
        CliCommand::Repl(request) => {
            nixos::run_repl(request, &config.env, &config.flake)
        }
        CliCommand::Info(request) => nixos::run_info(&request),
        CliCommand::Rollback(request) => {
            nixos::run_rollback(request, config)
        }
        CliCommand::Search(request) => search::run(&request),
        CliCommand::Clean(request) => clean::run(&request, config),
    }
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
