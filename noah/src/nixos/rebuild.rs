use std::convert::Into;
use std::ops::Deref;
use std::path::{Path, PathBuf};

use nh_installable::{FlakeConfig, Installable};
use rootcause::{Result, bail, prelude::ResultExt as _, report};
use tracing::{debug, info, warn};

use super::request::{
    Activation, ActivationAction,
    ActivationRequest as ParsedActivationRequest, RebuildCommand,
    RebuildRequest,
};
use super::{
    SYSTEM_PROFILE, has_elevation_status, resolve_specialisation,
};
use crate::app::RuntimeConfig;
use crate::command::{self, Command, Elevation};
use crate::diff::handle_nixos;
use crate::remote::copy::copy_to_remote;
use crate::remote::{
    ActivateRemoteConfig, ActivationType, BuildConfig, Host, Platform,
    SshConfig, activate_remote, build_remote, init_ssh_control,
    open_ssh_control_master, probe_remote_uid, validate_remote_closure,
};
use crate::update::update;
use crate::util::{ensure_ssh_key_login, get_hostname};
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

pub(super) fn run(
    command: RebuildCommand,
    env: &RuntimeConfig,
) -> Result<()> {
    match command {
        RebuildCommand::Build(request) => Rebuild(request).build_only(env),
        RebuildCommand::Activate(request) => {
            ActivationRequest::from(request).build_and_activate(env)
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
struct BuiltConfiguration {
    target_profile: PathBuf,
    actual_store_path: Option<PathBuf>,
}
impl ActivationRequest {
    fn build_and_activate(&self, env: &RuntimeConfig) -> Result<()> {
        let (local_elevate, target_hostname) =
            self.rebuild
                .setup_build_context(&env.elevation, &env.ssh)?;

        let (out_path, _tempdir_guard) =
            self.rebuild.determine_output_path(true)?;

        let toplevel =
            self.rebuild
                .prepare_toplevel(&target_hostname, &env.flake)?;

        // Initialize SSH control early if we have remote hosts - guard will keep
        // connections alive for both build and activation
        let _ssh_guard = if self.rebuild.build_host.is_some()
            || self.rebuild.target_host.is_some()
        {
            let guard = init_ssh_control(&env.ssh)?;

            // Pre-establish ControlMaster connections so that delegated SSH
            // invocations (e.g. `nix copy --to ssh://...`) reuse the already-
            // authenticated socket rather than opening a fresh connection where
            // SSH option ordering may differ.
            if let Some(build_host) = &self.rebuild.build_host {
                open_ssh_control_master(build_host, &env.ssh).context(
                    "Failed to establish SSH connection to build host",
                )?;
            }

            if let Some(target_host) = &self.rebuild.target_host {
                open_ssh_control_master(target_host, &env.ssh).context(
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
                .determine_remote_elevation(&env.elevation, &env.ssh)?
        } else {
            local_elevate
        };

        let built =
            self.rebuild
                .build_and_diff(toplevel, &out_path, &env.ssh)?;

        if self.activation.dry {
            if self.activation.ask {
                warn!("--ask has no effect as dry run was requested");
            }

            return Ok(());
        }

        self.activate_rebuilt_config(
            &out_path,
            &built.target_profile,
            built.actual_store_path.as_deref(),
            elevate,
            env,
        )?;

        Ok(())
    }

    fn activate_rebuilt_config(
        &self,
        out_path: &Path,
        target_profile: &Path,
        actual_store_path: Option<&Path>,
        elevate: bool,
        env: &RuntimeConfig,
    ) -> Result<()> {
        let action = self.activation.action;
        let ssh_config = &env.ssh;
        if self.activation.ask {
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
            copy_to_remote(
                target_host,
                target_profile,
                self.rebuild.use_substitutes,
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
            ActivationAction::Test { .. } => {
                self.activate_test_phase(
                    &resolved_profile,
                    &switch_to_configuration,
                    elevate,
                    env,
                )?;
            }
            ActivationAction::Boot { .. } => {
                self.activate_boot_phase(
                    out_path,
                    &resolved_profile,
                    &switch_to_configuration,
                    elevate,
                    env,
                )?;
            }
            ActivationAction::Switch { .. } => {
                self.activate_test_phase(
                    &resolved_profile,
                    &switch_to_configuration,
                    elevate,
                    env,
                )?;
                self.activate_boot_phase(
                    out_path,
                    &resolved_profile,
                    &switch_to_configuration,
                    elevate,
                    env,
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

        if self.activation.no_validate {
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
    fn activate_test_phase(
        &self,
        resolved_profile: &Path,
        switch_to_configuration: &Path,
        elevate: bool,
        env: &RuntimeConfig,
    ) -> Result<()> {
        let action = self.activation.action;
        if let Some(target_host) = &self.rebuild.target_host {
            let activation_type = match action {
                ActivationAction::Test { .. } => ActivationType::Test,
                ActivationAction::Switch { .. } => ActivationType::Switch,
                #[expect(
                    clippy::unreachable,
                    reason = "the boot action has no test phase"
                )]
                ActivationAction::Boot { .. } => {
                    unreachable!("the boot action has no test phase")
                }
            };

            activate_remote(
                target_host,
                resolved_profile,
                &ActivateRemoteConfig {
                    platform: Platform::NixOS,
                    activation_type,
                    install_bootloader: false,
                    show_logs: self.activation.action.show_logs(),
                    elevation: elevate.then_some(&env.elevation),
                },
                &env.process,
                &env.ssh,
            )
            .context(format!(
                "Activation ({}) failed",
                activation_type.as_str()
            ))?;
        } else {
            Command::new(
                switch_to_configuration,
                &env.process,
                &env.elevation,
            )
            .arg("test")
            .message("Activating configuration")
            .elevate(elevate)
            .preserve_envs(["NIXOS_INSTALL_BOOTLOADER", "NIXOS_NO_CHECK"])
            .show_output(self.activation.action.show_logs())
            .run()
            .context("Activation (test) failed")?;
        }

        Ok(())
    }

    /// Sets the system profile and installs the bootloader entry.
    fn activate_boot_phase(
        &self,
        out_path: &Path,
        resolved_profile: &Path,
        switch_to_configuration: &Path,
        elevate: bool,
        env: &RuntimeConfig,
    ) -> Result<()> {
        if let Some(target_host) = &self.rebuild.target_host {
            activate_remote(
                target_host,
                resolved_profile,
                &ActivateRemoteConfig {
                    platform: Platform::NixOS,
                    activation_type: ActivationType::Boot,
                    install_bootloader: self
                        .activation
                        .action
                        .install_bootloader(),
                    show_logs: false,
                    elevation: elevate.then_some(&env.elevation),
                },
                &env.process,
                &env.ssh,
            )
            .context("Bootloader activation failed")?;
        } else {
            // Use the base system closure instead of the specialisation one.
            // This is what makes all specialisations visible in the bootloader
            // instead of only the generation with the specialisation.
            let base_store_path = out_path.canonicalize().context(
                "Failed to resolve base output path to store path",
            )?;

            Command::new("nix", &env.process, &env.elevation)
                .args(["build", "--no-link", "--profile", SYSTEM_PROFILE])
                .arg(&base_store_path)
                .elevate(elevate)
                .run()
                .context("Failed to set system profile")?;

            let mut cmd = Command::new(
                switch_to_configuration,
                &env.process,
                &env.elevation,
            )
            .arg("boot")
            .elevate(elevate)
            .message("Adding configuration to bootloader")
            .preserve_envs(["NIXOS_INSTALL_BOOTLOADER", "NIXOS_NO_CHECK"]);

            if self.activation.action.install_bootloader() {
                cmd = cmd.set_env("NIXOS_INSTALL_BOOTLOADER", "1");
            }

            cmd.run().context("Bootloader activation failed")?;
        }

        Ok(())
    }
}
impl Rebuild {
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
        elevation: &Elevation,
        ssh_config: &SshConfig,
    ) -> Result<(bool, String)> {
        // Only check SSH key login if remote hosts are involved
        if self.build_host.is_some() || self.target_host.is_some() {
            ensure_ssh_key_login(ssh_config)?;
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
                .or_else(|| self.target_host.as_ref().map(Host::hostname))
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
    /// local case) or when elevation is disabled via
    /// `--elevation-strategy=none`.
    fn determine_remote_elevation(
        &self,
        elevation: &Elevation,
        ssh_config: &SshConfig,
    ) -> Result<bool> {
        let Some(target_host) = self.target_host.as_ref() else {
            return Ok(false);
        };
        if elevation.is_disabled() {
            return Ok(false);
        }
        let uid = probe_remote_uid(target_host, ssh_config)?;
        Ok(uid != 0)
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
        flake_config: &FlakeConfig,
    ) -> Result<Installable> {
        let mut toplevel = self
            .build
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

        if let Some(selection) = &self.update {
            update(&toplevel, selection, self.commit_lock_file)?;
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
            let mut extra_args =
                self.extra_args.iter().map(Into::into).collect();
            self.build.nix.append_args(&mut extra_args);
            let config = BuildConfig {
                build_host,
                target_host: self.target_host.clone(),
                use_nom: !self.build.no_nom,
                use_substitutes: self.use_substitutes,
                extra_args,
            };

            let actual_store_path = build_remote(
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
                .nix_options(&self.build.nix)
                .message(MESSAGE)
                .nom(!self.build.no_nom)
                .run()
                .context("Failed to build configuration")?;

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

        handle_nixos(
            &self.build.diff,
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
        let target_specialisation =
            resolve_specialisation(&self.specialisation);

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
            return Err(report!(
                "Target profile path does not exist: {}",
                target_profile.display()
            ));
        }

        Ok(target_profile)
    }

    /// Builds the toplevel configuration without activating (`nh build`).
    fn build_only(&self, env: &RuntimeConfig) -> Result<()> {
        let (_, target_hostname) =
            self.setup_build_context(&env.elevation, &env.ssh)?;

        let (out_path, _tempdir_guard) =
            self.determine_output_path(false)?;

        let toplevel =
            self.prepare_toplevel(&target_hostname, &env.flake)?;
        self.build_and_diff(toplevel, &out_path, &env.ssh)?;

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

/// Validates essential files on a remote host via SSH.
///
/// Similar to [`validate_system_closure`] but executes checks on a remote host.
fn validate_system_closure_remote(
    system_path: &Path,
    target_host: &Host,
    build_host: Option<&Host>,
    ssh_config: &SshConfig,
) -> Result<()> {
    // Build context string for error messages
    let context = build_host.map(|build| {
        if build.hostname() == target_host.hostname() {
            "also build host".to_owned()
        } else {
            format!("built on '{build}'")
        }
    });

    // Delegate to the generic remote validation function
    validate_remote_closure(
        target_host,
        system_path,
        ESSENTIAL_FILES,
        context.as_deref(),
        ssh_config,
    )
}
