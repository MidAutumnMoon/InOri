//! Managing a NixOS system.
//!
//! The `switch`, `boot`, `test`, and `build` commands share one rebuild
//! flow ([`rebuild`]); `repl`, `info`, and `rollback` are self-contained.
//! This module holds what the family shares: profile locations,
//! specialisation selection, and the root guard.

pub mod cli;
mod diff;
pub mod info;
pub mod rebuild;
pub mod repl;
pub mod rollback;
mod update;

use std::fs;
use std::path::Path;
use std::path::PathBuf;

use rootcause::Result;
use rootcause::bail;
use rootcause::report;
use tracing::warn;

use crate::elevation::Elevation;

pub const SYSTEM_PROFILE: &str = "/nix/var/nix/profiles/system";
pub const CURRENT_PROFILE: &str = "/run/current-system";

const SPEC_LOCATION: &str = "/etc/specialisation";

/// Which specialisation of a configuration to select.
#[derive(Clone, Debug)]
pub enum SpecialisationSelection {
    /// The specialisation currently recorded in `/etc/specialisation`.
    Current,
    /// The base configuration, ignoring specialisations.
    Base,
    /// A named specialisation.
    Named(String),
}

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

/// Path of a named specialisation inside a system profile, if it exists.
///
/// Whether a missing specialisation is an error or falls back to the base
/// profile is a per-flow policy: rebuilds fail (the spec was just built),
/// rollbacks fall back to the base configuration with a warning.
fn specialisation_in(profile: &Path, spec: &str) -> Option<PathBuf> {
    let spec_link = profile.join("specialisation").join(spec);
    spec_link.exists().then_some(spec_link)
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

/// Decides whether privileged commands must run through elevation, and
/// enforces the root guard on the way.
///
/// Returns `true` when the caller is not root and elevation is enabled, so
/// child processes must escalate. Returns `false` when the process already
/// runs as root or when elevation is disabled.
///
/// # Arguments
///
/// * `bypass_root_check` - If true, running as root does not error.
///
/// # Errors
///
/// Returns an error if `bypass_root_check` is false and the user is root,
/// as the os subcommands should not be run directly as root.
fn requires_elevation(
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

/// Generation number of a profile-directory entry such as `system-42-link`.
pub fn from_dir(generation_dir: &Path) -> Option<u64> {
    let generation_base = generation_dir
        .file_name()
        .and_then(|os_str| os_str.to_str())?;
    let no_link_gen = generation_base.trim_end_matches("-link");
    let (_, generation_num) = no_link_gen.rsplit_once('-')?;
    generation_num.parse::<u64>().ok()
}

/// Whether the system currently runs this generation.
pub fn is_current(generation_dir: &Path) -> bool {
    let Some(run_current_target) = fs::read_link(CURRENT_PROFILE)
        .ok()
        .and_then(|path| fs::canonicalize(path).ok())
    else {
        return false;
    };

    let Some(gen_store_path) = fs::read_link(generation_dir)
        .ok()
        .and_then(|path| fs::canonicalize(path).ok())
    else {
        return false;
    };

    run_current_target == gen_store_path
}
