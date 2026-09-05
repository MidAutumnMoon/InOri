//! Building and activating a NixOS configuration.
//!
//! One flow serves four commands: `build` stops after building; `switch`,
//! `boot`, and `test` continue with the corresponding activation. The
//! command line produces a [`RebuildCommand`], and [`run`] executes it: it
//! applies the family-wide root guard, then builds the configuration and,
//! for activation commands, activates it.

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
use super::requires_elevation;
use super::resolve_specialisation;
use super::specialisation_in;
use super::update;
use crate::command::Command;
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

impl RebuildCommand {
    /// Whether the request opted out of the root guard.
    fn bypass_root_check(&self) -> bool {
        match self {
            Self::Build(request) => request.bypass_root_check,
            Self::Activate(request) => request.rebuild.bypass_root_check,
        }
    }
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
/// The root guard is family-wide: even a plain `build` must not run as
/// root. The elevation decision is made once here; activation flows carry
/// it down to the commands that actually escalate.
///
/// # Errors
///
/// Returns an error if the root check fails, or if building or activating
/// the configuration fails.
pub fn run(command: RebuildCommand, config: &Config) -> Result<()> {
    let elevate =
        requires_elevation(command.bypass_root_check(), &config.elevation)?;

    match command {
        RebuildCommand::Build(request) => request.build_only(config),
        RebuildCommand::Activate(request) => {
            request.build_and_activate(config, elevate)
        }
    }
}

/// The out-link a build leaves behind, plus the temporary directory backing
/// it when one was allocated.
///
/// A temporary out-link is only meaningful while its directory exists, so
/// the directory lives in this value: keeping the [`OutLink`] alive keeps
/// the link resolvable.
#[derive(Debug)]
struct OutLink {
    path: PathBuf,
    /// Never read; held so the directory outlives every use of `path`.
    _temp_dir: Option<tempfile::TempDir>,
}

impl OutLink {
    /// The out-link for a plain `build`: `--out-link`, defaulting to
    /// `result` in the working directory so the user can collect the built
    /// system.
    fn for_build(explicit: Option<&Path>) -> Self {
        Self::at(explicit.unwrap_or_else(|| Path::new("result")))
    }

    /// The out-link for an activation flow: `--out-link`, else a throwaway
    /// temporary directory, since only the resolved store paths are used
    /// after the build.
    fn for_activation(explicit: Option<&Path>) -> Result<Self> {
        if let Some(path) = explicit {
            return Ok(Self::at(path));
        }

        let dir = tempfile::Builder::new().prefix("nh-os").tempdir()?;
        Ok(Self {
            path: dir.path().join("result"),
            _temp_dir: Some(dir),
        })
    }

    fn at(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            _temp_dir: None,
        }
    }
}

/// A successfully built system closure.
///
/// The build leaves an out-link pointing at the toplevel; the selected
/// closure is either the toplevel itself or one of its specialisations.
/// Both paths are canonical store paths, so diffing, validation, and
/// activation never have to re-resolve them.
#[derive(Debug)]
struct BuiltSystem {
    /// Store path of the built toplevel (the out-link's target).
    toplevel: PathBuf,

    /// Store path of the selected closure: the toplevel, or a
    /// specialisation of it.
    selected: PathBuf,
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

        let selected = match specialisation {
            None => toplevel.clone(),
            Some(spec) => {
                let spec_link = specialisation_in(out_link, spec)
                    .ok_or_else(|| {
                        report!(
                            "Specialisation '{spec}' does not exist in the \
                             built configuration"
                        )
                    })?;
                spec_link.canonicalize().context_with(|| {
                    format!(
                        "Failed to resolve specialisation '{spec}' to a store \
                         path"
                    )
                })?
            }
        };

        Ok(Self { toplevel, selected })
    }
}

impl Request {
    /// Builds the toplevel configuration without activating (`nh build`).
    fn build_only(&self, config: &Config) -> Result<()> {
        let out_link = OutLink::for_build(self.build.out_link.as_deref());

        let toplevel = self.prepare_toplevel(&config.env)?;
        self.build_and_diff(toplevel, &out_link)?;

        Ok(())
    }

    /// Resolves what to build and completes it to select the NixOS
    /// `toplevel` derivation, optionally refreshing inputs first.
    fn prepare_toplevel(&self, env: &Env) -> Result<BuildTarget> {
        let mut target = target::resolve(self.build.target.clone(), env)?;
        let hostname = self
            .hostname
            .as_deref()
            .unwrap_or_else(|| env.hostname());
        select_toplevel(&mut target, hostname)?;

        if let Some(selection) = &self.update {
            update::run(&target, selection, self.commit_lock_file)?;
        }

        Ok(target)
    }

