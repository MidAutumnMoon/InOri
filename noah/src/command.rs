use std::{
    collections::{BTreeMap, BTreeSet},
    convert::Infallible,
    ffi::{OsStr, OsString},
    io::{self, Write},
    path::PathBuf,
    str::FromStr,
};

use nh_installable::Installable;
pub use nix_command::{CommandKind, NixCommand};
use rootcause::{Result, bail, prelude::ResultExt, report};
use subprocess::{Exec, ExitStatus, Redirection};
use thiserror::Error;
use tracing::{debug, info, warn};
use which::which_in;

use crate::{args::NixBuildPassthroughArgs, runtime::RuntimeEnv};

/// Privilege-elevation configuration captured from environment variables.
///
/// Parsed once in `main()` from [`RuntimeEnv`] and passed by reference.
#[derive(Debug, Clone)]
pub struct SudoConfig {
    /// `NH_SUDOOPTS` (preferred) or `NIX_SUDOOPTS` (legacy), shell-split.
    pub opts: Vec<String>,
    /// `NH_SUDO_ASKPASS` — path to askpass helper.
    pub askpass: Option<String>,
    /// `NH_PRESERVE_ENV` — defaults to `true` when unset; `false` when "0".
    pub preserve_env: bool,
}

impl Default for SudoConfig {
    fn default() -> Self {
        Self {
            opts: Vec::new(),
            askpass: None,
            preserve_env: true,
        }
    }
}

impl SudoConfig {
    /// Parse privilege-elevation settings from a startup environment snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error if the selected sudo options contain unmatched shell
    /// quoting.
    pub fn from_env(env: &RuntimeEnv) -> Result<Self> {
        Ok(Self {
            opts: env.shell_words("NH_SUDOOPTS", "NIX_SUDOOPTS")?,
            askpass: env
                .non_empty_var("NH_SUDO_ASKPASS")
                .map(str::to_owned),
            preserve_env: env
                .var("NH_PRESERVE_ENV")
                .is_none_or(|value| value != "0"),
        })
    }
}

struct CaptureWriter<W> {
    stream: W,
    captured: Vec<u8>,
}

impl<W> CaptureWriter<W> {
    fn new(stream: W) -> Self {
        Self {
            stream,
            captured: Vec::new(),
        }
    }

    fn into_string(self) -> String {
        String::from_utf8_lossy(&self.captured).into_owned()
    }
}

impl<W: Write> Write for CaptureWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.stream.write_all(buf)?;
        self.captured.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

/// Run an [`Exec`], draining stdout and stderr concurrently into the supplied
/// writers.
///
/// # Errors
///
/// Returns an error if the command cannot start, communication fails, or the
/// process cannot be reaped.
pub(crate) fn exec_with_writers(
    cmd: Exec,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<subprocess::ExitStatus> {
    let mut job = cmd
        .stdout(Redirection::Pipe)
        .stderr(Redirection::Pipe)
        .start()
        .context("Failed to start command")?;

    let communication = job
        .communicate()
        .context("Failed to open command pipes")?
        .read_to(stdout, stderr);
    if let Err(error) = communication {
        let _ = job.kill();
        let _ = job.wait();
        return Err(error)
            .context("Failed to stream command output")
            .map_err(Into::into);
    }

    job.wait()
        .context("Failed to wait for command completion")
        .map_err(Into::into)
}

/// Run a command while forwarding and retaining both output streams.
///
/// # Errors
///
/// Returns an error if command execution or output streaming fails.
pub(crate) fn exec_with_streaming(
    cmd: Exec,
) -> Result<(subprocess::ExitStatus, String, String)> {
    let mut stdout = CaptureWriter::new(io::stdout());
    let mut stderr = CaptureWriter::new(io::stderr());
    let status = exec_with_writers(cmd, &mut stdout, &mut stderr)?;
    Ok((status, stdout.into_string(), stderr.into_string()))
}

/// Strategy argument for handling privilege elevation when running commands.
///
/// Defines how `nh` should handle privilege elevation for commands
/// that require root access (e.g., `switch-to-configuration`)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElevationStrategyArg {
    /// No elevation - commands run without privilege escalation.
    None,

    /// Automatically detect and use the first available elevation program
    /// (tries doas -> sudo -> run0 -> pkexec in order). Uses askpass helper if
    /// available.
    Auto,

    /// Use elevation program but skip password prompting for remote hosts with
    /// NOPASSWD configured.
    Passwordless,

    /// Use the specified elevation program.
    Program(PathBuf),
}

