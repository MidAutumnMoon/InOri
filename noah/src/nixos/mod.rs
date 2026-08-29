mod cli;
mod request;
pub use cli::{
    boot_cli, build_cli, generations_cli, repl_cli, rollback_cli,
    switch_cli, test_cli,
};
pub use request::{
    GenerationsRequest, RebuildCommand, ReplRequest, RollbackRequest,
};
pub mod generations;
mod info;
mod rebuild;
mod repl;
mod rollback;

use nh_installable::FlakeConfig;
use rootcause::Result;
use rootcause::bail;
use rootcause::report;
use std::fs;
use tracing::warn;

use self::request::SpecialisationSelection;
use crate::command::Elevation;
use crate::runtime::Config;
use crate::runtime::Env;

const SYSTEM_PROFILE: &str = "/nix/var/nix/profiles/system";
const CURRENT_PROFILE: &str = "/run/current-system";

const SPEC_LOCATION: &str = "/etc/specialisation";
fn resolve_specialisation(
    selection: &SpecialisationSelection,
) -> Option<String> {
    match selection {
        SpecialisationSelection::Current => {
            fs::read_to_string(SPEC_LOCATION)
                .ok()
                .map(|content| content.trim().to_owned())
        }
        SpecialisationSelection::Base => None,
        SpecialisationSelection::Named(name) => Some(name.clone()),
    }
}

/// Returns an error indicating that the 'switch-to-configuration' binary is
/// missing, along with common reasons and solutions.
fn missing_switch_to_configuration_error() -> rootcause::Report {
    report!(
        "The 'switch-to-configuration' binary is missing from the built \
     configuration.\n\nThis typically happens when 'system.switch.enable' is \
     set to false in your\nNixOS configuration. To fix this, please \
     either:\n1. Remove 'system.switch.enable = false' from your \
     configuration, or\n2. Set 'system.switch.enable = true' explicitly\n\nIf \
     the problem persists, please open an issue on our issue tracker!"
    )
}

/// Checks if the current user is root and returns whether elevation is needed.
///
/// Returns `true` if elevation is required (not root and `bypass_root_check` is
/// false). Returns `false` if elevation is not required (root,
/// `bypass_root_check` is true, or elevation is disabled).
///
/// # Arguments
///
/// * `bypass_root_check` - If true, bypasses the root check and assumes no
///   elevation is needed.
///
/// # Errors
///
/// Returns an error if `bypass_root_check` is false and the user is root,
/// as `nh os` subcommands should not be run directly as root.
fn has_elevation_status(
    bypass_root_check: bool,
    elevation: &Elevation,
) -> Result<bool> {
    use nix::unistd::Uid;

    // If elevation is disabled, never elevate. This also skips the root
    // guard: with elevation disabled there is nothing to escalate, so
    // running as root is legitimate.
    if elevation.is_disabled() {
        return Ok(false);
    }

    let is_root = Uid::effective().is_root();

    if is_root && !bypass_root_check {
        bail!(
            "Don't run nh os as root. It will escalate its privileges internally as \
       needed."
        );
    }

    if bypass_root_check {
        warn!(
            "Bypassing root check; running nix as {}",
            if is_root { "root" } else { "non-root" }
        );
    }

    Ok(!is_root)
}

pub fn run_rebuild(
    command: RebuildCommand,
    config: &Config,
) -> Result<()> {
    rebuild::run(command, config)
}

pub fn run_rollback(
    request: RollbackRequest,
    config: &Config,
) -> Result<()> {
    rollback::run(request, config)
}

pub fn run_repl(
    request: ReplRequest,
    env: &Env,
    flake_config: &FlakeConfig,
) -> Result<()> {
    repl::run(request, env, flake_config)
}

pub fn run_info(request: &GenerationsRequest) -> Result<()> {
    info::run(request)
}
