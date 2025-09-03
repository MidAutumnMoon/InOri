//! This module essentially reimplements nixos-rebuild-ng

use std::fs;
use std::path::{Path, PathBuf};

use color_eyre::eyre::{Context, bail};
use color_eyre::eyre::{Result, eyre};
use tracing::{debug, info, warn};

use crate::Runtime;
use crate::commands;
use crate::commands::Command;
use crate::generations;
use crate::handy;
use crate::handy::ensure_ssh_key_login;
use crate::handy::print_dix_diff;

const SYSTEM_PROFILE: &str = "/nix/var/nix/profiles/system";
const CURRENT_PROFILE: &str = "/run/current-system";

#[derive(clap::ValueEnum, Clone, Default, Debug)]
pub enum DiffType {
    /// Display package diff only if the of the
    /// current and the deployed configuration matches
    #[default]
    Auto,
    /// Always display package diff
    Always,
    /// Never display package diff
    Never,
}

#[derive(Debug, clap::Subcommand)]
pub enum OsSubcmd {
    /// Build && activate && add to boot entry.
    Switch(BuildOpts),

    /// Build && add to boot entry
    Boot(BuildOpts),

    /// Build && activate
    Test(BuildOpts),

    /// Build only
    Build(BuildOpts),

    /// Open an REPL with the configuration.
    Repl(ReplOpts),

    /// List system generations.
    // TODO: show info about /run/current-system
    // TODO: rename
    Info(OsGenerationsArgs),

    /// Rollback to a previous generation
    Rollback(RollbackOpts),

    /// Build VM
    // TODO: remove?
    Vm(BuildVmOpts),

    /// Update flake.lock and commit. Currently the commit message is
    /// hardcoded.
    Update {
        /// Disable automatic commit.
        #[arg(long, short)]
        no_commit: bool,
    },
}

impl OsSubcmd {
    pub fn run(self, runtime: Runtime) -> Result<()> {
        use BuildVariant::{Boot, Build, Switch, Test};
        match self {
            Self::Boot(opts) => build_nixos(opts, Boot, &runtime),
            Self::Test(opts) => build_nixos(opts, Test, &runtime),
            Self::Switch(opts) => build_nixos(opts, Switch, &runtime),
            Self::Build(opts) => {
                if opts.dry {
                    warn!("`--dry` have no effect for `nh os build`");
                }
                build_nixos(opts, Build, &runtime)
            }
            Self::Vm(opts) => {
                let variant = if opts.with_bootloader {
                    BuildVariant::VmWithBootloader
                } else {
                    BuildVariant::Vm
                };
                build_nixos(opts.common, variant, &runtime)
            }
            Self::Repl(opts) => opts.run(&runtime),
            Self::Info(opts) => opts.info(),
            Self::Rollback(opts) => opts.rollback(&runtime),
            Self::Update { .. } => todo!(),
        }
    }
}

#[derive(Debug, clap::Args)]
pub struct BuildVmOpts {
    #[command(flatten)]
    pub common: BuildOpts,

    /// Build with bootloader. Bootloader is bypassed by default.
    #[arg(long, short = 'B')]
    pub with_bootloader: bool,
}

#[derive(Debug, clap::Args)]
pub struct BuildOpts {
    /// Only print actions, without performing them
    #[arg(long, short = 'n')]
    pub dry: bool,

    /// Whether to display a package diff
    #[arg(long, short, value_enum, default_value_t = DiffType::Auto)]
    pub diff: DiffType,

    #[command(flatten)]
    pub passthrough: NixBuildPassthroughArgs,

    /// Select this hostname from nixosConfigurations
    #[arg(long, short = 'H', global = true)]
    pub hostname: Option<String>,

    /// Extra arguments passed to nix build
    #[arg(last = true)]
    pub extra_args: Vec<String>,

    /// Deploy the configuration to a different host over ssh
    #[arg(long)]
    pub target_host: Option<String>,

    /// Build the configuration to a different host over ssh
    #[arg(long)]
    pub builders: Option<String>,
}

#[derive(Debug, clap::Args)]
pub struct RollbackOpts {
    /// Only print actions, without performing them
    #[arg(long, short = 'n')]
    pub dry: bool,