impl From<&str> for ElevationStrategyArg {
    fn from(value: &str) -> Self {
        match value {
            "none" => Self::None,
            "auto" => Self::Auto,
            "passwordless" => Self::Passwordless,
            _ => Self::Program(PathBuf::from(
                value.strip_prefix("program:").unwrap_or(value),
            )),
        }
    }
}

impl FromStr for ElevationStrategyArg {
    type Err = Infallible;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        Ok(Self::from(value))
    }
}

/// Strategy for handling privilege elevation at runtime.
///
/// This enum defines how `nh` should handle privilege elevation for commands
/// that require root access (e.g., `switch-to-configuration`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElevationStrategy {
    /// Automatically detect and use the first available elevation program
    /// (tries doas -> sudo -> run0 -> pkexec in order). Uses askpass helper if
    /// available.
    Auto,

    /// Try the specified elevation program first, fall back to `Auto` if not
    /// found. Corresponds to CLI argument that is a path.
    Prefer(PathBuf),

    /// Do not use any elevation program. Commands run without privilege
    /// escalation. This will fail for commands requiring root unless the user is
    /// already root or the system has other privilege mechanisms configured.
    None,

    /// Use elevation program but skip password prompting. For remote hosts with
    /// passwordless sudo (NOPASSWD in sudoers) or similar configurations. The
    /// elevation command runs without `--stdin` or password input.
    Passwordless,
}

impl ElevationStrategy {
    /// Resolves the elevation strategy to an actual program path.
    ///
    /// Attempts to find an appropriate privilege elevation program based on the
    /// strategy variant and system availability.
    ///
    /// # Returns
    ///
    /// Returns `Ok(PathBuf)` containing the path to the elevation program binary.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    ///
    /// - `None` variant: Always fails (elevation is disabled via
    ///   `--elevation-strategy=none`)
    /// - Other variants: No suitable elevation programs are available on the
    ///   system
    pub fn resolve(&self, runtime_env: &RuntimeEnv) -> Result<PathBuf> {
        match self {
            Self::Auto | Self::Passwordless => Self::choice(runtime_env),
            Self::Prefer(program) => {
                Self::find(program, runtime_env).or_else(|_| {
                    warn!(
                        ?program,
                        "Preferred elevation program not found, falling back to \
                         auto-detection"
                    );
                    Self::choice(runtime_env)
                })
            }
            Self::None => {
                bail!("Elevation disabled via --elevation-strategy=none")
            }
        }
    }

    /// Gets a path to a privilege elevation program based on what is available in
    /// the system.
    ///
    /// This funtion checks for the existence of common privilege elevation
    /// program names in the `PATH` using the `which` crate and returns a Ok
    /// result with the `OsString` of the path to the binary. In the case none
    /// of the checked programs are found a Err result is returned.
    ///
    /// The search is done in this order:
    ///
    /// 1. `doas`
    /// 2. `sudo`
    /// 3. `run0`
    /// 4. `pkexec`
    ///
    /// The logic for choosing this order is that a person with `doas` installed
    /// is more likely to be using it as their main privilege elevation program.
    /// `run0` and `pkexec` are preinstalled in any `NixOS` system with polkit
    /// support installed, so they have been placed lower as it's easier to
    /// deactivate sudo than it is to remove `run0`/`pkexec`
    ///
    /// # Returns
    ///
    /// * `Result<PathBuf>` - The absolute path to the privilege elevation program
    ///   binary or an error if a program can't be found.
    fn choice(runtime_env: &RuntimeEnv) -> Result<PathBuf> {
        const STRATEGIES: [&str; 4] = ["doas", "sudo", "run0", "pkexec"];

        for strategy in STRATEGIES {
            if let Ok(path) = Self::find(strategy, runtime_env) {
                debug!(?path, "{strategy} path found");
                return Ok(path);
            }
        }

        Err(report!(
            "No elevation strategy found. Checked: {}",
            STRATEGIES.join(", ")
        ))
    }

