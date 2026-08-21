use std::{
    convert::Into,
    fs,
    path::{Path, PathBuf},
};

use crate::diff::{handle_nixos_diff, print_dix_diff};
use crate::remote::{RemoteBuildConfig, RemoteHost, SshConfig};
use crate::{
    args::DiffType,
    command::{
        self, Command, CommandKind, ElevationStrategy, NixCommand,
        SubprocessEnv, SudoConfig,
    },
    update::update,
    util::{ensure_ssh_key_login, get_hostname},
};
use color_eyre::eyre::{Context, Result, bail, eyre};
use nh_installable::{FlakeConfig, Installable};
use tracing::{debug, info, warn};

use crate::nixos::{
    args::{
        GenerationsArgs, RebuildActivateArgs, RebuildArgs, ReplArgs,
        RollbackArgs,
    },
    generations,
};

const SYSTEM_PROFILE: &str = "/nix/var/nix/profiles/system";
const CURRENT_PROFILE: &str = "/run/current-system";

const SPEC_LOCATION: &str = "/etc/specialisation";

/// Essential files that must exist in a valid NixOS system closure. Each tuple
/// contains the file path relative to the system profile and its description.
/// The descriptions are used on log messages or errors.
const ESSENTIAL_FILES: &[(&str, &str)] = &[
    ("bin/switch-to-configuration", "activation script"),
    ("nixos-version", "system version identifier"),
    ("init", "system init script"),
    ("sw/bin", "system path"),
];

/// Post-build activation action. `Switch` runs the test phase, then the boot
/// phase.
#[derive(Debug, Clone, Copy)]
pub enum ActivationAction {
    Test,
    Boot,
    Switch,
}

struct BuiltConfiguration {
    target_profile: PathBuf,
    actual_store_path: Option<PathBuf>,
}

impl RebuildActivateArgs {
    #[expect(clippy::missing_errors_doc)]
    pub fn build_and_activate(
        &self,
        action: ActivationAction,
        elevation: ElevationStrategy,
        subprocess_env: &SubprocessEnv,
        sudo_config: &SudoConfig,
        flake_config: &FlakeConfig,
        ssh_config: &SshConfig,
    ) -> Result<()> {
        let (local_elevate, target_hostname) =
            self.rebuild.setup_build_context(
                &elevation,
                subprocess_env,
                sudo_config,
            )?;

        let (out_path, _tempdir_guard) =
            self.rebuild.determine_output_path(true)?;

        let toplevel = self
            .rebuild
            .prepare_toplevel(&target_hostname, flake_config)?;

        // Initialize SSH control early if we have remote hosts - guard will keep
        // connections alive for both build and activation
        let _ssh_guard = if self.rebuild.build_host.is_some()
            || self.rebuild.target_host.is_some()
        {
            let guard = crate::remote::init_ssh_control();

            // Pre-establish ControlMaster connections so that delegated SSH
            // invocations (e.g. `nix copy --to ssh://...`) reuse the already-
            // authenticated socket rather than opening a fresh connection where
            // SSH option ordering may differ.
            if let Some(build_host) = &self.rebuild.build_host {
                crate::remote::open_ssh_control_master(
                    build_host, ssh_config,
                )
                .context(
                    "Failed to establish SSH connection to build host",
                )?;
            }

            if let Some(target_host) = &self.rebuild.target_host {
                crate::remote::open_ssh_control_master(
                    target_host,
                    ssh_config,
                )
                .context(
                    "Failed to establish SSH connection to target host",
                )?;
            }

            Some(guard)
        } else {
            None
        };

        // Now that the ControlMaster is up, probe the remote uid for elevation.
        let elevate = if self.rebuild.target_host.is_some() {
            self.rebuild
                .determine_remote_elevation(&elevation, ssh_config)?
        } else {
            local_elevate
        };

        let built = self
            .rebuild
            .build_and_diff(toplevel, &out_path, ssh_config)?;

        if self.rebuild.common.dry {
            if self.rebuild.common.ask {
                warn!("--ask has no effect as dry run was requested");
            }

            return Ok(());
        }

        self.activate_rebuilt_config(
            action,
            &out_path,
            &built.target_profile,
            built.actual_store_path.as_deref(),
            elevate,
            elevation,
            subprocess_env,
            sudo_config,
            ssh_config,
        )?;

        Ok(())
    }

