use std::{
    ffi::{OsStr, OsString},
    io::{self, Write},
    time::{Duration, Instant},
};

use subprocess::{Capture, Exec, ExitStatus, Job, Redirection};

#[derive(Debug, thiserror::Error)]
pub enum NixCmdError {
    #[error("io: {0}")]
    Io(#[from] io::Error),

    #[error("command '{command}' timed out after {duration:?}")]
    Timeout { command: String, duration: Duration },
}

pub type Result<T, E = NixCmdError> = std::result::Result<T, E>;

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
    kind: CommandKind,
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
        Self {
            kind,
            global_args: Vec::new(),
            args: Vec::new(),
            env: Vec::new(),
            print_build_logs: kind.print_build_logs(),
            interactive: kind.interactive(),
            timeout: None,
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
    pub fn env(
        mut self,
        key: impl AsRef<OsStr>,
        value: impl AsRef<OsStr>,
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
        argv.push(OsString::from("nix"));
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
            global_args,
            args,
            env,
            print_build_logs,
            ..
        } = self;
        let args =
            assemble_args(kind, print_build_logs, global_args, args);
        let command = Exec::cmd("nix").args(args);
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
        let timeout = self.timeout_context();
        capture_exec(self.into_exec(), timeout)
    }

    fn run_interactive(self) -> Result<ExitStatus> {
        let timeout = self.timeout_context();
        run_interactive_exec(self.into_exec(), timeout.as_ref())
    }

    fn communicate_to(
        self,
        stdout: &mut dyn Write,
        stderr: &mut dyn Write,
    ) -> Result<ExitStatus> {
        let timeout = self.timeout_context();
        communicate_exec(self.into_exec(), timeout, stdout, stderr)
    }

    fn timeout_context(&self) -> Option<TimeoutContext> {
        self.timeout.map(|duration| TimeoutContext {
            command: format!("nix {}", self.kind.as_str()),
            duration,
        })
    }
}

fn capture_exec(
    exec: Exec,
    timeout: Option<TimeoutContext>,
) -> Result<Capture> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let exit_status =
        communicate_exec(exec, timeout, &mut stdout, &mut stderr)?;
    Ok(Capture {
        stdout,
        stderr,
        exit_status,
    })
}

fn run_interactive_exec(
    exec: Exec,
    timeout: Option<&TimeoutContext>,
) -> Result<ExitStatus> {
    let job = exec.start()?;
    let deadline =
        timeout.map(|timeout| Instant::now() + timeout.duration);
    wait_for_job(&job, timeout, deadline)
}

fn communicate_exec(
    exec: Exec,
    timeout: Option<TimeoutContext>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<ExitStatus> {
    let mut job = exec
        .stdout(Redirection::Pipe)
        .stderr(Redirection::Pipe)
        .start()?;
    let deadline = timeout
        .as_ref()
        .map(|timeout| Instant::now() + timeout.duration);
    let communicator = job.communicate()?;
    let communication = if let Some(deadline) = deadline {
        communicator
            .limit_time(deadline.saturating_duration_since(Instant::now()))
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

fn assemble_args(
    kind: CommandKind,
    print_build_logs: bool,
    mut global_args: Vec<OsString>,
    args: Vec<OsString>,
) -> Vec<OsString> {
    global_args.push(OsString::from(kind.as_str()));
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
    fn into_error(self) -> NixCmdError {
        NixCmdError::Timeout {
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
        return Err(NixCmdError::Timeout {
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
    use std::assert_matches;

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

    #[cfg(unix)]
    #[test]
    fn capture_exec_captures_both_streams() -> Result<()> {
        let output = capture_exec(
            Exec::cmd("sh")
                .args(["-c", "printf stdout; printf stderr >&2"]),
            None,
        )?;
        assert!(output.success());
        assert_eq!(output.stdout, b"stdout");
        assert_eq!(output.stderr, b"stderr");
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn capture_timeout_kills_the_process() {
        let duration = Duration::from_millis(50);
        let start = Instant::now();
        let result = capture_exec(
            Exec::cmd("sh").args(["-c", "exec sleep 5"]),
            Some(TimeoutContext {
                command: "sh".to_owned(),
                duration,
            }),
        );
        assert_matches!(result, Err(NixCmdError::Timeout { .. }));
        assert!(start.elapsed() < Duration::from_secs(2));
    }
    #[cfg(unix)]
    #[test]
    fn interactive_timeout_kills_the_process() {
        let duration = Duration::from_millis(50);
        let timeout = TimeoutContext {
            command: "sh".to_owned(),
            duration,
        };
        let start = Instant::now();
        let result = run_interactive_exec(
            Exec::cmd("sh").args(["-c", "exec sleep 5"]),
            Some(&timeout),
        );
        assert_matches!(result, Err(NixCmdError::Timeout { .. }));
        assert!(start.elapsed() < Duration::from_secs(2));
    }
}