    fn find(
        program: impl AsRef<OsStr>,
        runtime_env: &RuntimeEnv,
    ) -> which::Result<PathBuf> {
        let path = runtime_env.var_os("PATH").unwrap_or_default();
        which_in(program, Some(path), runtime_env.current_dir())
    }
}

struct ElevationParts<'a> {
    program: PathBuf,
    args: Vec<OsString>,
    askpass: Option<&'a str>,
}

#[derive(Debug)]
pub struct Command<'env> {
    dry: bool,
    message: Option<String>,
    program: OsString,
    args: Vec<OsString>,
    elevate: Option<ElevationStrategy>,
    show_output: bool,
    preserved_env: BTreeSet<String>,
    env_overrides: BTreeMap<String, String>,
    runtime_env: &'env RuntimeEnv,
    sudo_config: &'env SudoConfig,
}

impl<'env> Command<'env> {
    pub fn new<S: AsRef<OsStr>>(
        command: S,
        runtime_env: &'env RuntimeEnv,
        sudo_config: &'env SudoConfig,
    ) -> Self {
        Self {
            dry: false,
            message: None,
            program: command.as_ref().to_os_string(),
            args: vec![],
            elevate: None,
            show_output: false,
            preserved_env: BTreeSet::new(),
            env_overrides: BTreeMap::new(),
            runtime_env,
            sudo_config,
        }
    }

    /// Set whether to run the command with elevated privileges.
    #[must_use]
    pub fn elevate(mut self, elevate: Option<ElevationStrategy>) -> Self {
        self.elevate = elevate;
        self
    }

    /// Set whether to perform a dry run.
    #[must_use]
    pub const fn dry(mut self, dry: bool) -> Self {
        self.dry = dry;
        self
    }

    /// Set whether to show command output.
    #[must_use]
    pub const fn show_output(mut self, show_output: bool) -> Self {
        self.show_output = show_output;
        self
    }

    /// Add a single argument to the command.
    #[must_use]
    pub fn arg<S: AsRef<OsStr>>(mut self, arg: S) -> Self {
        self.args.push(arg.as_ref().to_os_string());
        self
    }

    /// Add multiple arguments to the command.
    #[must_use]
    pub fn args<I>(mut self, args: I) -> Self
    where
        I: IntoIterator,
        I::Item: AsRef<OsStr>,
    {
        for elem in args {
            self.args.push(elem.as_ref().to_os_string());
        }
        self
    }

    /// Set a message to display before running the command.
    #[must_use]
    pub fn message<S: AsRef<str>>(mut self, message: S) -> Self {
        self.message = Some(message.as_ref().to_string());
        self
    }

    /// Preserve captured values for the named variables.
    #[must_use]
    pub fn preserve_envs<I, K>(mut self, keys: I) -> Self
    where
        I: IntoIterator<Item = K>,
        K: AsRef<str>,
    {
        self.preserved_env
            .extend(keys.into_iter().map(|key| key.as_ref().to_owned()));
        self
    }

    /// Override an environment variable for the child command.
    #[must_use]
    pub fn set_env<K, V>(mut self, key: K, value: V) -> Self
    where
        K: AsRef<str>,
        V: AsRef<str>,
    {
        self.env_overrides
            .insert(key.as_ref().to_owned(), value.as_ref().to_owned());
        self
    }