    #[expect(clippy::too_many_arguments)]
    fn activate_rebuilt_config(
        &self,
        action: ActivationAction,
        out_path: &Path,
        target_profile: &Path,
        actual_store_path: Option<&Path>,
        elevate: bool,
        elevation: ElevationStrategy,
        subprocess_env: &SubprocessEnv,
        sudo_config: &SudoConfig,
        ssh_config: &SshConfig,
    ) -> Result<()> {
        if self.rebuild.common.ask {
            let confirmation = inquire::Confirm::new("Apply the config?")
                .with_default(false)
                .prompt()?;

            if !confirmation {
                bail!("User rejected the new config");
            }
        }

        // Only copy if the output path exists locally (i.e., was copied back
        // from a remote build).
        if let Some(target_host) = &self.rebuild.target_host
            && out_path.exists()
        {
            crate::remote::copy_to_remote(
                target_host,
                target_profile,
                self.rebuild.common.passthrough.use_substitutes,
                ssh_config,
            )
            .context("Failed to copy configuration to target host")?;
        }

        let (resolved_profile, switch_to_configuration) = self
            .resolve_closure(
                out_path,
                target_profile,
                actual_store_path,
                ssh_config,
            )?;

        match action {
            ActivationAction::Test => {
                self.activate_test_phase(
                    &resolved_profile,
                    &switch_to_configuration,
                    ActivationAction::Test,
                    elevate,
                    &elevation,
                    subprocess_env,
                    sudo_config,
                    ssh_config,
                )?;
            }
            ActivationAction::Boot => {
                self.activate_boot_phase(
                    out_path,
                    &resolved_profile,
                    &switch_to_configuration,
                    elevate,
                    elevation,
                    subprocess_env,
                    sudo_config,
                    ssh_config,
                )?;
            }
            ActivationAction::Switch => {
                self.activate_test_phase(
                    &resolved_profile,
                    &switch_to_configuration,
                    ActivationAction::Switch,
                    elevate,
                    &elevation,
                    subprocess_env,
                    sudo_config,
                    ssh_config,
                )?;
                self.activate_boot_phase(
                    out_path,
                    &resolved_profile,
                    &switch_to_configuration,
                    elevate,
                    elevation,
                    subprocess_env,
                    sudo_config,
                    ssh_config,
                )?;
            }
        }

        if let Some(store_path) = actual_store_path {
            debug!(
                "Completed {action:?} operation with store path: {store_path:?}"
            );
        } else {
            debug!(
                "Completed {action:?} operation with local output path: {out_path:?}"
            );
        }

        Ok(())
    }

    /// Resolves and validates the closure to activate.
    ///
    /// Returns the resolved profile path and the path to the
    /// `switch-to-configuration` binary.
    fn resolve_closure(
        &self,
        out_path: &Path,
        target_profile: &Path,
        actual_store_path: Option<&Path>,
        ssh_config: &SshConfig,
    ) -> Result<(PathBuf, PathBuf)> {
        let is_remote_build = self.rebuild.target_host.is_some();

        // Validate system closure before activation, unless bypassed. For remote
        // builds, use the actual store path returned from the build. For local
        // builds, canonicalize the target_profile.
        let resolved_profile: PathBuf =
            if let Some(store_path) = actual_store_path {
                // Remote build - use the actual store path from the build output
                store_path.to_path_buf()
            } else if is_remote_build && !out_path.exists() {
                // Remote build with no local result and no store path captured
                // (shouldn't happen, but fallback)
                target_profile.to_path_buf()
            } else {
                // Local build - canonicalize the symlink to get the store path
                target_profile.canonicalize().context(
                    "Failed to resolve output path to actual store path",
                )?
            };

        if self.rebuild.no_validate {
            warn!(
                "Skipping pre-activation validation (--no-validate or NH_NO_VALIDATE \
         set)"
            );
            warn!(
                "This may result in activation failures if the system closure is \
         incomplete"
            );
        } else if let Some(target_host) = &self.rebuild.target_host {
            validate_system_closure_remote(
                &resolved_profile,
                target_host,
                self.rebuild.build_host.as_ref(),
                ssh_config,
            )?;
        } else {
            validate_system_closure(&resolved_profile)?;
        }

        let switch_to_configuration_path =
            resolved_profile.join("bin").join("switch-to-configuration");

        let switch_to_configuration =
            if is_remote_build && !out_path.exists() {
                // Remote build with no local result. Use uncanonicalized path
                // for SSH.
                switch_to_configuration_path
            } else {
                switch_to_configuration_path.canonicalize().context(
                    "Failed to resolve switch-to-configuration path",
                )?
            };

        Ok((resolved_profile, switch_to_configuration))
    }

