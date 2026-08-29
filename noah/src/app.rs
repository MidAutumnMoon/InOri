use nh_installable::FlakeConfig;
use rootcause::Result;

use crate::clean;
use crate::cli::CliCommand;
use crate::command::Elevation;
use crate::command::ElevationStrategy;
use crate::nixos;
use crate::remote::SshConfig;
use crate::runtime::RuntimeEnv;
use crate::search;

/// Runtime configuration captured once after CLI parsing.
pub struct RuntimeConfig {
    pub process: RuntimeEnv,
    pub elevation: Elevation,
    pub flake: FlakeConfig,
    pub ssh: SshConfig,
}

impl RuntimeConfig {
    pub fn from_env(
        process: RuntimeEnv,
        elevation_strategy: Option<ElevationStrategy>,
    ) -> Result<Self> {
        let elevation = Elevation::new(elevation_strategy, &process)?;
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
            elevation,
            flake,
            ssh,
        })
    }
}

/// Execute a parsed CLI command with explicit runtime dependencies.
pub fn run(command: CliCommand, env: &RuntimeConfig) -> Result<()> {
    match command {
        CliCommand::Rebuild(command) => nixos::run_rebuild(*command, env),
        CliCommand::Repl(request) => {
            nixos::run_repl(request, &env.process, &env.flake)
        }
        CliCommand::Info(request) => nixos::run_info(&request),
        CliCommand::Rollback(request) => nixos::run_rollback(request, env),
        CliCommand::Search(request) => search::run(&request),
        CliCommand::Clean(request) => clean::run(&request, env),
    }
}