    fn environment<'a>(
        &'a self,
        elevated: bool,
    ) -> Vec<(&'a str, &'a str)> {
        let mut vars: Vec<(&'a str, &'a str)> =
            Vec::with_capacity(self.preserved_env.len() + 10);

        if elevated {
            if let Some(user) = self.runtime_env.var("USER") {
                vars.push(("USER", user));
            }
            if self.sudo_config.preserve_env {
                for (key, value) in self.runtime_env.nix_child_env() {
                    vars.push((key, value));
                }
            }
        } else {
            for (key, value) in self.runtime_env.child_env() {
                vars.push((key, value));
            }
        }

        if !elevated || self.sudo_config.preserve_env {
            for key in &self.preserved_env {
                if let Some(value) = self.runtime_env.var(key) {
                    vars.push((key, value));
                }
            }
        }

        vars.extend(
            self.env_overrides
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_str())),
        );
        vars
    }

    fn apply_env_to_exec(&self, mut cmd: Exec) -> Exec {
        for (key, value) in self.environment(false) {
            cmd = cmd.env(key, value);
        }
        cmd
    }

    fn elevation_parts(&self) -> Result<ElevationParts<'_>> {
        let strategy = self.elevate.as_ref().ok_or_else(|| {
            report!("Command is not configured for elevation")
        })?;
        let program = strategy
            .resolve(self.runtime_env)
            .context("Failed to resolve elevation program")?;
        let program_name = program
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                report!("Failed to determine elevation program name")
            })?;
        let passwordless =
            matches!(strategy, ElevationStrategy::Passwordless);
        let mut args = Vec::new();
        let mut askpass = None;

        if program_name == "sudo" {
            if !passwordless
                && let Some(configured_askpass) =
                    self.sudo_config.askpass.as_deref()
            {
                args.push(OsString::from("-A"));
                askpass = Some(configured_askpass);
            }
            args.extend(self.sudo_config.opts.iter().map(OsString::from));
        } else if program_name == "run0" {
            // Without a private PTY, run0 can transfer ownership of the
            // caller's terminal to root.
            args.push(OsString::from("--pty-late"));
        }

        args.push(OsString::from("env"));
        args.extend(
            self.environment(true).into_iter().map(|(key, value)| {
                OsString::from(format!("{key}={value}"))
            }),
        );

        Ok(ElevationParts {
            program,
            args,
            askpass,
        })
    }

    fn build_sudo_cmd(&self) -> Result<Exec> {
        let parts = self.elevation_parts()?;
        let mut command = Exec::cmd(parts.program).args(parts.args);
        if let Some(askpass) = parts.askpass {
            command = command.env("SUDO_ASKPASS", askpass);
        }
        Ok(command)
    }

    /// Build the command used to replace this process with an elevated copy.
    ///
    /// # Errors
    ///
    /// Returns an error if the elevation program cannot be resolved.
    pub fn self_elevate_cmd(
        strategy: ElevationStrategy,
        runtime_env: &'env RuntimeEnv,
        sudo_config: &'env SudoConfig,
    ) -> Result<std::process::Command> {
        let builder =
            Self::new(runtime_env.executable(), runtime_env, sudo_config)
                .elevate(Some(strategy))
                // `clean all` is the only self-elevating path and NH_ASK is its only
                // environment-backed option not already represented in argv.
                .preserve_envs(["NH_ASK"]);
        let parts = builder.elevation_parts()?;
        let mut command = std::process::Command::new(parts.program);
        command
            .args(parts.args)
            .arg(runtime_env.executable())
            .args(runtime_env.arguments());
        if let Some(askpass) = parts.askpass {
            command.env("SUDO_ASKPASS", askpass);
        }
        Ok(command)
    }

    /// Run the configured command.
    ///
    /// # Errors
    ///
    /// Returns an error if the command fails to execute or returns a non-zero
    /// exit status.
    pub fn run(&self) -> Result<()> {
        let cmd = if self.elevate.is_some() {
            self.build_sudo_cmd()?.arg(&self.program).args(&self.args)
        } else {
            self.apply_env_to_exec(
                Exec::cmd(&self.program).args(&self.args),
            )
        };

        let cmd = if self.show_output {
            cmd.stderr(Redirection::Merge)
        } else {
            cmd.stderr(Redirection::None).stdout(Redirection::None)
        };

        if let Some(m) = &self.message {
            info!("{m}");
        }

        debug!(?cmd);

        if self.dry {
            return Ok(());
        }

        let msg = self
            .message
            .clone()
            .unwrap_or_else(|| "Command failed".to_string());

        if self.show_output {
            let exit_status = cmd.join().context(msg.clone())?;
            if !exit_status.success() {
                bail!(format!("{} (exit status {:?})", msg, exit_status));
            }
            Ok(())
        } else {
            let res = cmd.capture();
            match res {
                Ok(capture) => {
                    let status = &capture.exit_status;
                    if !status.success() {
                        let stderr = capture.stderr_str();
                        if stderr.trim().is_empty() {
                            bail!(format!(
                                "{} (exit status {:?})",
                                msg, status
                            ));
                        }
                        bail!(format!(
                            "{} (exit status {:?})\nstderr:\n{}",
                            msg, status, stderr
                        ));
                    }
                    Ok(())
                }
                Err(e) => Err(e).context(msg).map_err(Into::into),
            }
        }
    }
}

