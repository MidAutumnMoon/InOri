use ino_shell::{Shell, cmd};
use nh::{
    EyreRootcauseBridge,
    command::{ElevationStrategy, ElevationStrategyArg, SudoConfig},
    remote::SshConfig,
    runtime::RuntimeEnv,
};
use nh_installable::FlakeConfig;
use rootcause::Result;
use rootcause::prelude::ResultExt;
use tracing::debug;

mod interface;

const NH_VERSION: &str = env!("CARGO_PKG_VERSION");
const NH_REV: Option<&str> = option_env!("NH_REV");

/// All runtime configuration captured from the environment at startup.
///
/// Assembled once in `main()`, passed by reference to subcommands.
/// Each subsystem receives only the slice it needs.
struct RuntimeConfig {
    process: RuntimeEnv,
    sudo: SudoConfig,
    flake: FlakeConfig,
    ssh: SshConfig,
}

impl RuntimeConfig {
    fn from_env(process: RuntimeEnv) -> Result<Self> {
        let sudo = SudoConfig::from_env(&process)?;
        let flake = FlakeConfig {
            os_flake: process
                .non_empty_var("NH_OS_FLAKE")
                .map(str::to_owned),
            flake: process.non_empty_var("NH_FLAKE").map(str::to_owned),
            file: process.non_empty_var("NH_FILE").map(str::to_owned),
            attrp: process.var("NH_ATTRP").unwrap_or_default().to_owned(),
        };
        let ssh = SshConfig::from_env(&process)?;

        Ok(Self {
            process,
            sudo,
            flake,
            ssh,
        })
    }
}

/// Variant of the system Nix. Determinate Nix is not supported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NixVariant {
    Nix,
    Lix,
}

fn main() -> rootcause::Result<()> {
    ino_tracing::init_tracing_subscriber();
    let mut args = <interface::Main as clap::Parser>::parse();

    color_eyre::config::HookBuilder::default()
        .display_location_section(true)
        .panic_section(
            "Please report the bug at https://github.com/nix-community/nh/issues",
        )
        .display_env_section(false)
        .install().into_rootcause()?;

    // Capture environment-derived configuration once, after clap has handled
    // early exits such as --help and --version.
    let process = RuntimeEnv::capture()?;
    let env = RuntimeConfig::from_env(process)?;

    // Validate the Nix environment before dispatching the command.
    nix_variant()?;

    // Backward compatibility: support NH_ELEVATION_PROGRAM env var if
    // NH_ELEVATION_STRATEGY is not set.
    // TODO: Remove this fallback in a future version
    if args.elevation_strategy.is_none()
        && let Some(old_value) =
            env.process.non_empty_var("NH_ELEVATION_PROGRAM")
    {
        tracing::warn!(
            "NH_ELEVATION_PROGRAM is deprecated, use NH_ELEVATION_STRATEGY instead. \
            Falling back to NH_ELEVATION_PROGRAM for backward compatibility. \
            Accepted values: none, passwordless, program:<path>"
        );
        args.elevation_strategy = Some(old_value.into());
    }

    tracing::debug!("{args:#?}");
    tracing::debug!(%NH_VERSION, ?NH_REV);

    let elevation = args.elevation_strategy.as_ref().map_or(
        ElevationStrategy::Auto,
        |arg| match arg {
            ElevationStrategyArg::Auto => ElevationStrategy::Auto,
            ElevationStrategyArg::None => ElevationStrategy::None,
            ElevationStrategyArg::Passwordless => {
                ElevationStrategy::Passwordless
            }
            ElevationStrategyArg::Program(path) => {
                ElevationStrategy::Prefer(path.clone())
            }
        },
    );

    args.command.run(&env, elevation)
}

fn nix_variant() -> rootcause::Result<NixVariant> {
    let variant = guess_nix_variant_from_version_output()?;
    ensure_features_needed_are_set()?;
    Ok(variant)
}

fn guess_nix_variant_from_version_output() -> rootcause::Result<NixVariant>
{
    let shell = Shell::new()?;
    let version_output = cmd!(shell, "nix --version")
        .read()
        .context("Failed to run `nix --version`")?;

    if version_output.to_lowercase().contains("lix") {
        Ok(NixVariant::Lix)
    } else {
        Ok(NixVariant::Nix)
    }
}

fn ensure_features_needed_are_set() -> rootcause::Result<()> {
    let shell = Shell::new()?;
    let expr_features =
        cmd!(shell, "nix config show experimental-features")
            .read()
            .context("Failed to read enabled experimental features")?;

    debug!(expr_features);

    if expr_features.contains("flakes")
        && expr_features.contains("nix-command")
    {
        Ok(())
    } else {
        rootcause::bail!(
            "Required flake features (nix-command, flakes) are not enabled"
        )
    }
}