    /// Builds the toplevel, resolves the built system from the out-link and
    /// shows the configured diff.
    fn build_and_diff(
        &self,
        toplevel: BuildTarget,
        out_link: &OutLink,
    ) -> Result<BuiltSystem> {
        self.execute_build(toplevel, &out_link.path)?;

        let target_specialisation =
            resolve_specialisation(&self.specialisation);
        debug!("Target specialisation: {target_specialisation:?}");

        let built = BuiltSystem::resolve(
            &out_link.path,
            target_specialisation.as_deref(),
        )?;
        debug!("Built toplevel: {}", built.toplevel.display());
        debug!("Selected closure: {}", built.selected.display());

        super::diff::run(&self.build.diff, &built.selected)?;

        Ok(built)
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
}

/// Completes a resolved target so the build produces the NixOS `toplevel`
/// derivation.
///
/// Flake targets get their attribute path completed to
/// `nixosConfigurations.<hostname>.config.system.build.toplevel`: a bare
/// reference or a bare `nixosConfigurations` infers the hostname, a bare
/// configuration name gets `nixosConfigurations` prepended, and anything
/// deeper is rejected as too specific. File and expression targets only
/// gain the `config.system.build.toplevel` suffix; a store path already is
/// the built output.
///
/// # Errors
///
/// Returns an error for a flake attribute path too specific to infer a
/// configuration from.
fn select_toplevel(target: &mut BuildTarget, hostname: &str) -> Result<()> {
    const TOPLEVEL_ATTRS: [&str; 4] =
        ["config", "system", "build", "toplevel"];

    match target {
        BuildTarget::Flake { attribute, .. } => {
            match attribute.as_slice() {
                [] => {
                    attribute.push(String::from("nixosConfigurations"));
                    attribute.push(hostname.to_owned());
                }
                [head] if head == "nixosConfigurations" => {
                    info!(
                        "Inferring hostname '{hostname}' for \
                         nixosConfigurations"
                    );
                    attribute.push(hostname.to_owned());
                }
                [head, _] if head == "nixosConfigurations" => {}
                [head, second, ..] if head == "nixosConfigurations" => {
                    bail!(
                        "Attribute path is too specific: {attribute}. Please \
                         either:\n  1. Use the flake reference without \
                         attributes (e.g., '.')\n  2. Specify only the \
                         configuration name (e.g., '.#{second}')"
                    );
                }
                _ => {
                    attribute
                        .insert(0, String::from("nixosConfigurations"));
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

    Ok(())
}

impl ActivationRequest {
    fn build_and_activate(
        &self,
        config: &Config,
        elevate: bool,
    ) -> Result<()> {
        let out_link = OutLink::for_activation(
            self.rebuild.build.out_link.as_deref(),
        )?;

        let toplevel = self.rebuild.prepare_toplevel(&config.env)?;
        let built = self.rebuild.build_and_diff(toplevel, &out_link)?;

        if self.activation.dry {
            if self.activation.ask {
                warn!("--ask has no effect as dry run was requested");
            }

            return Ok(());
        }

        self.activate_rebuilt_config(&built, config, elevate)?;

        Ok(())
    }

    fn activate_rebuilt_config(
        &self,
        built: &BuiltSystem,
        config: &Config,
        elevate: bool,
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
            self.resolve_closure(&built.selected)?;

        match action {
            ActivationAction::Test { .. } => {
                self.activate_test_phase(
                    &switch_to_configuration,
                    config,
                    elevate,
                )?;
            }
            ActivationAction::Boot { .. } => {
                self.activate_boot_phase(
                    built,
                    &switch_to_configuration,
                    config,
                    elevate,
                )?;
            }
            ActivationAction::Switch { .. } => {
                self.activate_test_phase(
                    &switch_to_configuration,
                    config,
                    elevate,
                )?;
                self.activate_boot_phase(
                    built,
                    &switch_to_configuration,
                    config,
                    elevate,
                )?;
            }
        }

        debug!(
            "Completed {action:?} operation with store path: {}",
            built.selected.display()
        );

        Ok(())
    }

    /// Validates the closure to activate and resolves the path to its
    /// `switch-to-configuration` binary.
    fn resolve_closure(&self, selected: &Path) -> Result<PathBuf> {
        if self.activation.no_validate {
            warn!(
                "Skipping pre-activation validation (--no-validate set)"
            );
            warn!(
                "This may result in activation failures if the system closure is \
         incomplete"
            );
        } else {
            validate_system_closure(selected)?;
        }

        let switch_to_configuration = selected
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
        config: &Config,
        elevate: bool,
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
        config: &Config,
        elevate: bool,
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

/// Essential files that must exist in a valid NixOS system closure. Each tuple
/// contains the file path relative to the system profile and its description.
/// The descriptions are used on log messages or errors.
const ESSENTIAL_FILES: &[(&str, &str)] = &[
    ("bin/switch-to-configuration", "activation script"),
    ("nixos-version", "system version identifier"),
    ("init", "system init script"),
    ("sw/bin", "system path"),
];

/// Validates that essential files exist in the system closure.
///
/// Checks for the files in [`ESSENTIAL_FILES`], which must be present in a
/// complete NixOS system. This is essentially in-line with what
/// nixos-rebuild-ng checks for.
///
/// # Errors
///
/// Returns an error listing the missing files, with common causes and fixes.
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
#[expect(
    clippy::unwrap_used,
    clippy::panic,
    clippy::used_underscore_binding,
    reason = "Test assertions"
)]
mod tests {
    use super::*;
    use crate::target::AttrPath;
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
        assert_eq!(built.selected, built.toplevel);
    }

    #[test]
    fn resolve_resolves_specialisation_to_its_own_store_path() {
        let dir = tempfile::tempdir().unwrap();
        let out_link = fake_built_system(dir.path());
        let base = dir.path().canonicalize().unwrap();

        let built = BuiltSystem::resolve(&out_link, Some("foo")).unwrap();

        assert_eq!(built.toplevel, base.join("store-toplevel"));
        assert_eq!(built.selected, base.join("store-spec"));
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

    /// A flake target carrying the given attribute segments.
    fn flake_target(segments: &[&str]) -> BuildTarget {
        let mut attribute = AttrPath::default();
        attribute
            .extend(segments.iter().map(|segment| String::from(*segment)));
        BuildTarget::Flake {
            reference: String::from("."),
            attribute,
        }
    }

    fn flake_attribute(target: &BuildTarget) -> &AttrPath {
        let BuildTarget::Flake { attribute, .. } = target else {
            panic!("expected flake target")
        };
        attribute
    }

    #[test]
    fn bare_flake_reference_infers_hostname() {
        let mut target = flake_target(&[]);

        select_toplevel(&mut target, "host").unwrap();

        assert_eq!(
            flake_attribute(&target).to_vec(),
            [
                "nixosConfigurations",
                "host",
                "config",
                "system",
                "build",
                "toplevel"
            ]
        );
    }

    #[test]
    fn bare_nixos_configurations_infers_hostname() {
        let mut target = flake_target(&["nixosConfigurations"]);

        select_toplevel(&mut target, "host").unwrap();

        assert_eq!(
            flake_attribute(&target).to_vec(),
            [
                "nixosConfigurations",
                "host",
                "config",
                "system",
                "build",
                "toplevel"
            ]
        );
    }

    #[test]
    fn complete_configuration_path_only_gains_toplevel() {
        let mut target = flake_target(&["nixosConfigurations", "host"]);

        select_toplevel(&mut target, "other").unwrap();

        assert_eq!(
            flake_attribute(&target).to_vec(),
            [
                "nixosConfigurations",
                "host",
                "config",
                "system",
                "build",
                "toplevel"
            ]
        );
    }

    #[test]
    fn bare_configuration_name_is_qualified() {
        let mut target = flake_target(&["myhost"]);

        select_toplevel(&mut target, "host").unwrap();

        assert_eq!(
            flake_attribute(&target).to_vec(),
            [
                "nixosConfigurations",
                "myhost",
                "config",
                "system",
                "build",
                "toplevel"
            ]
        );
    }

    #[test]
    fn too_specific_path_is_rejected() {
        let mut target =
            flake_target(&["nixosConfigurations", "host", "extra"]);

        let error = select_toplevel(&mut target, "host").unwrap_err();

        assert!(error.to_string().contains("too specific"));
    }

    #[test]
    fn file_target_gains_toplevel_attribute() {
        let mut target = BuildTarget::File {
            path: PathBuf::from("/etc/nixos/configuration.nix"),
            attribute: AttrPath::default(),
        };

        select_toplevel(&mut target, "host").unwrap();

        let BuildTarget::File { attribute, .. } = &target else {
            panic!("expected file target")
        };
        assert_eq!(
            attribute.to_vec(),
            ["config", "system", "build", "toplevel"]
        );
    }

    #[test]
    fn build_out_link_defaults_to_result() {
        let link = OutLink::for_build(None);

        assert_eq!(link.path, PathBuf::from("result"));
        assert!(link._temp_dir.is_none());

        let explicit = OutLink::for_build(Some(Path::new("/tmp/out")));
        assert_eq!(explicit.path, PathBuf::from("/tmp/out"));
        assert!(explicit._temp_dir.is_none());
    }

    #[test]
    fn activation_out_link_defaults_to_temporary() {
        let link = OutLink::for_activation(None).unwrap();

        assert!(link.path.ends_with("result"));
        assert!(link._temp_dir.is_some());

        let explicit =
            OutLink::for_activation(Some(Path::new("/tmp/out"))).unwrap();
        assert_eq!(explicit.path, PathBuf::from("/tmp/out"));
        assert!(explicit._temp_dir.is_none());
    }
}
