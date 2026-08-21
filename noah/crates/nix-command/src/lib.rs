use std::{
    ffi::{OsStr, OsString},
    io::{self, Write},
    time::{Duration, Instant},
};

use subprocess::{Capture, Exec, ExitStatus, Job, Redirection};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("command '{command}' timed out after {duration:?}")]
    Timeout { command: String, duration: Duration },
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandKind {
    Build,
    Copy,
    Eval,
    Flake,
    PathInfo,
    Repl,
}

impl CommandKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::Copy => "copy",
            Self::Eval => "eval",
            Self::Flake => "flake",
            Self::PathInfo => "path-info",
            Self::Repl => "repl",
        }
    }

    const fn print_build_logs(self) -> bool {
        matches!(self, Self::Build)
    }

    const fn interactive(self) -> bool {
        matches!(self, Self::Repl)
    }
}

pub struct NixCommand {
    kind: Option<CommandKind>,
    binary: OsString,
    global_args: Vec<OsString>,
    args: Vec<OsString>,
    env: Vec<(OsString, OsString)>,
    print_build_logs: bool,
    interactive: bool,
    timeout: Option<Duration>,
}

impl NixCommand {
    #[must_use]
    pub fn new(kind: CommandKind) -> Self {
        Self::with_binary("nix", Some(kind))
    }

    fn with_binary<S: AsRef<OsStr>>(
        binary: S,
        kind: Option<CommandKind>,
    ) -> Self {
        let (print_build_logs, interactive) = kind
            .map_or((false, false), |kind| {
                (kind.print_build_logs(), kind.interactive())
            });
        Self {
            kind,
            binary: binary.as_ref().to_os_string(),
            global_args: Vec::new(),
            args: Vec::new(),
            env: Vec::new(),
            print_build_logs,
            interactive,
            timeout: None,
        }
    }

    #[must_use]
    pub fn nix_instantiate() -> Self {
        Self::with_binary("nix-instantiate", None)
    }