    /// Rollback to a specific generation number (defaults to previous generation)
    #[arg(long, short)]
    pub to: Option<u64>,

    /// Whether to display a package diff
    #[arg(long, short, value_enum, default_value_t = DiffType::Auto)]
    pub diff: DiffType,
}

#[derive(Debug, clap::Args)]
pub struct ReplOpts {
    /// Select the hostname.
    #[arg(long, short = 'H', global = true)]
    pub hostname: Option<String>,
}

#[derive(Debug, clap::Args)]
pub struct OsGenerationsArgs {
    /// Path to Nix' profiles directory
    #[arg(
        long,
        short = 'P',
        default_value = "/nix/var/nix/profiles/system"
    )]
    pub profile: Option<String>,
}

#[derive(Debug, clap::Args)]
// TODO: this does not need to be a catalog of options
pub struct NixBuildPassthroughArgs {
    /// Number of concurrent jobs Nix should run
    #[arg(long, short = 'j')]
    pub max_jobs: Option<usize>,

    /// Number of cores Nix should utilize
    #[arg(long)]
    pub cores: Option<usize>,

    /// Continue building despite encountering errors
    #[arg(long, short = 'k')]
    pub keep_going: bool,

    /// Keep build outputs from failed builds
    #[arg(long, short = 'K')]
    pub keep_failed: bool,

    /// Print build logs.
    #[arg(long, short = 'L')]
    pub print_build_logs: bool,

    /// Display tracebacks on errors
    #[arg(long, short = 't')]
    pub show_trace: bool,

    /// Build without internet access
    #[arg(long)]
    pub offline: bool,
}

impl NixBuildPassthroughArgs {
    #[must_use]
    pub fn generate_passthrough_args(&self) -> Vec<String> {
        let mut args = Vec::new();

        if let Some(jobs) = self.max_jobs {
            args.push("--max-jobs".into());
            args.push(jobs.to_string());
        }
        if let Some(cores) = self.cores {
            args.push("--cores".into());
            args.push(cores.to_string());
        }
        if self.keep_going {
            args.push("--keep-going".into());
        }
        if self.keep_failed {
            args.push("--keep-failed".into());
        }
        if self.print_build_logs {
            args.push("--print-build-logs".into());
        }
        if self.show_trace {
            args.push("--show-trace".into());
        }
        if self.offline {
            args.push("--offline".into());
        }
        args
    }
}

#[derive(Debug)]
enum BuildVariant {
    Build,
    Switch,
    Boot,
    Test,
    Vm,
    VmWithBootloader,
}

impl BuildVariant {
    fn attr(&self) -> &'static str {
        match self {
            Self::Build | Self::Switch | Self::Boot | Self::Test => {
                "toplevel"
            }
            Self::Vm => "vm",
            Self::VmWithBootloader => "vmWithBootLoader",
        }
    }
}