#[derive(Debug)]
pub struct Build {
    message: Option<String>,
    installable: Installable,
    extra_args: Vec<OsString>,
    nom: bool,
}

impl Build {
    #[must_use]
    pub const fn new(installable: Installable) -> Self {
        Self {
            message: None,
            installable,
            extra_args: vec![],
            nom: false,
        }
    }

    #[must_use]
    pub fn message<S: AsRef<str>>(mut self, message: S) -> Self {
        self.message = Some(message.as_ref().to_string());
        self
    }

    #[must_use]
    pub fn extra_arg<S: AsRef<OsStr>>(mut self, arg: S) -> Self {
        self.extra_args.push(arg.as_ref().to_os_string());
        self
    }

    #[must_use]
    pub const fn nom(mut self, yes: bool) -> Self {
        self.nom = yes;
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
    pub fn passthrough(
        self,
        passthrough: &NixBuildPassthroughArgs,
    ) -> Self {
        self.extra_args(passthrough.generate_passthrough_args())
    }

    /// Run the build command.
    ///
    /// # Errors
    ///
    /// Returns an error if the build command fails to execute.
    pub fn run(&self) -> Result<()> {
        if let Some(m) = &self.message {
            info!("{m}");
        }

        let installable_args = self.installable.to_args();

        let base_command = NixCommand::new(CommandKind::Build)
            .print_build_logs(false)
            .args(&installable_args)
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

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::unwrap_used,
        reason = "Fine in tests"
    )]

    use std::collections::BTreeMap;

    use super::*;

    fn environment(
        command: &Command<'_>,
        elevated: bool,
    ) -> BTreeMap<String, String> {
        command
            .environment(elevated)
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect()
    }

    #[test]
    fn child_environment_is_selected_from_the_startup_snapshot() {
        let runtime = RuntimeEnv::from_pairs([
            ("USER", "teapot"),
            ("HOME", "/home/teapot"),
            ("PATH", "/run/current-system/sw/bin"),
            ("NIX_CONFIG", "experimental-features = nix-command flakes"),
            ("NH_FLAKE", "/configuration"),
        ]);
        let sudo = SudoConfig::default();
        let command = Command::new("true", &runtime, &sudo);

        let env = environment(&command, false);
        assert_eq!(env.get("USER").map(String::as_str), Some("teapot"));
        assert_eq!(
            env.get("HOME").map(String::as_str),
            Some("/home/teapot")
        );
        assert_eq!(
            env.get("PATH").map(String::as_str),
            Some("/run/current-system/sw/bin")
        );
        assert!(env.contains_key("NIX_CONFIG"));
        assert!(!env.contains_key("NH_FLAKE"));
    }

