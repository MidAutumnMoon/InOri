//! Building and activating a NixOS configuration.
//!
//! One flow serves four commands: `build` stops after building; `switch`,
//! `boot`, and `test` continue with the corresponding activation. The
//! command line produces a [`RebuildCommand`], and [`run`] executes it.

use std::path::Path;
use std::path::PathBuf;

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
use super::has_elevation_status;
use super::resolve_specialisation;
use super::specialisation_in;
use super::update;
use crate::command::Command;
use crate::elevation::Elevation;
use crate::nix::build::Build;
use crate::nix::options::NixBuildOptions;
use crate::runtime::Config;
use crate::runtime::Env;
use crate::target::{self, BuildTarget};

/// Which rebuild flow to run: build only, or build followed by activation.
#[derive(Clone, Debug)]
#[expect(
    clippy::module_name_repetitions,
    reason = "rebuild::Command would read as the generic runner Command"
)]
pub enum RebuildCommand {
    Build(Request),
    Activate(ActivationRequest),
}

#[derive(Clone, Debug)]
pub struct Request {
    pub build: BuildOptions,
    pub update: Option<update::Selection>,
    pub hostname: Option<String>,
    pub specialisation: SpecialisationSelection,
    pub extra_args: Vec<String>,
    pub bypass_root_check: bool,
    pub commit_lock_file: bool,
}

#[derive(Clone, Debug)]
pub struct BuildOptions {
    pub target: Option<BuildTarget>,
    pub no_nom: bool,
    pub out_link: Option<PathBuf>,
    pub diff: DiffMode,
    pub nix: NixBuildOptions,
}

#[derive(Clone, Debug)]
pub struct ActivationRequest {
    pub rebuild: Request,
    pub activation: Activation,
}

#[derive(Clone, Debug)]
pub struct Activation {
    pub action: ActivationAction,
    pub dry: bool,
    pub ask: bool,
    pub no_validate: bool,
}

#[derive(Clone, Copy, Debug)]
pub enum ActivationAction {
    Test {
        show_logs: bool,
    },
    Boot {
        install_bootloader: bool,
    },
    Switch {
        show_logs: bool,
        install_bootloader: bool,
    },
}

impl ActivationAction {
    #[must_use]
    pub const fn show_logs(self) -> bool {
        match self {
            Self::Test { show_logs } | Self::Switch { show_logs, .. } => {
                show_logs
            }
            Self::Boot { .. } => false,
        }
    }

    #[must_use]
    pub const fn install_bootloader(self) -> bool {
        match self {
            Self::Boot { install_bootloader }
            | Self::Switch {
                install_bootloader, ..
            } => install_bootloader,
            Self::Test { .. } => false,
        }
    }
}

