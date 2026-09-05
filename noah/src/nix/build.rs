//! Running `nix build`, optionally piped through `nom`.

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
use crate::nix::options::NixBuildOptions;
use crate::target::BuildTarget;

#[derive(Debug)]
pub struct Build {
    message: Option<String>,
    target: BuildTarget,
    extra_args: Vec<OsString>,
    nom: bool,
    dry_run: bool,
}

impl Build {
    #[must_use]
    pub const fn new(target: BuildTarget) -> Self {
        Self {
            message: None,
            target,
            extra_args: vec![],
            nom: false,
            dry_run: false,
        }
    }

    #[must_use]
    pub fn message(mut self, message: impl AsRef<str>) -> Self {
        self.message = Some(message.as_ref().to_owned());
        self
    }

    #[must_use]
    pub fn extra_arg(mut self, arg: impl AsRef<OsStr>) -> Self {
        self.extra_args.push(arg.as_ref().to_os_string());
        self
    }

    #[must_use]
    pub const fn nom(mut self, yes: bool) -> Self {
        self.nom = yes;
        self
    }

    #[must_use]
    pub const fn dry_run(mut self, yes: bool) -> Self {
        self.dry_run = yes;
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

        let target_args = self.target.to_args();

        let base_command = NixCommand::new(Kind::Build)
            .args(self.dry_run.then_some("--dry-run"))
            .args(&target_args)
            .args(&self.extra_args)
            .into_exec();

        if self.nom {
            let pipeline = {
                base_command
                    .args(["--log-format", "internal-json", "--verbose"])
                    .stderr(Redirection::Merge)
                    .stdout(Redirection::Pipe)
                    | Exec::cmd("nom").args(["--json"])
            }
            .stdout(Redirection::None);
            debug!(?pipeline);

            // Use `popen()` to get access to individual processes so we can check
            // Nix's exit status, not nom's. The pipeline's `join()` only returns
            // the exit status of the last command (nom), which always succeeds
            // even when Nix fails.
            let job = pipeline.start()?;

            // Wait for all processes to finish
            for proc in &job.processes {
                proc.wait()?;
            }

            // Check the exit status of the FIRST process (nix build)
            // This is the one that matters. If Nix fails, we should fail as well
            if let Some(nix_proc) = job.processes.first() {
                let exit_status = nix_proc.wait()?;
                if !exit_status.success() {
                    bail!(ExitError(exit_status));
                }
            }
        } else {
            let cmd = base_command
                .stderr(Redirection::Merge)
                .stdout(Redirection::None);

            debug!(?cmd);
            let exit = cmd.join();

            let exit_status = exit?;
            if !exit_status.success() {
                bail!(ExitError(exit_status));
            }
        }

        Ok(())
    }
}

#[derive(Debug, Error)]
#[error("Command exited with status {0:?}")]
pub struct ExitError(ExitStatus);