#[expect(clippy::too_many_lines)]
fn build_nixos(
    build_opts: BuildOpts,
    variant: BuildVariant,
    runtime: &Runtime,
) -> Result<()> {
    use BuildVariant::{Boot, Build, Switch, Test, Vm};

    if build_opts.builders.is_some() || build_opts.target_host.is_some() {
        // if it fails its okay
        let _ = ensure_ssh_key_login();
    }

    let elevate = if runtime.no_root_check {
        warn!("Bypassing root check, now running nix as root");
        false
    } else {
        if nix::unistd::Uid::effective().is_root() {
            bail!(
                "Don't run nh os as root. I will call sudo internally as needed"
            );
        }
        true
    };

    let local_hostname = handy::hostname()
        .context("Failed to get hostname of current machine")?;

    let target_hostname = match &build_opts.hostname {
        Some(h) => h.to_owned(),
        None => {
            // TODO: reword
            info!("Using hostname {local_hostname}");
            local_hostname.clone()
        }
    };

    let (out_path, _tempdir_guard): (PathBuf, Option<tempfile::TempDir>) =
        match variant {
            Vm | Build => (PathBuf::from("result"), None),
            _ => {
                let dir =
                    tempfile::Builder::new().prefix("nh-os").tempdir()?;
                (dir.as_ref().join("result"), Some(dir))
            }
        };

    debug!("Output path: {out_path:?}");

    let drv = format! {
        "{}#nixosConfigurations.{}.config.system.build.{}",
        runtime.flake,
        target_hostname,
        variant.attr(),
    };

    let message = match variant {
        Vm => "Building NixOS VM image",
        _ => "Building NixOS configuration",
    };

    commands::Build::new(drv)
        .extra_arg("--out-link")
        .extra_arg(&out_path)
        .extra_args(&build_opts.extra_args)
        .passthrough(&build_opts.passthrough)
        .builder(build_opts.builders.clone())
        .message(message)
        .run()
        .wrap_err("Failed to build configuration")?;

    let target_profile = out_path.clone();

    debug!("Output path: {out_path:?}");
    debug!("Target profile path: {}", target_profile.display());
    debug!("Target profile exists: {}", target_profile.exists());

    if !target_profile
        .try_exists()
        .context("Failed to check if target profile exists")?
    {
        return Err(eyre!(
            "Target profile path does not exist: {}",
            target_profile.display()
        ));
    }

    match build_opts.diff {
        DiffType::Always => {
            let _ = print_dix_diff(
                &PathBuf::from(CURRENT_PROFILE),
                &target_profile,
            );
        }
        DiffType::Never => {
            debug!("Not running dix as the --diff flag is set to never.");
        }
        DiffType::Auto => {
            // if local_hostname.is_none_or(|h| h == target_hostname)
            //     && self.target_host.is_none()
            //     && self.build_host.is_none()
            // {
            //     debug!(
            //         "Comparing with target profile: {}",
            //         target_profile.display()
            //     );
            //     let _ = print_dix_diff(
            //         &PathBuf::from(CURRENT_PROFILE),
            //         &target_profile,
            //     );
            // } else {
            //     debug!(
            //         "Not running dix as the target hostname is different from the system hostname."
            //     );
            // }
            todo!()
        }
    }

    if build_opts.dry || matches!(variant, Build | Vm) {
        return Ok(());
    }

    if let Some(target_host) = &build_opts.target_host {
        Command::new("nix")
            .args([
                "copy",
                "--to",
                format!("ssh://{target_host}").as_str(),
                match target_profile.to_str() {
                    Some(s) => s,
                    None => {
                        return Err(eyre!(
                            "target_profile path is not valid UTF-8"
                        ));
                    }
                },
            ])
            .message("Copying configuration to target")
            .with_required_env()
            .run()?;
    }

    if let Test | Switch = variant {
        let switch_to_configuration =
            target_profile.join("bin").join("switch-to-configuration");

        if !switch_to_configuration.exists() {
            return Err(eyre!(
                "The 'switch-to-configuration' binary is missing from the built configuration.\n\
         \n\
         This typically happens when 'system.switch.enable' is set to false in your\n\
         NixOS configuration. To fix this, please either:\n\
         1. Remove 'system.switch.enable = false' from your configuration, or\n\
         2. Set 'system.switch.enable = true' explicitly\n\
         \n\
         If the problem persists, please open an issue on our issue tracker!"
            ));
        }

        let switch_to_configuration = switch_to_configuration
            .canonicalize()
            .context("Failed to resolve switch-to-configuration path")?;
        let switch_to_configuration =
            switch_to_configuration.to_str().ok_or_else(|| {
                eyre!(
                    "switch-to-configuration path contains invalid UTF-8"
                )
            })?;

        Command::new(switch_to_configuration)
            .arg("test")
            .ssh(build_opts.target_host.clone())
            .message("Activating configuration")
            .elevate(elevate)
            .preserve_envs(["NIXOS_INSTALL_BOOTLOADER"])
            .with_required_env()
            .run()
            .wrap_err("Activation (test) failed")?;
    }

    if let Boot | Switch = variant {
        let canonical_out_path = out_path
            .canonicalize()
            .context("Failed to resolve output path")?;

        Command::new("nix")
            .elevate(elevate)
            .args(["build", "--no-link", "--profile", SYSTEM_PROFILE])
            .arg(&canonical_out_path)
            .ssh(build_opts.target_host.clone())
            .with_required_env()
            .run()
            .wrap_err("Failed to set system profile")?;

        let switch_to_configuration =
            out_path.join("bin").join("switch-to-configuration");

        if !switch_to_configuration.exists() {
            return Err(eyre!(
                "The 'switch-to-configuration' binary is missing from the built configuration.\n\
         \n\
         This typically happens when 'system.switch.enable' is set to false in your\n\
         NixOS configuration. To fix this, please either:\n\
         1. Remove 'system.switch.enable = false' from your configuration, or\n\
         2. Set 'system.switch.enable = true' explicitly\n\
         \n\
         If the problem persists, please open an issue on our issue tracker!"
            ));
        }

        let switch_to_configuration = switch_to_configuration
            .canonicalize()
            .context("Failed to resolve switch-to-configuration path")?;
        let switch_to_configuration =
            switch_to_configuration.to_str().ok_or_else(|| {
                eyre!(
                    "switch-to-configuration path contains invalid UTF-8"
                )
            })?;

        Command::new(switch_to_configuration)
            .arg("boot")
            .ssh(build_opts.target_host)
            .elevate(elevate)
            .message("Adding configuration to bootloader")
            .preserve_envs(["NIXOS_INSTALL_BOOTLOADER"])
            .with_required_env()
            .run()
            .wrap_err("Bootloader activation failed")?;
    }

    debug!("Completed operation with output path: {out_path:?}");

    Ok(())
}

