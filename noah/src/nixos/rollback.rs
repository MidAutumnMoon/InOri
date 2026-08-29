use std::fs;
use std::ops::Deref;
use std::path::{Path, PathBuf};

use rootcause::{Result, bail, prelude::ResultExt as _, report};
use tracing::{debug, info, warn};

use super::request::RollbackRequest as ParsedRollbackRequest;
use super::{
    CURRENT_PROFILE, SYSTEM_PROFILE, generations, has_elevation_status,
    missing_switch_to_configuration_error, resolve_specialisation,
};
use crate::command::{Command, ElevationStrategy, SudoConfig};
use crate::diff::{Mode as DiffMode, print_dix_report};
use crate::runtime::RuntimeEnv;
struct Rollback(ParsedRollbackRequest);

impl Deref for Rollback {
    type Target = ParsedRollbackRequest;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub(super) fn run(
    request: ParsedRollbackRequest,
    elevation: ElevationStrategy,
    runtime_env: &RuntimeEnv,
    sudo_config: &SudoConfig,
) -> Result<()> {
    Rollback(request).rollback(elevation, runtime_env, sudo_config)
}

impl Rollback {
    #[expect(
        clippy::too_many_lines,
        reason = "linear rollback flow whose errors carry their own context"
    )]
    fn rollback(
        &self,
        elevation: ElevationStrategy,
        runtime_env: &RuntimeEnv,
        sudo_config: &SudoConfig,
    ) -> Result<()> {
        let elevate =
            has_elevation_status(self.bypass_root_check, &elevation)?;

        let generations = list_generations()?;

        let current_generation = generations
            .iter()
            .find(|generation| generation.current)
            .ok_or_else(|| report!("Current generation not found"))?;

        // Find previous generation or specific generation
        let target_generation = if let Some(gen_number) = self.to {
            get_generation_by_number(gen_number, &generations)?
        } else {
            &find_previous_generation(
                current_generation.number,
                &generations,
            )?
        };

        info!("Rolling back to generation {}", target_generation.number);

        // Construct path to the generation
        let profile_dir = Path::new(SYSTEM_PROFILE).parent().unwrap_or_else(|| {
      tracing::warn!(
        "SYSTEM_PROFILE has no parent, defaulting to /nix/var/nix/profiles"
      );
      Path::new("/nix/var/nix/profiles")
    });
        let generation_link = profile_dir
            .join(format!("system-{}-link", target_generation.number));

        // Handle specialisations

        let target_specialisation =
            resolve_specialisation(&self.specialisation);

        debug!("target_specialisation: {target_specialisation:?}");

        // Compare changes between current and target generation
        if matches!(self.diff, DiffMode::Never) {
            debug!(
                "Not running dix as the target hostname is different from the system \
         hostname."
            );
        } else {
            debug!(
                "Comparing with target profile: {}",
                generation_link.display()
            );
            if let Err(error) = print_dix_report(
                &PathBuf::from(CURRENT_PROFILE),
                &generation_link,
            ) {
                warn!(%error, "Failed to compare the current and target profiles");
            }
        }

        if self.dry {
            info!(
                "Dry run: would roll back to generation {}",
                target_generation.number
            );
            return Ok(());
        }

        if self.ask {
            let confirmation = inquire::Confirm::new(&format!(
                "Roll back to generation {}?",
                target_generation.number
            ))
            .with_default(false)
            .prompt()?;

            if !confirmation {
                bail!("User rejected the rollback");
            }
        }

        // Set the system profile
        info!("Setting system profile...");

        // Instead of direct symlink operations, use a command with proper elevation
        Command::new("ln", runtime_env, sudo_config)
            .arg("-sfn") // force, symbolic link
            .arg(&generation_link)
            .arg(SYSTEM_PROFILE)
            .elevate(elevate.then_some(elevation.clone()))
            .message("Setting system profile")
            .run()
            .context("Failed to set system profile during rollback")?;

        // Determine the correct profile to use with specialisations
        let final_profile = match &target_specialisation {
            None => generation_link,
            Some(spec) => {
                let spec_path =
                    generation_link.join("specialisation").join(spec);
                if spec_path.exists() {
                    spec_path
                } else {
                    warn!(
                        "Specialisation '{}' does not exist in generation {}",
                        spec, target_generation.number
                    );
                    warn!(
                        "Using base configuration without specialisations"
                    );
                    generation_link
                }
            }
        };

        // Activate the configuration
        info!("Activating...");

        let switch_to_configuration =
            final_profile.join("bin").join("switch-to-configuration");

        if !switch_to_configuration.exists() {
            return Err(missing_switch_to_configuration_error());
        }

        match Command::new(
            &switch_to_configuration,
            runtime_env,
            sudo_config,
        )
        .arg("switch")
        .elevate(elevate.then_some(elevation.clone()))
        .preserve_envs(["NIXOS_INSTALL_BOOTLOADER", "NIXOS_NO_CHECK"])
        .run()
        {
            Ok(()) => {
                info!(
                    "Successfully rolled back to generation {}",
                    target_generation.number
                );
            }
            Err(err) => {
                // If activation fails, rollback the profile
                if current_generation.number > 0 {
                    let current_gen_link = profile_dir.join(format!(
                        "system-{}-link",
                        current_generation.number
                    ));

                    Command::new("ln", runtime_env, sudo_config)
                        .arg("-sfn") // Force, symbolic link
                        .arg(&current_gen_link)
                        .arg(SYSTEM_PROFILE)
                        .elevate(elevate.then_some(elevation))
                        .message("Rolling back system profile")
                        .run()
                        .context("NixOS: Failed to restore previous system profile after failed activation")?;
                }

                return Err(report!(
                    "Activation (switch) failed: {}",
                    err
                )
                .context("Failed to activate configuration")
                .into());
            }
        }

        Ok(())
    }
}
fn find_previous_generation(
    current_number: u64,
    generations: &[generations::GenerationInfo],
) -> Result<generations::GenerationInfo> {
    let current = generations
        .iter()
        .find(|generation| generation.number == current_number)
        .ok_or_else(|| report!("Current generation not found"))?;

    generations
        .iter()
        .rev()
        .find(|generation| generation.number < current.number)
        .cloned()
        .ok_or_else(|| {
            report!("No generation older than the current one exists")
        })
}

fn get_generation_by_number(
    number: u64,
    generations: &[generations::GenerationInfo],
) -> Result<&generations::GenerationInfo> {
    generations
        .iter()
        .find(|generation| generation.number == number)
        .ok_or_else(|| report!("Generation {} not found", number))
}

fn list_generations() -> Result<Vec<generations::GenerationInfo>> {
    let profile_path = PathBuf::from(SYSTEM_PROFILE);
    let profiles_dir = profile_path
        .parent()
        .unwrap_or_else(|| Path::new("/nix/var/nix/profiles"));

    let mut generations = Vec::new();
    for entry in fs::read_dir(profiles_dir)? {
        let entry = match entry {
            Ok(dir_entry) => dir_entry,
            Err(err) => {
                warn!(
                    "Failed to read entry in profile directory: {}",
                    err
                );
                continue;
            }
        };

        let path = entry.path();
        if let Some(name) =
            path.file_name().and_then(|os_str| os_str.to_str())
            && name.starts_with("system-")
            && name.ends_with("-link")
            && let Some(gen_info) = generations::describe(&path, None)
        {
            generations.push(gen_info);
        }
    }

    if generations.is_empty() {
        bail!("No generations found");
    }

    tracing::debug!("{} generations found", generations.len());

    generations.sort_by_key(|generation| generation.number);

    Ok(generations)
}
