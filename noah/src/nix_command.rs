//! Building and running `nix` commands.
//!
//! [`NixCommand`] runs a `nix <kind>` invocation: the kind contributes the
//! subcommand word, and the run uses the stream handling appropriate to it —
//! interactive commands inherit the standard streams, non-interactive ones
//! drain stdout and stderr concurrently without deadlock.

use std::ffi::OsStr;
use std::ffi::OsString;
use std::io;
use std::io::Write;

use rootcause::Result;
use rootcause::prelude::ResultExt as _;
use subprocess::Capture;
use subprocess::Exec;
use subprocess::ExitStatus;
use subprocess::Job;
use subprocess::Redirection;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandKind {
    Build,
    Flake,
    PathInfo,
    Repl,
}

impl CommandKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::Flake => "flake",
            Self::PathInfo => "path-info",
            Self::Repl => "repl",
        }
    }

    const fn interactive(self) -> bool {
        matches!(self, Self::Repl)
    }
}

pub struct NixCommand {
    kind: CommandKind,
    args: Vec<OsString>,
    env: Vec<(OsString, OsString)>,
    interactive: bool,
}

impl NixCommand {
    #[must_use]
    pub fn new(kind: CommandKind) -> Self {
        Self {
            kind,
            args: Vec::new(),
            env: Vec::new(),
            interactive: kind.interactive(),
        }
    }

    #[must_use]
    pub fn arg(mut self, arg: impl AsRef<OsStr>) -> Self {
        self.args.push(arg.as_ref().to_os_string());
        self
    }

    #[must_use]
    pub fn args<I>(mut self, args: I) -> Self
    where
        I: IntoIterator,
        I::Item: AsRef<OsStr>,
    {
        self.args.extend(
            args.into_iter().map(|arg| arg.as_ref().to_os_string()),
        );
        self
    }

    #[must_use]
    pub fn envs<I, K, V>(mut self, envs: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        self.env.extend(envs.into_iter().map(|(key, value)| {
            (key.as_ref().to_os_string(), value.as_ref().to_os_string())
        }));
        self
    }

    pub fn into_exec(self) -> Exec {
        let Self {
            kind,
            args,
            env,
            ..
        } = self;
        let mut argv = Vec::with_capacity(2 + args.len());
        argv.push(OsString::from("nix"));
        argv.push(OsString::from(kind.as_str()));
        argv.extend(args);
        let command = Exec::cmd("nix").args(argv);
        if env.is_empty() {
            command
        } else {
            command.env_extend(env)
        }
    }

    /// Run the command, forwarding stdout and stderr as they arrive.
    ///
    /// Interactive commands inherit all three standard streams.
    /// Non-interactive commands drain stdout and stderr concurrently.
    ///
    /// # Errors
    ///
    /// Returns an error if the command cannot start, stream communication
    /// or waiting fails.
    pub fn run_with_logs(self) -> Result<ExitStatus> {
        if self.interactive {
            return self.run_interactive();
        }

        let mut stdout = io::stdout();
        let mut stderr = io::stderr();
        self.communicate_to(&mut stdout, &mut stderr)
    }

    /// Run the command and collect stdout and stderr.
    ///
    /// # Errors
    ///
    /// Returns an error if the command cannot start, capture or waiting
    /// fails.
    pub fn output(self) -> Result<Capture> {
        capture_exec(self.into_exec())
    }

    fn run_interactive(self) -> Result<ExitStatus> {
        let job = self
            .into_exec()
            .start()
            .context("Failed to start nix")?;
        job.wait()
            .context("Failed to wait for nix")
            .map_err(Into::into)
    }

    fn communicate_to(
        self,
        stdout: &mut dyn Write,
        stderr: &mut dyn Write,
    ) -> Result<ExitStatus> {
        let mut job = self
            .into_exec()
            .stdout(Redirection::Pipe)
            .stderr(Redirection::Pipe)
            .start()
            .context("Failed to start nix")?;
        let communication = job
            .communicate()
            .context("Failed to open nix output pipes")?
            .read_to(stdout, stderr);

        if let Err(error) = communication {
            kill_and_wait(&job)?;
            return Err(error)
                .context("Failed to stream nix output")
                .map_err(Into::into);
        }

        job.wait()
            .context("Failed to wait for nix")
            .map_err(Into::into)
    }
}

fn capture_exec(exec: Exec) -> Result<Capture> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit_status = communicate_exec(exec, &mut stdout, &mut stderr)?;
    Ok(Capture {
        stdout,
        stderr,
        exit_status,
    })
}

fn communicate_exec(
    exec: Exec,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<ExitStatus> {
    let mut job = exec
        .stdout(Redirection::Pipe)
        .stderr(Redirection::Pipe)
        .start()
        .context("Failed to start nix")?;
    let communication = job
        .communicate()
        .context("Failed to open nix output pipes")?
        .read_to(stdout, stderr);

    if let Err(error) = communication {
        kill_and_wait(&job)?;
        return Err(error)
            .context("Failed to stream nix output")
            .map_err(Into::into);
    }

    job.wait()
        .context("Failed to wait for nix")
        .map_err(Into::into)
}

fn kill_and_wait(job: &Job) -> Result<()> {
    let kill_result = job.kill();
    job.wait()?;
    Ok(kill_result?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interactive_defaults_are_schema_owned() {
        assert!(NixCommand::new(CommandKind::Repl).interactive);
        assert!(!NixCommand::new(CommandKind::Build).interactive);
    }

    #[cfg(unix)]
    #[test]
    #[expect(clippy::panic_in_result_fn, reason = "tests")]
    fn capture_exec_captures_both_streams() -> Result<()> {
        let output = capture_exec(
            Exec::cmd("sh")
                .args(["-c", "printf stdout; printf stderr >&2"]),
        )?;
        assert!(output.success());
        assert_eq!(output.stdout, b"stdout");
        assert_eq!(output.stderr, b"stderr");
        Ok(())
    }
}