/// Execute a rebuild command.
///
/// # Errors
///
/// Returns an error if building or activating the configuration fails.
pub fn run(command: RebuildCommand, config: &Config) -> Result<()> {
    match command {
        RebuildCommand::Build(request) => request.build_only(config),
        RebuildCommand::Activate(request) => {
            request.build_and_activate(config)
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

/// A successfully built system closure.
///
/// The build leaves an out-link pointing at the toplevel; the selected
/// profile is either the toplevel itself or one of its specialisations.
/// Both paths are canonical store paths, so diffing, validation, and
/// activation never have to re-resolve them.
#[derive(Debug)]
struct BuiltSystem {
    /// Store path of the built toplevel (the out-link's target).
    toplevel: PathBuf,

    /// Store path of the selected profile: the toplevel, or a
    /// specialisation of it.
    profile: PathBuf,
}

impl BuiltSystem {
    /// Resolve the built system from the out-link left by the build.
    ///
    /// # Errors
    ///
    /// Returns an error if the out-link cannot be resolved to a store path,
    /// or a selected specialisation does not exist in the built
    /// configuration.
    fn resolve(
        out_link: &Path,
        specialisation: Option<&str>,
    ) -> Result<Self> {
        let toplevel = out_link.canonicalize().context(
            "Failed to resolve output path to actual store path",
        )?;

        let profile = match specialisation {
            None => toplevel.clone(),
            Some(spec) => {
                let spec_link = specialisation_in(out_link, spec)
                    .ok_or_else(|| {
                        report!(
                            "Specialisation '{spec}' does not exist in the built \
                             configuration"
                        )
                    })?;
                spec_link.canonicalize().context_with(|| {
                    format!(
                        "Failed to resolve specialisation '{spec}' to a store path"
                    )
                })?
            }
        };

        Ok(Self { toplevel, profile })
    }
}

impl Request {
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
        let elevate =
            has_elevation_status(self.bypass_root_check, elevation)?;

        let target_hostname = self
            .hostname
            .clone()
            .unwrap_or_else(|| env.hostname().to_owned());

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

        let mut toplevel =
            target::resolve(self.build.target.clone(), env)?;

        match &mut toplevel {
            BuildTarget::Flake { attribute, .. } => {
                let second = attribute
                    .get(1)
                    .map_or_else(String::new, String::clone);
                match attribute.first().map(String::as_str) {
                    None => {
                        attribute
                            .push(String::from("nixosConfigurations"));
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
            update::run(&toplevel, selection, self.commit_lock_file)?;
        }

        Ok(toplevel)
    }

    fn execute_build(
        &self,
        toplevel: BuildTarget,
        out_path: &Path,
    ) -> Result<()> {
        const MESSAGE: &str = "Building NixOS configuration";

        Build::new(toplevel)
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
    ) -> Result<BuiltSystem> {
        self.execute_build(toplevel, out_path)?;

        let target_specialisation =
            resolve_specialisation(&self.specialisation);
        debug!("Target specialisation: {target_specialisation:?}");

        let built = BuiltSystem::resolve(
            out_path,
            target_specialisation.as_deref(),
        )?;
        debug!("Built toplevel: {}", built.toplevel.display());
        debug!("Selected profile: {}", built.profile.display());

        super::diff::run(&self.build.diff, &built.profile)?;

        Ok(built)
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

impl ActivationRequest {
    fn build_and_activate(&self, config: &Config) -> Result<()> {
        let (elevate, target_hostname) = self
            .rebuild
            .setup_build_context(&config.elevation, &config.env)?;

        let (out_path, _tempdir_guard) =
            self.rebuild.determine_output_path(true)?;

        let toplevel = self
            .rebuild
            .prepare_toplevel(&target_hostname, &config.env)?;

        let built = self.rebuild.build_and_diff(toplevel, &out_path)?;

        if self.activation.dry {
            if self.activation.ask {
                warn!("--ask has no effect as dry run was requested");
            }

            return Ok(());
        }

        self.activate_rebuilt_config(&built, elevate, config)?;

        Ok(())
    }

    fn activate_rebuilt_config(
        &self,
        built: &BuiltSystem,
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
            self.resolve_closure(&built.profile)?;

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
                    built,
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
                    built,
                    &switch_to_configuration,
                    elevate,
                    config,
                )?;
            }
        }

        debug!(
            "Completed {action:?} operation with store path: {}",
            built.profile.display()
        );

        Ok(())
    }

    /// Validates the closure to activate and resolves the path to its
    /// `switch-to-configuration` binary.
    fn resolve_closure(&self, profile: &Path) -> Result<PathBuf> {
        if self.activation.no_validate {
            warn!(
                "Skipping pre-activation validation (--no-validate set)"
            );
            warn!(
                "This may result in activation failures if the system closure is \
         incomplete"
            );
        } else {
            validate_system_closure(profile)?;
        }

        let switch_to_configuration = profile
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
        built: &BuiltSystem,
        switch_to_configuration: &Path,
        elevate: bool,
        config: &Config,
    ) -> Result<()> {
        // Use the base system closure instead of the specialisation one.
        // This is what makes all specialisations visible in the bootloader
        // instead of only the generation with the specialisation.
        Command::new("nix", &config.env, &config.elevation)
            .args(["build", "--no-link", "--profile", SYSTEM_PROFILE])
            .arg(&built.toplevel)
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

        cmd.run().context("Bootloader activation failed")?;

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

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "Test assertions")]
mod tests {
    use super::*;
    use std::fs;

    /// Lay out a fake built system: an out-link to a toplevel containing one
    /// specialisation, both as store-path-shaped directories.
    fn fake_built_system(dir: &Path) -> PathBuf {
        let toplevel = dir.join("store-toplevel");
        let spec = dir.join("store-spec");
        fs::create_dir_all(toplevel.join("specialisation")).unwrap();
        fs::create_dir_all(&spec).unwrap();
        std::os::unix::fs::symlink(
            &spec,
            toplevel.join("specialisation/foo"),
        )
        .unwrap();
        std::os::unix::fs::symlink(&toplevel, dir.join("result")).unwrap();
        dir.join("result")
    }

    #[test]
    fn resolve_without_specialisation_selects_the_toplevel() {
        let dir = tempfile::tempdir().unwrap();
        let out_link = fake_built_system(dir.path());
        let base = dir.path().canonicalize().unwrap();

        let built = BuiltSystem::resolve(&out_link, None).unwrap();

        assert_eq!(built.toplevel, base.join("store-toplevel"));
        assert_eq!(built.profile, built.toplevel);
    }

    #[test]
    fn resolve_resolves_specialisation_to_its_own_store_path() {
        let dir = tempfile::tempdir().unwrap();
        let out_link = fake_built_system(dir.path());
        let base = dir.path().canonicalize().unwrap();

        let built = BuiltSystem::resolve(&out_link, Some("foo")).unwrap();

        assert_eq!(built.toplevel, base.join("store-toplevel"));
        assert_eq!(built.profile, base.join("store-spec"));
    }

    #[test]
    fn resolve_rejects_missing_specialisation() {
        let dir = tempfile::tempdir().unwrap();
        let out_link = fake_built_system(dir.path());

        let error =
            BuiltSystem::resolve(&out_link, Some("missing")).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("does not exist in the built configuration")
        );
    }
}
