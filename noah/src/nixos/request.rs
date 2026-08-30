use std::path::PathBuf;

use crate::diff::Mode as DiffMode;
use crate::nix_options::NixBuildOptions;
use crate::nixos::generations::Field;
use crate::remote::Host;
use crate::target::BuildTarget;
use crate::update::Selection;
#[derive(Clone, Debug)]
pub enum RebuildCommand {
    Build(RebuildRequest),
    Activate(ActivationRequest),
}

#[derive(Clone, Debug)]
pub struct RebuildRequest {
    pub build: BuildOptions,
    pub update: Option<Selection>,
    pub hostname: Option<String>,
    pub specialisation: SpecialisationSelection,
    pub extra_args: Vec<String>,
    pub bypass_root_check: bool,
    pub target_host: Option<Host>,
    pub build_host: Option<Host>,
    pub commit_lock_file: bool,
    pub use_substitutes: bool,
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
    pub rebuild: RebuildRequest,
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

#[derive(Clone, Debug)]
pub enum SpecialisationSelection {
    Current,
    Base,
    Named(String),
}

#[derive(Clone, Debug)]
pub struct RollbackRequest {
    pub dry: bool,
    pub ask: bool,
    pub specialisation: SpecialisationSelection,
    pub to: Option<u64>,
    pub bypass_root_check: bool,
    pub diff: DiffMode,
}

#[derive(Clone, Debug)]
pub struct ReplRequest {
    pub target: Option<BuildTarget>,
    pub hostname: Option<String>,
}

#[derive(Clone, Debug)]
pub struct GenerationsRequest {
    pub profile: PathBuf,
    pub fields: Option<Vec<Field>>,
}