    /// Runs the test phase. For remote switches this runs the full `switch`
    /// action instead of `test`.
    #[expect(clippy::too_many_arguments)]
    fn activate_test_phase(
        &self,
        resolved_profile: &Path,
        switch_to_configuration: &Path,
        action: ActivationAction,
        elevate: bool,
        elevation: &ElevationStrategy,
        subprocess_env: &SubprocessEnv,
        sudo_config: &SudoConfig,
        ssh_config: &SshConfig,
    ) -> Result<()> {
        if let Some(target_host) = &self.rebuild.target_host {
            let activation_type = match action {
                ActivationAction::Test => {
                    crate::remote::ActivationType::Test
                }
                ActivationAction::Switch => {
                    crate::remote::ActivationType::Switch
                }
                #[allow(
                    clippy::unreachable,
                    reason = "the boot action has no test phase"
                )]
                ActivationAction::Boot => {
                    unreachable!("the boot action has no test phase")
                }
            };

            crate::remote::activate_remote(
                target_host,
                resolved_profile,
                &crate::remote::ActivateRemoteConfig {
                    platform: crate::remote::Platform::NixOS,
                    activation_type,
                    install_bootloader: false,
                    show_logs: self.show_activation_logs,
                    elevation: elevate.then_some(elevation.clone()),
                },
                ssh_config,
                sudo_config,
            )
            .wrap_err(format!(
                "Activation ({}) failed",
                activation_type.as_str()
            ))?;
        } else {
            Command::new(
                switch_to_configuration,
                subprocess_env,
                sudo_config,
            )
            .arg("test")
            .message("Activating configuration")
            .elevate(elevate.then_some(elevation.clone()))
            .preserve_envs(["NIXOS_INSTALL_BOOTLOADER", "NIXOS_NO_CHECK"])
            .with_env()
            .show_output(self.show_activation_logs)
            .run()
            .wrap_err("Activation (test) failed")?;
        }

        Ok(())
    }

    /// Sets the system profile and installs the bootloader entry.
    #[expect(clippy::too_many_arguments)]
    fn activate_boot_phase(
        &self,
        out_path: &Path,
        resolved_profile: &Path,
        switch_to_configuration: &Path,
        elevate: bool,
        elevation: ElevationStrategy,
        subprocess_env: &SubprocessEnv,
        sudo_config: &SudoConfig,
        ssh_config: &SshConfig,
    ) -> Result<()> {
        if let Some(target_host) = &self.rebuild.target_host {
            crate::remote::activate_remote(
                target_host,
                resolved_profile,
                &crate::remote::ActivateRemoteConfig {
                    platform: crate::remote::Platform::NixOS,
                    activation_type: crate::remote::ActivationType::Boot,
                    install_bootloader: self.rebuild.install_bootloader,
                    show_logs: false,
                    elevation: elevate.then_some(elevation),
                },
                ssh_config,
                sudo_config,
            )
            .wrap_err("Bootloader activation failed")?;
        } else {
            // Use the base system closure instead of the specialisation one.
            // This is what makes all specialisations visible in the bootloader
            // instead of only the generation with the specialisation.
            let base_store_path = out_path.canonicalize().context(
                "Failed to resolve base output path to store path",
            )?;

            Command::new("nix", subprocess_env, sudo_config)
                .args(["build", "--no-link", "--profile", SYSTEM_PROFILE])
                .arg(&base_store_path)
                .elevate(elevate.then_some(elevation.clone()))
                .with_env()
                .run()
                .wrap_err("Failed to set system profile")?;

            let mut cmd = Command::new(
                switch_to_configuration,
                subprocess_env,
                sudo_config,
            )
            .arg("boot")
            .elevate(elevate.then_some(elevation))
            .message("Adding configuration to bootloader")
            .preserve_envs(["NIXOS_INSTALL_BOOTLOADER", "NIXOS_NO_CHECK"]);

            if self.rebuild.install_bootloader {
                cmd = cmd.set_env("NIXOS_INSTALL_BOOTLOADER", "1");
            }

            cmd.with_env()
                .run()
                .wrap_err("Bootloader activation failed")?;
        }

        Ok(())
    }
}