    #[test]
    fn elevated_environment_honors_preservation_policy() {
        let runtime = RuntimeEnv::from_pairs([
            ("USER", "teapot"),
            ("HOME", "/home/teapot"),
            ("PATH", "/bin"),
            ("NIXOS_NO_CHECK", "1"),
        ]);
        let sudo = SudoConfig {
            preserve_env: false,
            ..SudoConfig::default()
        };
        let command = Command::new("true", &runtime, &sudo)
            .preserve_envs(["NIXOS_NO_CHECK"])
            .set_env("NIXOS_INSTALL_BOOTLOADER", "1");

        let env = environment(&command, true);
        assert_eq!(env.get("USER").map(String::as_str), Some("teapot"));
        assert!(!env.contains_key("HOME"));
        assert!(!env.contains_key("PATH"));
        assert!(!env.contains_key("NIXOS_NO_CHECK"));
        assert_eq!(
            env.get("NIXOS_INSTALL_BOOTLOADER").map(String::as_str),
            Some("1")
        );
    }

    #[test]
    fn explicit_environment_override_wins_over_captured_value() {
        let runtime = RuntimeEnv::from_pairs([("NIXOS_NO_CHECK", "0")]);
        let sudo = SudoConfig::default();
        let command = Command::new("true", &runtime, &sudo)
            .preserve_envs(["NIXOS_NO_CHECK"])
            .set_env("NIXOS_NO_CHECK", "1");

        let env = environment(&command, true);
        assert_eq!(
            env.get("NIXOS_NO_CHECK").map(String::as_str),
            Some("1")
        );
    }

    #[test]
    fn sudo_configuration_uses_preferred_shell_words() {
        let runtime = RuntimeEnv::from_pairs([
            ("NH_SUDOOPTS", "--preserve-env='NIX_CONFIG NIX_PATH'"),
            ("NIX_SUDOOPTS", "--legacy"),
            ("NH_SUDO_ASKPASS", "/bin/askpass"),
            ("NH_PRESERVE_ENV", "0"),
        ]);

        let config = SudoConfig::from_env(&runtime).unwrap();
        assert_eq!(config.opts, ["--preserve-env=NIX_CONFIG NIX_PATH"]);
        assert_eq!(config.askpass.as_deref(), Some("/bin/askpass"));
        assert!(!config.preserve_env);
    }

    #[cfg(unix)]
    #[test]
    fn exec_with_writers_drains_both_output_streams() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status = exec_with_writers(
            Exec::cmd("sh")
                .args(["-c", "printf stdout; printf stderr >&2"]),
            &mut stdout,
            &mut stderr,
        )
        .unwrap();

        assert!(status.success());
        assert_eq!(stdout, b"stdout");
        assert_eq!(stderr, b"stderr");
    }

    #[cfg(unix)]
    #[test]
    fn command_failure_includes_captured_stderr() {
        let runtime = RuntimeEnv::from_pairs([("IGNORED", "")]);
        let sudo = SudoConfig::default();
        let error = Command::new("/bin/sh", &runtime, &sudo)
            .args(["-c", "printf failure >&2; exit 7"])
            .run()
            .unwrap_err();

        let message = error.to_string();
        assert!(message.contains("failure"));
        assert!(message.contains("exit status"));
    }

    #[test]
    fn elevation_program_prefix_is_parsed() {
        let parsed = "program:/path/to/bin"
            .parse::<ElevationStrategyArg>()
            .unwrap();
        assert_eq!(
            parsed,
            ElevationStrategyArg::Program(PathBuf::from("/path/to/bin"))
        );
    }
}
