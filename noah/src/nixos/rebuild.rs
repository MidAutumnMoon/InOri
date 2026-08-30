use std::ops::Deref;
use std::path::{Path, PathBuf};

use rootcause::{Result, bail, prelude::ResultExt as _, report};
use tracing::{debug, info, warn};

use super::request::{
    Activation, ActivationAction,
    ActivationRequest as ParsedActivationRequest, RebuildCommand,
    RebuildRequest,
};
use super::{SYSTEM_PROFILE, has_elevation_status, resolve_specialisation};
use crate::command::{self, Command, Elevation};
use crate::diff::handle_nixos;
use crate::runtime::Config;
use crate::runtime::Env;
use crate::target::{self, BuildTarget};
use crate::update::update;

struct Rebuild(RebuildRequest);

impl Deref for Rebuild {
    type Target = RebuildRequest;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

struct ActivationRequest {
    rebuild: Rebuild,
    activation: Activation,
}

impl From<ParsedActivationRequest> for ActivationRequest {
    fn from(request: ParsedActivationRequest) -> Self {
        Self {
            rebuild: Rebuild(request.rebuild),
            activation: request.activation,
        }
    }
}

pub(super) fn run(command: RebuildCommand, config: &Config) -> Result<()> {
    match command {
        RebuildCommand::Build(request) => {
            Rebuild(request).build_only(config)
        }
        RebuildCommand::Activate(request) => {
            ActivationRequest::from(request).build_and_activate(config)
        }
    }
}

/// Essential files that must exist in a valid NixOS system closure. Each tuple
/// contains the file path relative to the system profile and its description.
/// The descriptions are used on log messages or errors.
const ESSENTIAL_FILES: &[(&str, &str)] = &[
    ("bin/switch-to-configuration", "activation script"),
    ("nixos-version", "system version identifier"),
    ("init", "system init script"),
    ("sw/bin", "system path"),
];

impl ActivationRequest {
    fn build_and_activate(&self, config: &Config) -> Result<()> {
        let (elevate, target_hostname) = self
            .rebuild
            .setup_build_context(&config.elevation, &config.env)?;

        let (out_path, _tempdir_guard) =
            self.rebuild.determine_output_path(true)?;

        let toplevel =
            self.rebuild.prepare_toplevel(&target_hostname, &config.env)?;

        let target_profile =
            self.rebuild.build_and_diff(toplevel, &out_path)?;

        if self.activation.dry {
            if self.activation.ask {
                warn!("--ask has no effect as dry run was requested");
            }

            return Ok(());
        }

        self.activate_rebuilt_config(
            &out_path,
            &target_profile,
            elevate,
            config,
        )?;

        Ok(())
    }

    fn activate_rebuilt_config(
        &self,
        out_path: &Path,
        target_profile: &Path,
        elevate: bool,
        config: &Config,
    ) -> Result<()> {
        let action = self.activation.action;

        if self.activation.ask {
            let confirmation = inquire::Confirm::new("Apply the config?")
                .with_default(false)
                .prompt()?;

            if !confirmation {
                bail!("User rejected the new config");
            }
        }

        let switch_to_configuration =
            self.resolve_closure(target_profile)?;

        match action {
            ActivationAction::Test { .. } => {
                self.activate_test_phase(
                    &switch_to_configuration,
                    elevate,
                    config,
                )?;
            }
            ActivationAction::Boot { .. } => {
                self.activate_boot_phase(
                    out_path,
                    &switch_to_configuration,
                    elevate,
                    config,
                )?;
            }
            ActivationAction::Switch { .. } => {
                self.activate_test_phase(
                    &switch_to_configuration,
                    elevate,
                    config,
                )?;
                self.activate_boot_phase(
                    out_path,
                    &switch_to_configuration,
                    elevate,
                    config,
                )?;
            }
        }

        debug!(
            "Completed {action:?} operation with output path: {out_path:?}"
        );

        Ok(())
    }

    /// Validates the closure to activate and resolves the path to its
    /// `switch-to-configuration` binary.
    fn resolve_closure(&self, target_profile: &Path) -> Result<PathBuf> {
        let resolved_profile = target_profile.canonicalize().context(
            "Failed to resolve output path to actual store path",
        )?;

        if self.activation.no_validate {
            warn!(
                "Skipping pre-activation validation (--no-validate or NH_NO_VALIDATE \
         set)"
            );
            warn!(
                "This may result in activation failures if the system closure is \
         incomplete"
            );
        } else {
            validate_system_closure(&resolved_profile)?;
        }

        let switch_to_configuration = resolved_profile
            .join("bin")
            .join("switch-to-configuration")
            .canonicalize()
            .context("Failed to resolve switch-to-configuration path")?;

        Ok(switch_to_configuration)
    }

