//! Running `nix build` directly, through `nom`, or as a forecast.

use std::ffi::OsStr;
use std::ffi::OsString;

use rootcause::Result;
use rootcause::bail;
use subprocess::Exec;
use subprocess::ExitStatus;
use subprocess::Redirection;
use thiserror::Error;
use tracing::debug;
use tracing::info;

use crate::nix::command::Kind;
use crate::nix::command::NixCommand;
use crate::nix::forecast;
use crate::nix::forecast::ForecastOptions;
use crate::nix::options::NixBuildOptions;
use crate::target::BuildTarget;

/// How a Nix build is executed and presented.
#[derive(Clone, Copy, Debug)]
pub enum BuildMode {
    /// Run Nix directly.
    Direct,
    /// Render Nix's internal JSON event stream with `nom`.
    Nom,
    /// Run a dry build and render its forecast.
    Forecast(ForecastOptions),
}

#[derive(Debug)]
pub struct Build {
    message: Option<&'static str>,
    target: BuildTarget,
    extra_args: Vec<OsString>,
    mode: BuildMode,
}

impl Build {
    #[must_use]
    pub const fn new(target: BuildTarget) -> Self {
        Self {
            message: None,
            target,
            extra_args: vec![],
            mode: BuildMode::Direct,
        }
    }

    #[must_use]
    pub const fn message(mut self, message: &'static str) -> Self {
        self.message = Some(message);
        self
    }

    #[must_use]
    pub fn extra_arg(mut self, arg: impl AsRef<OsStr>) -> Self {
        self.extra_args.push(arg.as_ref().to_os_string());
        self
    }

    #[must_use]
    pub const fn mode(mut self, mode: BuildMode) -> Self {
        self.mode = mode;
        self
    }

    #[must_use]
    pub fn extra_args<I>(mut self, args: I) -> Self
    where
        I: IntoIterator,
        I::Item: AsRef<OsStr>,
    {
        for elem in args {
            self.extra_args.push(elem.as_ref().to_os_string());
        }
        self
    }

    #[must_use]
    pub fn nix_options(mut self, options: &NixBuildOptions) -> Self {
        options.append_args(&mut self.extra_args);
        self
    }

    /// Run the build command.
    ///
    /// # Errors
    ///
    /// Returns an error if the build command fails to execute.
    pub fn run(&self) -> Result<()> {
        if let Some(message) = &self.message {
            info!("{message}");
        }

        let command = match self.mode {
            BuildMode::Forecast(_) => NixCommand::new(Kind::Build)
                .arg("--dry-run")
                .args(self.target.to_args())
                .args(&self.extra_args)
                .args(["--log-format", "raw"]),
            BuildMode::Direct | BuildMode::Nom => {
                NixCommand::new(Kind::Build)
                    .args(self.target.to_args())
                    .args(&self.extra_args)
            }
        };
        match self.mode {
            BuildMode::Direct => run_direct(command.into_exec()),
            BuildMode::Nom => run_with_nom(command.into_exec()),
            BuildMode::Forecast(options) => ensure_success(
                "nix build",
                forecast::run(command, options)?,
            ),
        }
    }
}

fn run_direct(command: Exec) -> Result<()> {
    let command =
        command.stderr(Redirection::Merge).stdout(Redirection::None);
    debug!(?command);

    ensure_success("nix build", command.join()?)
}

fn run_with_nom(command: Exec) -> Result<()> {
    let pipeline = {
        command
            .args(["--log-format", "internal-json", "--verbose"])
            .stderr(Redirection::Merge)
            .stdout(Redirection::Pipe)
            | Exec::cmd("nom").args(["--json"])
    }
    .stdout(Redirection::None);
    debug!(?pipeline);

    let job = pipeline.start()?;
    let mut processes = job.processes.iter();
    let Some(nix) = processes.next() else {
        bail!("Nix pipeline started without a nix process");
    };
    let Some(nom) = processes.next() else {
        bail!("Nix pipeline started without a nom process");
    };
    if processes.next().is_some() {
        bail!("Nix pipeline started with unexpected extra processes");
    }

    let nix_status = nix.wait()?;
    let nom_status = nom.wait()?;
    ensure_success("nix build", nix_status)?;
    ensure_success("nom", nom_status)
}

fn ensure_success(
    command: &'static str,
    exit_status: ExitStatus,
) -> Result<()> {
    if !exit_status.success() {
        bail!(ExitError {
            command,
            status: exit_status,
        });
    }
    Ok(())
}

#[derive(Debug, Error)]
#[error("{command} exited with status {status:?}")]
struct ExitError {
    command: &'static str,
    status: ExitStatus,
}