    #[must_use]
    pub fn arg<S: AsRef<OsStr>>(mut self, arg: S) -> Self {
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
    pub fn global_args<I>(mut self, args: I) -> Self
    where
        I: IntoIterator,
        I::Item: AsRef<OsStr>,
    {
        self.global_args.extend(
            args.into_iter().map(|arg| arg.as_ref().to_os_string()),
        );
        self
    }

    #[must_use]
    pub fn env<K: AsRef<OsStr>, V: AsRef<OsStr>>(
        mut self,
        key: K,
        value: V,
    ) -> Self {
        self.env.push((
            key.as_ref().to_os_string(),
            value.as_ref().to_os_string(),
        ));
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

    #[must_use]
    pub const fn print_build_logs(mut self, yes: bool) -> Self {
        self.print_build_logs = yes;
        self
    }

    #[must_use]
    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    #[must_use]
    pub fn argv(&self) -> Vec<OsString> {
        let mut argv = Vec::with_capacity(
            1 + self.global_args.len() + self.args.len() + 2,
        );
        argv.push(self.binary.clone());
        argv.extend(assemble_args(
            self.kind,
            self.print_build_logs,
            self.global_args.clone(),
            self.args.clone(),
        ));
        argv
    }

    pub fn into_exec(self) -> Exec {
        let Self {
            kind,
            binary,
            global_args,
            args,
            env,
            print_build_logs,
            ..
        } = self;
        let args =
            assemble_args(kind, print_build_logs, global_args, args);
        let command = Exec::cmd(binary).args(args);
        if env.is_empty() {
            command
        } else {
            command.env_extend(env)
        }
    }

    /// Run the command, forwarding stdout and stderr as they arrive.
    ///
    /// Interactive commands inherit all three standard streams. Non-interactive
    /// commands use [`subprocess::Communicator`] to drain stdout and stderr
    /// concurrently without deadlock.
    ///
    /// # Errors
    ///
    /// Returns an error if the command cannot start, stream communication or
    /// waiting fails, or the configured timeout expires.
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
    /// The configured timeout applies to both communication and process exit.
    ///
    /// # Errors
    ///
    /// Returns an error if the command cannot start, capture or waiting fails,
    /// or the configured timeout expires.
    pub fn output(self) -> Result<Capture> {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit_status = self.communicate_to(&mut stdout, &mut stderr)?;
        Ok(Capture {
            stdout,
            stderr,
            exit_status,
        })
    }

    fn run_interactive(self) -> Result<ExitStatus> {
        let timeout = self.timeout_context();
        let job = self.into_exec().start()?;
        let deadline = timeout
            .as_ref()
            .map(|timeout| Instant::now() + timeout.duration);
        wait_for_job(&job, timeout.as_ref(), deadline)
    }

    fn communicate_to(
        self,
        stdout: &mut dyn Write,
        stderr: &mut dyn Write,
    ) -> Result<ExitStatus> {
        let timeout = self.timeout_context();
        let mut job = self
            .into_exec()
            .stdout(Redirection::Pipe)
            .stderr(Redirection::Pipe)
            .start()?;
        let deadline = timeout
            .as_ref()
            .map(|timeout| Instant::now() + timeout.duration);
        let communicator = job.communicate()?;
        let communication = if let Some(deadline) = deadline {
            communicator
                .limit_time(
                    deadline.saturating_duration_since(Instant::now()),
                )
                .read_to(stdout, stderr)
        } else {
            let mut communicator = communicator;
            communicator.read_to(stdout, stderr)
        };

        if let Err(error) = communication {
            kill_and_wait(&job)?;
            if error.kind() == io::ErrorKind::TimedOut
                && let Some(timeout) = timeout
            {
                return Err(timeout.into_error());
            }
            return Err(error.into());
        }

        wait_for_job(&job, timeout.as_ref(), deadline)
    }

    fn timeout_context(&self) -> Option<TimeoutContext> {
        self.timeout.map(|duration| TimeoutContext {
            command: self.command_name(),
            duration,
        })
    }

    fn command_name(&self) -> String {
        self.kind.map_or_else(
            || self.binary.to_string_lossy().into_owned(),
            |kind| format!("nix {}", kind.as_str()),
        )
    }
}

fn assemble_args(
    kind: Option<CommandKind>,
    print_build_logs: bool,
    mut global_args: Vec<OsString>,
    args: Vec<OsString>,
) -> Vec<OsString> {
    if let Some(kind) = kind {
        global_args.push(OsString::from(kind.as_str()));
    }
    if print_build_logs
        && !args
            .iter()
            .any(|arg| arg == OsStr::new("--no-build-output"))
    {
        global_args.push(OsString::from("--print-build-logs"));
    }
    global_args.extend(args);
    global_args
}

struct TimeoutContext {
    command: String,
    duration: Duration,
}

impl TimeoutContext {
    fn into_error(self) -> Error {
        Error::Timeout {
            command: self.command,
            duration: self.duration,
        }
    }
}

fn wait_for_job(
    job: &Job,
    timeout: Option<&TimeoutContext>,
    deadline: Option<Instant>,
) -> Result<ExitStatus> {
    if let (Some(timeout), Some(deadline)) = (timeout, deadline) {
        if let Some(status) = job.wait_timeout(
            deadline.saturating_duration_since(Instant::now()),
        )? {
            return Ok(status);
        }
        kill_and_wait(job)?;
        return Err(Error::Timeout {
            command: timeout.command.clone(),
            duration: timeout.duration,
        });
    }
    Ok(job.wait()?)
}

fn kill_and_wait(job: &Job) -> Result<()> {
    let _ = job.kill();
    job.wait()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argv_is_deterministic_and_schema_driven() {
        let argv = NixCommand::new(CommandKind::Build)
            .arg("--impure")
            .arg("nixpkgs#hello")
            .argv();
        assert_eq!(
            argv,
            [
                "nix",
                "build",
                "--print-build-logs",
                "--impure",
                "nixpkgs#hello"
            ]
        );
    }

    #[test]
    fn no_build_output_suppresses_print_build_logs() {
        let argv = NixCommand::new(CommandKind::Build)
            .arg("--no-build-output")
            .argv();
        assert_eq!(argv, ["nix", "build", "--no-build-output"]);
    }

    #[test]
    fn eval_defaults_to_quiet_schema() {
        assert_eq!(
            NixCommand::new(CommandKind::Eval).argv(),
            ["nix", "eval"]
        );
    }

    #[test]
    fn interactive_defaults_are_schema_owned() {
        assert!(NixCommand::new(CommandKind::Repl).interactive);
        assert!(!NixCommand::new(CommandKind::Build).interactive);
    }

    #[test]
    fn global_args_are_inserted_before_subcommand() {
        let argv = NixCommand::new(CommandKind::Eval)
            .global_args([
                "--extra-experimental-features",
                "nix-command flakes",
            ])
            .arg("--raw")
            .arg("nixpkgs#hello")
            .argv();
        assert_eq!(
            argv,
            [
                "nix",
                "--extra-experimental-features",
                "nix-command flakes",
                "eval",
                "--raw",
                "nixpkgs#hello"
            ]
        );
    }

    #[test]
    fn legacy_binary_omits_nix_subcommand() {
        let argv = NixCommand::nix_instantiate().arg("--eval").argv();
        assert_eq!(argv, ["nix-instantiate", "--eval"]);
    }

    #[cfg(unix)]
    #[test]
    fn output_captures_both_streams() -> Result<()> {
        let output = NixCommand::with_binary("sh", None)
            .args(["-c", "printf stdout; printf stderr >&2"])
            .output()?;
        assert!(output.success());
        assert_eq!(output.stdout, b"stdout");
        assert_eq!(output.stderr, b"stderr");
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn output_timeout_kills_the_process() {
        let start = Instant::now();
        let result = NixCommand::with_binary("sh", None)
            .args(["-c", "exec sleep 5"])
            .with_timeout(Duration::from_millis(50))
            .output();
        assert!(matches!(result, Err(Error::Timeout { .. })));
        assert!(start.elapsed() < Duration::from_secs(2));
    }
    #[cfg(unix)]
    #[test]
    fn interactive_timeout_kills_the_process() {
        let mut command = NixCommand::with_binary("sh", None)
            .args(["-c", "exec sleep 5"])
            .with_timeout(Duration::from_millis(50));
        command.interactive = true;

        let start = Instant::now();
        let result = command.run_with_logs();
        assert!(matches!(result, Err(Error::Timeout { .. })));
        assert!(start.elapsed() < Duration::from_secs(2));
    }
}