    /// Runs the test phase of activation.
    fn activate_test_phase(
        &self,
        switch_to_configuration: &Path,
        elevate: bool,
        config: &Config,
    ) -> Result<()> {
        Command::new(
            switch_to_configuration,
            &config.env,
            &config.elevation,
        )
        .arg("test")
        .message("Activating configuration")
        .elevate(elevate)
        .preserve_envs(["NIXOS_INSTALL_BOOTLOADER", "NIXOS_NO_CHECK"])
        .show_output(self.activation.action.show_logs())
        .run()
        .context("Activation (test) failed")?;

        Ok(())
    }

    /// Sets the system profile and installs the bootloader entry.
    fn activate_boot_phase(
        &self,
        out_path: &Path,
        switch_to_configuration: &Path,
        elevate: bool,
        config: &Config,
    ) -> Result<()> {
        // Use the base system closure instead of the specialisation one.
        // This is what makes all specialisations visible in the bootloader
        // instead of only the generation with the specialisation.
        let base_store_path = out_path
            .canonicalize()
            .context("Failed to resolve base output path to store path")?;

        Command::new("nix", &config.env, &config.elevation)
            .args(["build", "--no-link", "--profile", SYSTEM_PROFILE])
            .arg(&base_store_path)
            .elevate(elevate)
            .run()
            .context("Failed to set system profile")?;

        let mut cmd = Command::new(
            switch_to_configuration,
            &config.env,
            &config.elevation,
        )
        .arg("boot")
        .elevate(elevate)
        .message("Adding configuration to bootloader")
        .preserve_envs(["NIXOS_INSTALL_BOOTLOADER", "NIXOS_NO_CHECK"]);

        if self.activation.action.install_bootloader() {
            cmd = cmd.set_env("NIXOS_INSTALL_BOOTLOADER", "1");
        }

        cmd.run()
            .context("Bootloader activation failed")?;

        Ok(())
    }
}

impl Rebuild {
    /// Performs initial setup for an OS rebuild operation:
    ///
    /// - Determining whether activation needs elevation (and enforcing the
    ///   root guard unless bypassed).
    /// - Resolving the target hostname for the build.
    ///
    /// # Returns
    ///
    /// A tuple of:
    ///
    /// - `bool`: `true` if activation requires elevation.
    /// - `String`: The resolved target hostname.
    fn setup_build_context(
        &self,
        elevation: &Elevation,
        env: &Env,
    ) -> Result<(bool, String)> {
        let elevate = has_elevation_status(self.bypass_root_check, elevation)?;

        let target_hostname = match &self.hostname {
            Some(hostname) => hostname.clone(),
            None => env.hostname().to_owned(),
        };

        Ok((elevate, target_hostname))
    }

    fn determine_output_path(
        &self,
        temporary: bool,
    ) -> Result<(PathBuf, Option<tempfile::TempDir>)> {
        if let Some(path) = self.build.out_link.clone() {
            return Ok((path, None));
        }

        if temporary {
            let dir =
                tempfile::Builder::new().prefix("nh-os").tempdir()?;
            Ok((dir.as_ref().join("result"), Some(dir)))
        } else {
            Ok((PathBuf::from("result"), None))
        }
    }

    fn prepare_toplevel(
        &self,
        target_hostname: &str,
        env: &Env,
    ) -> Result<BuildTarget> {
        const TOPLEVEL_ATTRS: [&str; 4] =
            ["config", "system", "build", "toplevel"];

        let mut toplevel = target::resolve(self.build.target.clone(), env)?;

        match &mut toplevel {
            BuildTarget::Flake { attribute, .. } => {
                let second = attribute
                    .get(1)
                    .map_or_else(String::new, String::clone);
                match attribute.first().map(String::as_str) {
                    None => {
                        attribute.push(String::from("nixosConfigurations"));
                        attribute.push(target_hostname.to_owned());
                    }
                    Some("nixosConfigurations") => match attribute.len() {
                        1 => {
                            info!(
                                "Inferring hostname '{target_hostname}' for \
                                 nixosConfigurations"
                            );
                            attribute.push(target_hostname.to_owned());
                        }
                        2 => {}
                        _ => {
                            bail!(
                                "Attribute path is too specific: {attribute}. Please \
                                 either:\n  1. Use the flake reference without \
                                 attributes (e.g., '.')\n  2. Specify only the \
                                 configuration name (e.g., '.#{second}')"
                            );
                        }
                    },
                    Some(_) => {
                        attribute.insert(
                            0,
                            String::from("nixosConfigurations"),
                        );
                    }
                }
                attribute.extend(TOPLEVEL_ATTRS.map(str::to_owned));
            }
            BuildTarget::File { attribute, .. }
            | BuildTarget::Expression { attribute, .. } => {
                attribute.extend(TOPLEVEL_ATTRS.map(str::to_owned));
            }
            BuildTarget::StorePath(_) => {}
        }

        if let Some(selection) = &self.update {
            update(&toplevel, selection, self.commit_lock_file)?;
        }

        Ok(toplevel)
    }