impl RollbackOpts {
    fn rollback(&self, runtime: &Runtime) -> Result<()> {
        let elevate = if runtime.no_root_check {
            warn!("Bypassing root check, now running nix as root");
            false
        } else {
            if nix::unistd::Uid::effective().is_root() {
                bail!(
                    "Don't run nh os as root. I will call sudo internally as needed"
                );
            }
            true
        };

        // Find previous generation or specific generation
        let target_generation = if let Some(gen_number) = self.to {
            find_generation_by_number(gen_number)?
        } else {
            find_previous_generation()?
        };

        info!("Rolling back to generation {}", target_generation.number);

        // Construct path to the generation
        let profile_dir = Path::new(SYSTEM_PROFILE).parent().unwrap_or_else(|| {
            tracing::warn!("SYSTEM_PROFILE has no parent, defaulting to /nix/var/nix/profiles");
            Path::new("/nix/var/nix/profiles")
        });
        let generation_link = profile_dir
            .join(format!("system-{}-link", target_generation.number));

        // Compare changes between current and target generation
        if matches!(self.diff, DiffType::Never) {
            debug!(
                "Not running dix as the target hostname is different from the system hostname."
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

        // Get current generation number for potential rollback
        let current_gen_number = match get_current_generation_number() {
            Ok(num) => num,
            Err(e) => {
                warn!("Failed to get current generation number: {}", e);
                0
            }
        };

        // Set the system profile
        info!("Setting system profile...");

        // Instead of direct symlink operations, use a command with proper elevation
        Command::new("ln")
            .arg("-sfn") // force, symbolic link
            .arg(&generation_link)
            .arg(SYSTEM_PROFILE)
            .elevate(elevate)
            .message("Setting system profile")
            .with_required_env()
            .run()
            .wrap_err("Failed to set system profile during rollback")?;

        let final_profile = generation_link;

        // Activate the configuration
        info!("Activating...");

        let switch_to_configuration =
            final_profile.join("bin").join("switch-to-configuration");

        if !switch_to_configuration.exists() {
            return Err(eyre!(
                "The 'switch-to-configuration' binary is missing from the built configuration.\n\
         \n\
         This typically happens when 'system.switch.enable' is set to false in your\n\
         NixOS configuration. To fix this, please either:\n\
         1. Remove 'system.switch.enable = false' from your configuration, or\n\
         2. Set 'system.switch.enable = true' explicitly\n\
         \n\
         If the problem persists, please open an issue on our issue tracker!"
            ));
        }

        match Command::new(&switch_to_configuration)
            .arg("switch")
            .elevate(elevate)
            .preserve_envs(["NIXOS_INSTALL_BOOTLOADER"])
            .with_required_env()
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
                if current_gen_number > 0 {
                    let current_gen_link = profile_dir
                        .join(format!("system-{current_gen_number}-link"));

                    Command::new("ln")
                        .arg("-sfn") // Force, symbolic link
                        .arg(&current_gen_link)
                        .arg(SYSTEM_PROFILE)
                        .elevate(elevate)
                        .message("Rolling back system profile")
                        .with_required_env()
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

fn find_previous_generation() -> Result<generations::GenerationInfo> {
    let profile_path = PathBuf::from(SYSTEM_PROFILE);

    let mut generations: Vec<generations::GenerationInfo> = fs::read_dir(
        profile_path
            .parent()
            .unwrap_or(Path::new("/nix/var/nix/profiles")),
    )?
    .filter_map(|entry| {
        entry.ok().and_then(|e| {
            let path = e.path();
            if let Some(filename) = path.file_name()
                && let Some(name) = filename.to_str()
                && name.starts_with("system-")
                && name.ends_with("-link")
            {
                return generations::describe(&path);
            }
            None
        })
    })
    .collect();

    if generations.is_empty() {
        bail!("No generations found");
    }

    generations.sort_by(|a, b| {
        a.number
            .parse::<u64>()
            .unwrap_or(0)
            .cmp(&b.number.parse::<u64>().unwrap_or(0))
    });

    let current_idx = generations
        .iter()
        .position(|g| g.current)
        .ok_or_else(|| eyre!("Current generation not found"))?;

    if current_idx == 0 {
        bail!("No generation older than the current one exists");
    }

    Ok(generations[current_idx - 1].clone())
}

fn find_generation_by_number(
    number: u64,
) -> Result<generations::GenerationInfo> {
    let profile_path = PathBuf::from(SYSTEM_PROFILE);

    let generations: Vec<generations::GenerationInfo> = fs::read_dir(
        profile_path
            .parent()
            .unwrap_or(Path::new("/nix/var/nix/profiles")),
    )?
    .filter_map(|entry| {
        entry.ok().and_then(|e| {
            let path = e.path();
            if let Some(filename) = path.file_name()
                && let Some(name) = filename.to_str()
                && name.starts_with("system-")
                && name.ends_with("-link")
            {
                return generations::describe(&path);
            }
            None
        })
    })
    .filter(|generation| generation.number == number.to_string())
    .collect();

    if generations.is_empty() {
        bail!("Generation {} not found", number);
    }

    Ok(generations[0].clone())
}

fn get_current_generation_number() -> Result<u64> {
    let profile_path = PathBuf::from(SYSTEM_PROFILE);

    let generations: Vec<generations::GenerationInfo> = fs::read_dir(
        profile_path
            .parent()
            .unwrap_or(Path::new("/nix/var/nix/profiles")),
    )?
    .filter_map(|entry| {
        entry.ok().and_then(|e| generations::describe(&e.path()))
    })
    .collect();

    let current_gen = generations
        .iter()
        .find(|g| g.current)
        .ok_or_else(|| eyre!("Current generation not found"))?;

    current_gen
        .number
        .parse::<u64>()
        .wrap_err("Invalid generation number")
}

impl ReplOpts {
    fn run(self, runtime: &Runtime) -> Result<()> {
        // TODO: dedup
        let local_hostname = handy::hostname()
            .context("Failed to get hostname of current machine")?;

        let target_hostname = match &self.hostname {
            Some(h) => h.to_owned(),
            None => {
                // TODO: reword
                info!("Using hostname {local_hostname}");
                local_hostname.clone()
            }
        };

        let attr = {
            // TODO: escape?
            format! { "{}#nixosConfigurations.{}",
                runtime.flake,
                target_hostname,
            }
        };

        Command::new("nix")
            .arg("repl")
            .arg(attr)
            .with_required_env()
            .show_output(true)
            .run()?;

        Ok(())
    }
}

impl OsGenerationsArgs {
    fn info(&self) -> Result<()> {
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

        let descriptions: Vec<generations::GenerationInfo> = generations
            .iter()
            .filter_map(|gen_dir| generations::describe(gen_dir))
            .collect();

        let _ = generations::print_info(descriptions);

        Ok(())
    }
}