impl RebuildArgs {
    /// Performs initial setup and gathers context for an OS rebuild operation.
    ///
    /// This includes:
    /// - Ensuring SSH key login if a remote build/target host is involved.
    /// - Determining elevation status for local activation; the remote case is
    ///   handled by [`Self::determine_remote_elevation`] after the SSH
    ///   `ControlMaster` is up.
    /// - Performing updates to Nix inputs if specified.
    /// - Resolving the target hostname for the build.
    ///
    /// # Returns
    ///
    /// `Result` containing a tuple:
    ///
    /// - `bool`: `true` if local elevation is required. When `target_host` is set
    ///   this value is meaningless (returned as `false`), and the real answer is
    ///   produced by [`Self::determine_remote_elevation`] once the SSH
    ///   `ControlMaster` is up.
    /// - `String`: The resolved target hostname.
    fn setup_build_context(
        &self,
        elevation: &ElevationStrategy,
        _subprocess_env: &SubprocessEnv,
        _sudo_config: &SudoConfig,
    ) -> Result<(bool, String)> {
        // Only check SSH key login if remote hosts are involved
        if self.build_host.is_some() || self.target_host.is_some() {
            ensure_ssh_key_login()?;
        }

        // We still call this for the local-root guard it performs, even though
        // remote-target flows take their elevate answer from
        // `determine_remote_elevation` later.
        let local_elevate =
            has_elevation_status(self.bypass_root_check, elevation)?;
        let elevate = self.target_host.is_none() && local_elevate;

        let target_hostname = get_hostname(
            self.hostname
                .as_deref()
                .or_else(|| {
                    self.target_host.as_ref().map(RemoteHost::hostname)
                })
                .map(ToOwned::to_owned),
        )?;
        Ok((elevate, target_hostname))
    }

    /// Probe the remote uid to decide whether activation needs elevation.
    ///
    /// This must be called after [`crate::remote::open_ssh_control_master`]
    /// so the probe reuses the established connection.
    ///
    /// # Returns
    ///
    /// `false` when `target_host` is unset (caller should use
    /// [`Self::setup_build_context`] or [`has_elevation_status`] for the
    /// local case) or when the elevation strategy is [`None`].
    fn determine_remote_elevation(
        &self,
        elevation: &ElevationStrategy,
        ssh_config: &SshConfig,
    ) -> Result<bool> {
        let Some(target_host) = &self.target_host else {
            return Ok(false);
        };
        if matches!(elevation, ElevationStrategy::None) {
            return Ok(false);
        }
        let uid =
            crate::remote::probe_remote_uid(target_host, ssh_config)?;
        Ok(uid != 0)
    }

