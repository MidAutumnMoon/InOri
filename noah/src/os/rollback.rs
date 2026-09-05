//! The `rollback` command: switch the system profile to an older
//! generation and activate it.

use std::fs;
use std::path::Path;
use std::path::PathBuf;

use bpaf::Parser;
use bpaf::construct;
use bpaf::long;
use rootcause::Result;
use rootcause::bail;
use rootcause::prelude::ResultExt as _;
use rootcause::report;
use tracing::debug;
use tracing::info;
use tracing::warn;

use super::SYSTEM_PROFILE;
use super::SpecialisationSelection;
use super::diff::DiffMode;
use super::from_dir;
use super::requires_elevation;
use super::is_current;
use super::missing_switch_to_configuration_error;
use super::resolve_specialisation;
use super::specialisation_in;
use crate::command::Command;
use crate::runtime::Config;

#[derive(Clone, Debug)]
pub struct Request {
    pub dry: bool,
    pub ask: bool,
    pub specialisation: SpecialisationSelection,
    pub to: Option<u64>,
    pub bypass_root_check: bool,
    pub diff: DiffMode,
}

/// Parse the `rollback` command.
#[must_use]
pub fn cli() -> impl Parser<Request> {
    let dry = long("dry")
        .short('n')
        .switch()
        .help("Only print actions, without performing them");
    let ask = long("ask").short('a').help("Ask for confirmation").switch();
    let specialisation = super::cli::specialisation_cli();
    let to = long("to")
        .short('t')
        .argument::<u64>("GENERATION")
        .help(
            "Rollback to a specific generation number (defaults to previous \
             generation)",
        )
        .optional();
    let bypass_root_check = long("bypass-root-check")
        .short('R')
        .help("Don't panic if calling nh as root")
        .switch();
    let diff = long("diff")
        .short('d')
        .argument::<DiffMode>("DIFF")
        .help("Whether to display a package diff")
        .fallback(DiffMode::Auto)
        .display_fallback();

    construct!(Request {
        dry,
        ask,
        specialisation,
        to,
        bypass_root_check,
        diff,
    })
}

/// Run a rollback request.
///
/// # Errors
///
/// Returns an error if the profile cannot be switched or activation fails.
pub fn run(request: &Request, config: &Config) -> Result<()> {
    request.rollback(config)
}

/// The facts about a generation entry readable from the profiles directory
/// alone, without the metadata queries the full description performs.
#[derive(Debug, Clone)]
struct GenerationSummary {
    /// Number of a generation.
    number: u64,

    /// Whether the system currently runs this generation.
    current: bool,
}

/// Summarize a generation entry in the profiles directory.
fn summarize(generation_dir: &Path) -> Option<GenerationSummary> {
    Some(GenerationSummary {
        number: from_dir(generation_dir)?,
        current: is_current(generation_dir),
    })
}

impl Request {
    #[expect(
        clippy::too_many_lines,
        reason = "linear rollback flow whose errors carry their own context"
    )]
    fn rollback(&self, config: &Config) -> Result<()> {
        let elevate = requires_elevation(
            self.bypass_root_check,
            &config.elevation,
        )?;

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

        // Compare the currently running system with the target generation.
        // A failed comparison must not block the rollback itself.
        if let Err(error) = super::diff::run(&self.diff, &generation_link)
        {
            warn!(%error, "Failed to compare the current and target profiles");
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
        Command::new("ln", &config.env, &config.elevation)
            .arg("-sfn") // force, symbolic link
            .arg(&generation_link)
            .arg(SYSTEM_PROFILE)
            .elevate(elevate)
            .message("Setting system profile")
            .run()
            .context("Failed to set system profile during rollback")?;

        // Determine the correct profile to use with specialisations
        let final_profile = match &target_specialisation {
            None => generation_link,
            Some(spec) => {
                specialisation_in(&generation_link, spec).unwrap_or_else(|| {
                    warn!(
                        "Specialisation '{spec}' does not exist in generation {}",
                        target_generation.number
                    );
                    warn!("Using base configuration without specialisations");
                    generation_link.clone()
                })
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
            &config.env,
            &config.elevation,
        )
        .arg("switch")
        .elevate(elevate)
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

                    Command::new("ln", &config.env, &config.elevation)
                        .arg("-sfn") // Force, symbolic link
                        .arg(&current_gen_link)
                        .arg(SYSTEM_PROFILE)
                        .elevate(elevate)
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
    generations: &[GenerationSummary],
) -> Result<GenerationSummary> {
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
    generations: &[GenerationSummary],
) -> Result<&GenerationSummary> {
    generations
        .iter()
        .find(|generation| generation.number == number)
        .ok_or_else(|| report!("Generation {} not found", number))
}

fn list_generations() -> Result<Vec<GenerationSummary>> {
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
            && let Some(summary) = summarize(&path)
        {
            generations.push(summary);
        }
    }

    if generations.is_empty() {
        bail!("No generations found");
    }

    tracing::debug!("{} generations found", generations.len());

    generations.sort_by_key(|generation| generation.number);

    Ok(generations)
}