    fn execute_build(
        &self,
        toplevel: BuildTarget,
        out_path: &Path,
    ) -> Result<()> {
        const MESSAGE: &str = "Building NixOS configuration";

        command::Build::new(toplevel)
            .extra_arg("--out-link")
            .extra_arg(out_path)
            .extra_args(&self.extra_args)
            .nix_options(&self.build.nix)
            .message(MESSAGE)
            .nom(!self.build.no_nom)
            .run()
            .context("Failed to build configuration")?;

        Ok(())
    }

    fn build_and_diff(
        &self,
        toplevel: BuildTarget,
        out_path: &Path,
    ) -> Result<PathBuf> {
        self.execute_build(toplevel, out_path)?;
        let target_profile =
            self.resolve_specialisation_and_profile(out_path)?;

        handle_nixos(&self.build.diff, &target_profile)?;

        Ok(target_profile)
    }

    fn resolve_specialisation_and_profile(
        &self,
        out_path: &Path,
    ) -> Result<PathBuf> {
        let target_specialisation =
            resolve_specialisation(&self.specialisation);

        debug!("Target specialisation: {target_specialisation:?}");

        // Determine target profile, falling back to base if specialisation
        // doesn't exist
        let target_profile = match &target_specialisation {
            None => out_path.to_path_buf(),
            Some(spec) => {
                let spec_path = out_path.join("specialisation").join(spec);

                // Check if specialisation exists and fail if not
                if out_path.exists() && !spec_path.exists() {
                    bail!(
                        "Specialisation '{}' does not exist in the built configuration",
                        spec
                    );
                }

                spec_path
            }
        };

        debug!("Output path: {out_path:?}");
        debug!("Target profile path: {}", target_profile.display());

        // Validate the final target profile exists
        if out_path.exists() && !target_profile.exists() {
            return Err(report!(
                "Target profile path does not exist: {}",
                target_profile.display()
            ));
        }

        Ok(target_profile)
    }

    /// Builds the toplevel configuration without activating (`nh build`).
    fn build_only(&self, config: &Config) -> Result<()> {
        let (_, target_hostname) =
            self.setup_build_context(&config.elevation, &config.env)?;

        let (out_path, _tempdir_guard) =
            self.determine_output_path(false)?;

        let toplevel =
            self.prepare_toplevel(&target_hostname, &config.env)?;
        self.build_and_diff(toplevel, &out_path)?;

        Ok(())
    }
}

/// Validates that essential files exist in the system closure.
///
/// Checks for a few critical files that must be present in a complete NixOS
/// system. This is essentially in-line with what nixos-rebuild-ng checks for.
///
/// - bin/switch-to-configuration: activation script
/// - nixos-version: system version identifier
/// - init: system init script
/// - sw/bin: system path binaries
///
/// # Returns
///
/// `Ok(())` if all files exist, or an error listing missing files.
fn validate_system_closure(system_path: &Path) -> Result<()> {
    let mut missing = Vec::new();
    for (file, description) in ESSENTIAL_FILES {
        let path = system_path.join(file);
        if !path.exists() {
            missing.push(format!("  - {file} ({description})"));
        }
    }

    if !missing.is_empty() {
        let missing_list = missing.join("\n");
        return Err(report!(
            "System closure validation failed. Missing essential files:\n{}\n\nThis \
       typically happens when:\n1. 'system.switch.enable' is set to false in \
       your configuration\n2. The build was incomplete or corrupted\n3. \
       You're using an incomplete derivation\n\nTo fix this:\n1. Check if \
       'system.switch.enable = false' is set and remove it\n2. Rebuild your \
       system configuration\n3. If the problem persists, verify your system \
       closure is complete\n\nSystem path checked: {}",
            missing_list,
            system_path.display()
        ));
    }

    Ok(())
}