    fn determine_output_path(
        &self,
        temporary: bool,
    ) -> Result<(PathBuf, Option<tempfile::TempDir>)> {
        if let Some(p) = self.common.out_link.clone() {
            return Ok((p, None));
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
        flake_config: &FlakeConfig,
    ) -> Result<Installable> {
        let mut toplevel = self
            .common
            .installable
            .clone()
            .resolve_or_default(flake_config)?;
        let attrs = ["config", "system", "build", "toplevel"]
            .into_iter()
            .map(String::from);

        match &mut toplevel {
            Installable::Flake { attribute, .. } => {
                let first = attribute.first().cloned();
                let second = attribute.get(1).cloned();
                match (first.as_deref(), attribute.len()) {
                    (None, _) => {
                        attribute
                            .push(String::from("nixosConfigurations"));
                        attribute.push(target_hostname.to_owned());
                    }
                    (Some("nixosConfigurations"), 1) => {
                        info!(
                            "Inferring hostname '{target_hostname}' for \
                             nixosConfigurations"
                        );
                        attribute.push(target_hostname.to_owned());
                    }
                    (Some("nixosConfigurations"), 2) => {}
                    (Some("nixosConfigurations"), _) => {
                        bail!(
                            "Attribute path is too specific: {}. Please either:\n  1. Use the \
                             flake reference without attributes (e.g., '.')\n  2. Specify \
                             only the configuration name (e.g., '.#{}')",
                            attribute.join("."),
                            second.as_deref().unwrap_or_default()
                        );
                    }
                    _ => {
                        attribute.insert(
                            0,
                            String::from("nixosConfigurations"),
                        );
                    }
                }
                attribute.extend(attrs);
            }
            Installable::File { attribute, .. }
            | Installable::Expression { attribute, .. } => {
                attribute.extend(attrs);
            }
            Installable::Store { .. } => {}
        }

        if self.update_args.update_all
            || self.update_args.update_input.is_some()
        {
            update(
                &toplevel,
                self.update_args.update_input.clone(),
                self.common.passthrough.commit_lock_file,
            )?;
        }

        Ok(toplevel)
    }

    fn execute_build(
        &self,
        toplevel: Installable,
        out_path: &Path,
        ssh_config: &SshConfig,
    ) -> Result<Option<PathBuf>> {
        const MESSAGE: &str = "Building NixOS configuration";

        // If a build host is specified, use proper remote build semantics:
        //
        // 1. Evaluate derivation locally
        // 2. Copy derivation to build host (user-initiated SSH)
        // 3. Build on remote host
        // 4. Copy result back (to localhost or target_host)
        if let Some(build_host) = self.build_host.clone() {
            info!("{MESSAGE}");
            let config = RemoteBuildConfig {
                build_host,
                target_host: self.target_host.clone(),
                use_nom: !self.common.no_nom,
                use_substitutes: self.common.passthrough.use_substitutes,
                extra_args: self
                    .extra_args
                    .iter()
                    .map(Into::into)
                    .chain(
                        self.common
                            .passthrough
                            .generate_passthrough_args()
                            .into_iter()
                            .map(Into::into),
                    )
                    .collect(),
            };

            let actual_store_path = crate::remote::build_remote(
                &toplevel,
                &config,
                Some(out_path),
                ssh_config,
            )?;

            Ok(Some(actual_store_path))
        } else {
            // Local build - use the existing path
            command::Build::new(toplevel)
                .extra_arg("--out-link")
                .extra_arg(out_path)
                .extra_args(&self.extra_args)
                .passthrough(&self.common.passthrough)
                .message(MESSAGE)
                .nom(!self.common.no_nom)
                .run()
                .wrap_err("Failed to build configuration")?;

            Ok(None) // Local builds don't have separate store path
        }
    }

    fn build_and_diff(
        &self,
        toplevel: Installable,
        out_path: &Path,
        ssh_config: &SshConfig,
    ) -> Result<BuiltConfiguration> {
        let actual_store_path =
            self.execute_build(toplevel, out_path, ssh_config)?;
        let target_profile =
            self.resolve_specialisation_and_profile(out_path)?;

        handle_nixos_diff(
            &self.common.diff,
            self.target_host.as_ref(),
            &target_profile,
            actual_store_path.as_deref(),
            out_path,
            ssh_config,
        )?;

        Ok(BuiltConfiguration {
            target_profile,
            actual_store_path,
        })
    }

    fn resolve_specialisation_and_profile(
        &self,
        out_path: &Path,
    ) -> Result<PathBuf> {
        let current_specialisation =
            std::fs::read_to_string(SPEC_LOCATION)
                .ok()
                .map(|s| s.trim().to_owned());

        let target_specialisation = if self.no_specialisation {
            None
        } else {
            self.specialisation.clone().or(current_specialisation)
        };

        debug!("Target specialisation: {target_specialisation:?}");

        // Determine target profile, falling back to base if specialisation doesn't
        // exist
        let target_profile = match &target_specialisation {
            None => out_path.to_path_buf(),
            Some(spec) => {
                let spec_path = out_path.join("specialisation").join(spec);

                // For local builds, check if specialisation exists and fall back if not
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

        // Validate the final target profile exists if it's a local build
        if out_path.exists() && !target_profile.exists() {
            return Err(eyre!(
                "Target profile path does not exist: {}",
                target_profile.display()
            ));
        }

        Ok(target_profile)
    }

    /// Builds the toplevel configuration without activating (`nh build`).
    #[expect(clippy::missing_errors_doc)]
    pub fn build_only(
        &self,
        elevation: &ElevationStrategy,
        subprocess_env: &SubprocessEnv,
        sudo_config: &SudoConfig,
        flake_config: &FlakeConfig,
        ssh_config: &SshConfig,
    ) -> Result<()> {
        let (_, target_hostname) = self.setup_build_context(
            elevation,
            subprocess_env,
            sudo_config,
        )?;

        let (out_path, _tempdir_guard) =
            self.determine_output_path(false)?;

        let toplevel =
            self.prepare_toplevel(&target_hostname, flake_config)?;
        self.build_and_diff(toplevel, &out_path, ssh_config)?;

        Ok(())
    }
}

impl RollbackArgs {
    #[expect(clippy::too_many_lines, clippy::missing_errors_doc)]
    pub fn rollback(
        &self,
        elevation: ElevationStrategy,
        subprocess_env: &SubprocessEnv,
        sudo_config: &SudoConfig,
        _flake_config: &FlakeConfig,
    ) -> Result<()> {
        let elevate =
            has_elevation_status(self.bypass_root_check, &elevation)?;

        let generations = list_generations()?;

        let current_generation = generations
            .iter()
            .find(|g| g.current)
            .ok_or_else(|| eyre!("Current generation not found"))?;

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
        let current_specialisation = fs::read_to_string(SPEC_LOCATION)
            .ok()
            .map(|s| s.trim().to_owned());

        let target_specialisation = if self.no_specialisation {
            None
        } else {
            self.specialisation.clone().or(current_specialisation)
        };

        debug!("target_specialisation: {target_specialisation:?}");

        // Compare changes between current and target generation
        if matches!(self.diff, DiffType::Never) {
            debug!(
                "Not running dix as the target hostname is different from the system \
         hostname."
            );
        } else {
            debug!(
                "Comparing with target profile: {}",
                generation_link.display()
            );
            let _ = print_dix_diff(
                &PathBuf::from(CURRENT_PROFILE),
                &generation_link,
            );
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
        Command::new("ln", subprocess_env, sudo_config)
            .arg("-sfn") // force, symbolic link
            .arg(&generation_link)
            .arg(SYSTEM_PROFILE)
            .elevate(elevate.then_some(elevation.clone()))
            .message("Setting system profile")
            .with_env()
            .run()
            .wrap_err("Failed to set system profile during rollback")?;

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
            subprocess_env,
            sudo_config,
        )
        .arg("switch")
        .elevate(elevate.then_some(elevation.clone()))
        .preserve_envs(["NIXOS_INSTALL_BOOTLOADER", "NIXOS_NO_CHECK"])
        .with_env()
        .run()
        {
            Ok(()) => {
                info!(
                    "Successfully rolled back to generation {}",
                    target_generation.number
                );
            }
            Err(e) => {
                // If activation fails, rollback the profile
                if current_generation.number > 0 {
                    let current_gen_link = profile_dir.join(format!(
                        "system-{}-link",
                        current_generation.number
                    ));

                    Command::new("ln", subprocess_env, sudo_config)
                        .arg("-sfn") // Force, symbolic link
                        .arg(&current_gen_link)
                        .arg(SYSTEM_PROFILE)
                        .elevate(elevate.then_some(elevation))
                        .message("Rolling back system profile")
                        .with_env()
                        .run()
                        .wrap_err("NixOS: Failed to restore previous system profile after failed activation")?;
                }

                return Err(eyre!("Activation (switch) failed: {}", e))
                    .context("Failed to activate configuration");
            }
        }

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
        return Err(eyre!(
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

/// Validates essential files on a remote host via SSH.
///
/// Similar to [`validate_system_closure`] but executes checks on a remote host.
fn validate_system_closure_remote(
    system_path: &Path,
    target_host: &RemoteHost,
    build_host: Option<&RemoteHost>,
    ssh_config: &SshConfig,
) -> Result<()> {
    // Build context string for error messages
    let context = build_host.map(|build| {
        if build.hostname() == target_host.hostname() {
            "also build host".to_string()
        } else {
            format!("built on '{build}'")
        }
    });

    // Delegate to the generic remote validation function
    crate::remote::validate_closure_remote(
        target_host,
        system_path,
        ESSENTIAL_FILES,
        context.as_deref(),
        ssh_config,
    )
}

/// Returns an error indicating that the 'switch-to-configuration' binary is
/// missing, along with common reasons and solutions.
fn missing_switch_to_configuration_error() -> color_eyre::eyre::Report {
    eyre!(
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
/// false). Returns `false` if elevation is not required (root or
/// `bypass_root_check` is true).
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
    elevation: &command::ElevationStrategy,
) -> Result<bool> {
    // If elevation strategy is None, never elevate
    if matches!(elevation, command::ElevationStrategy::None) {
        return Ok(false);
    }

    let is_root = nix::unistd::Uid::effective().is_root();

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

fn find_previous_generation(
    current_number: u64,
    generations: &[generations::GenerationInfo],
) -> Result<generations::GenerationInfo> {
    let current = generations
        .iter()
        .find(|g| g.number == current_number)
        .ok_or_else(|| eyre!("Current generation not found"))?;

    generations
        .iter()
        .rev()
        .find(|g| g.number < current.number)
        .cloned()
        .ok_or_else(|| {
            eyre!("No generation older than the current one exists")
        })
}

fn get_generation_by_number(
    number: u64,
    generations: &[generations::GenerationInfo],
) -> Result<&generations::GenerationInfo> {
    generations
        .iter()
        .find(|g| g.number == number)
        .ok_or_else(|| eyre!("Generation {} not found", number))
}

fn list_generations() -> Result<Vec<generations::GenerationInfo>> {
    let profile_path = PathBuf::from(SYSTEM_PROFILE);
    let profiles_dir = profile_path
        .parent()
        .unwrap_or_else(|| Path::new("/nix/var/nix/profiles"));

    let mut generations = Vec::new();
    for entry in fs::read_dir(profiles_dir)? {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                warn!("Failed to read entry in profile directory: {}", e);
                continue;
            }
        };

        let path = entry.path();
        if let Some(name) = path.file_name().and_then(|s| s.to_str())
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

    generations.sort_by_key(|g| g.number);

    Ok(generations)
}

impl ReplArgs {
    #[expect(clippy::missing_errors_doc)]
    pub fn run(
        self,
        subprocess_env: &SubprocessEnv,
        _sudo_config: &SudoConfig,
        flake_config: &FlakeConfig,
    ) -> Result<()> {
        let mut target_installable =
            self.installable.resolve_or_default(flake_config)?;

        if matches!(target_installable, Installable::Store { .. }) {
            bail!("Nix doesn't support nix store installables.");
        }

        let hostname = get_hostname(self.hostname)?;

        if let Installable::Flake {
            ref mut attribute, ..
        } = target_installable
            && attribute.is_empty()
        {
            attribute.push(String::from("nixosConfigurations"));
            attribute.push(hostname);
        }

        let status = NixCommand::new(CommandKind::Repl)
            .args(target_installable.to_args())
            .envs(subprocess_env.vars())
            .run_with_logs()?;
        if !status.success() {
            bail!("nix repl failed (exit status {status:?})");
        }

        Ok(())
    }
}

impl GenerationsArgs {
    #[expect(clippy::missing_errors_doc)]
    pub fn info(
        &self,
        _subprocess_env: &SubprocessEnv,
        _sudo_config: &SudoConfig,
    ) -> Result<()> {
        let profile = match self.profile {
            Some(ref p) => PathBuf::from(p),
            None => bail!("Profile path is required"),
        };

        if !profile.is_symlink() {
            return Err(eyre!(
                "No profile `{:?}` found",
                profile.file_name().unwrap_or_default()
            ));
        }

        let profile_dir =
            profile.parent().unwrap_or_else(|| Path::new("."));

        let generations: Vec<_> = fs::read_dir(profile_dir)?
            .filter_map(|entry| {
                entry.ok().and_then(|e| {
                    let path = e.path();
                    if path
                        .file_name()?
                        .to_str()?
                        .starts_with(profile.file_name()?.to_str()?)
                    {
                        Some(path)
                    } else {
                        None
                    }
                })
            })
            .collect();

        let gen_dir_refs: Vec<&std::path::Path> =
            generations.iter().map(PathBuf::as_path).collect();
        let closure_sizes =
            generations::get_closure_sizes_batch(&gen_dir_refs);

        let descriptions: Vec<generations::GenerationInfo> = generations
            .iter()
            .filter_map(|gen_dir| {
                let size = closure_sizes.get(gen_dir).cloned();
                generations::describe(gen_dir, size)
            })
            .collect();

        generations::print_info(descriptions, self.fields.as_deref())?;

        Ok(())
    }
}
