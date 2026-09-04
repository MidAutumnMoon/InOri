use std::{
    cell::OnceCell,
    collections::{BTreeMap, BTreeSet},
    convert::Infallible,
    ffi::{OsStr, OsString},
    os::unix::process::CommandExt as _,
    path::PathBuf,
    str::FromStr,
};

use crate::nix_command::CommandKind;
use crate::nix_command::NixCommand;
use rootcause::{Result, bail, prelude::ResultExt as _, report};
use subprocess::{Exec, ExitStatus, Redirection};
use thiserror::Error;
use tracing::{debug, info, warn};
use which::which_in;

use crate::nix_options::NixBuildOptions;
use crate::runtime::Env;
use crate::target::BuildTarget;

/// Strategy for handling privilege elevation when running commands.
///
/// Defines how `nh` should handle privilege elevation for commands
/// that require root access (e.g., `switch-to-configuration`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElevationStrategy {
    /// Do not use any elevation program. Commands run without privilege
    /// escalation. This will fail for commands requiring root unless the user
    /// is already root or the system has other privilege mechanisms
    /// configured.
    None,

    /// Automatically detect and use the first available elevation program
    /// (tries doas -> sudo -> run0 -> pkexec in order). Uses askpass helper if
    /// available.
    Auto,

    /// Use elevation program but skip password prompting. For remote hosts
    /// with passwordless sudo (NOPASSWD in sudoers) or similar
    /// configurations. The elevation command runs without `--stdin` or
    /// password input.
    Passwordless,

    /// Use the specified elevation program, falling back to `Auto` detection
    /// when it is not found.
    Program(PathBuf),
}

impl From<&str> for ElevationStrategy {
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

impl FromStr for ElevationStrategy {
    type Err = Infallible;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        Ok(Self::from(value))
    }
}

/// Privilege-elevation policy for a run, captured once at startup.
///
/// Combines the chosen [`ElevationStrategy`] with the sudo-related
/// environment (`NH_SUDOOPTS`, `NH_SUDO_ASKPASS`, `NH_PRESERVE_ENV`). The
/// elevation program is resolved against `PATH` on first use and cached, so
/// a single run consistently uses one program.
#[derive(Debug)]
pub struct Elevation {
    strategy: ElevationStrategy,
    /// `NH_SUDOOPTS` (preferred) or `NIX_SUDOOPTS` (legacy), shell-split.
    opts: Vec<String>,
    /// `NH_SUDO_ASKPASS` — path to askpass helper.
    askpass: Option<String>,
    /// `NH_PRESERVE_ENV` — defaults to `true` when unset; `false` when "0".
    preserve_env: bool,
    /// Resolved elevation program, filled on first use.
    program: OnceCell<PathBuf>,
}

impl Elevation {
    /// Build the elevation policy from the CLI strategy and a startup
    /// environment snapshot.
    ///
    /// Falls back to the deprecated `NH_ELEVATION_PROGRAM` variable when no
    /// strategy was given.
    ///
    /// # Errors
    ///
    /// Returns an error if the selected sudo options contain unmatched shell
    /// quoting.
    pub fn new(
        strategy: Option<ElevationStrategy>,
        env: &Env,
    ) -> Result<Self> {
        let strategy = Self::chosen(strategy, env);

        Ok(Self {
            strategy,
            opts: env.shell_words("NH_SUDOOPTS", "NIX_SUDOOPTS")?,
            askpass: env
                .non_empty_var("NH_SUDO_ASKPASS")
                .map(str::to_owned),
            // NH_PRESERVE_ENV keeps upstream's legacy semantics where any
            // value except "0" enables it, so `false` enables it too.
            preserve_env: env
                .var("NH_PRESERVE_ENV")
                .is_none_or(|value| value != "0"),
            program: OnceCell::new(),
        })
    }

    /// Resolve the CLI strategy to a policy, falling back to the deprecated
    /// `NH_ELEVATION_PROGRAM` variable, then to auto-detection.
    fn chosen(
        strategy: Option<ElevationStrategy>,
        env: &Env,
    ) -> ElevationStrategy {
        if let Some(strategy) = strategy {
            return strategy;
        }

        env.non_empty_var("NH_ELEVATION_PROGRAM").map_or(
            ElevationStrategy::Auto,
            |old_value| {
                // TODO: Remove this fallback in a future version
                warn!(
                    "NH_ELEVATION_PROGRAM is deprecated, use \
                     --elevation-strategy instead. Falling back to \
                     NH_ELEVATION_PROGRAM for backward compatibility. \
                     Accepted values: none, passwordless, program:<path>"
                );
                ElevationStrategy::from(old_value)
            },
        )
    }

    /// Whether elevation is disabled via `--elevation-strategy=none`.
    #[must_use]
    pub const fn is_disabled(&self) -> bool {
        matches!(self.strategy, ElevationStrategy::None)
    }

    /// Whether the elevation program should run without password prompting.
    #[must_use]
    pub const fn is_passwordless(&self) -> bool {
        matches!(self.strategy, ElevationStrategy::Passwordless)
    }

    /// Path to the elevation program, resolved on first use and cached for
    /// the rest of the process.
    ///
    /// # Errors
    ///
    /// Returns an error if the strategy is `None` or no elevation program is
    /// installed.
    pub fn program(&self, env: &Env) -> Result<PathBuf> {
        if let Some(path) = self.program.get() {
            return Ok(path.clone());
        }

        let path = self.resolve(env)?;
        if let Err(first) = self.program.set(path.clone()) {
            // Another resolution already stored a program; keep the first one.
            return Ok(first);
        }
        Ok(path)
    }

    /// Resolve the strategy to an actual elevation program path.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    ///
    /// - [`ElevationStrategy::None`]: elevation is disabled via
    ///   `--elevation-strategy=none`
    /// - Other variants: no suitable elevation programs are available on the
    ///   system
    fn resolve(&self, env: &Env) -> Result<PathBuf> {
        match &self.strategy {
            ElevationStrategy::Auto | ElevationStrategy::Passwordless => {
                Self::choice(env)
            }
            ElevationStrategy::Program(program) => Self::find(
                program, env,
            )
            .or_else(|_| {
                warn!(
                    ?program,
                    "Preferred elevation program not found, falling back \
                         to auto-detection"
                );
                Self::choice(env)
            }),
            ElevationStrategy::None => {
                bail!("Elevation disabled via --elevation-strategy=none")
            }
        }
    }

    /// Gets a path to a privilege elevation program based on what is available
    /// in the system.
    ///
    /// This function checks for the existence of common privilege elevation
    /// program names in the `PATH` using the `which` crate and returns a Ok
    /// result with the path to the binary. In the case none of the checked
    /// programs are found a Err result is returned.
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
    /// deactivate sudo than it is to remove `run0`/`pkexec`.
    ///
    /// # Errors
    ///
    /// Returns an error if no elevation program can be found.
    fn choice(env: &Env) -> Result<PathBuf> {
        const STRATEGIES: [&str; 4] = ["doas", "sudo", "run0", "pkexec"];

        for strategy in STRATEGIES {
            if let Ok(path) = Self::find(strategy, env) {
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
        env: &Env,
    ) -> which::Result<PathBuf> {
        let path = env.var_os("PATH").unwrap_or_default();
        which_in(program, Some(path), env.current_dir())
    }
}

struct ElevationParts<'ctx> {
    program: PathBuf,
    args: Vec<OsString>,
    askpass: Option<&'ctx str>,
}

#[derive(Debug)]
pub struct Command<'env> {
    dry: bool,
    message: Option<String>,
    program: OsString,
    args: Vec<OsString>,
    elevate: bool,
    show_output: bool,
    preserved_env: BTreeSet<String>,
    env_overrides: BTreeMap<String, String>,
    env: &'env Env,
    elevation: &'env Elevation,
}

impl<'env> Command<'env> {
    pub fn new(
        command: impl AsRef<OsStr>,
        env: &'env Env,
        elevation: &'env Elevation,
    ) -> Self {
        Self {
            dry: false,
            message: None,
            program: command.as_ref().to_os_string(),
            args: vec![],
            elevate: false,
            show_output: false,
            preserved_env: BTreeSet::new(),
            env_overrides: BTreeMap::new(),
            env,
            elevation,
        }
    }

    /// Set whether to run the command with elevated privileges.
    #[must_use]
    pub const fn elevate(mut self, elevate: bool) -> Self {
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
    pub fn arg(mut self, arg: impl AsRef<OsStr>) -> Self {
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
    pub fn message(mut self, message: impl AsRef<str>) -> Self {
        self.message = Some(message.as_ref().to_owned());
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

    fn environment<'ctx>(
        &'ctx self,
        elevated: bool,
    ) -> Vec<(&'ctx str, &'ctx str)> {
        let mut vars: Vec<(&'ctx str, &'ctx str)> =
            Vec::with_capacity(self.preserved_env.len() + 10);

        if elevated {
            if let Some(user) = self.env.var("USER") {
                vars.push(("USER", user));
            }
            if self.elevation.preserve_env {
                for (key, value) in self.env.nix_child_env() {
                    vars.push((key, value));
                }
            }
        } else {
            for (key, value) in self.env.child_env() {
                vars.push((key, value));
            }
        }

        if !elevated || self.elevation.preserve_env {
            for key in &self.preserved_env {
                if let Some(value) = self.env.var(key) {
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
        let program = self
            .elevation
            .program(self.env)
            .context("Failed to resolve elevation program")?;
        let program_name = program
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                report!("Failed to determine elevation program name")
            })?;
        let passwordless = self.elevation.is_passwordless();
        let mut args = Vec::new();
        let mut askpass = None;

        if program_name == "sudo" {
            if !passwordless
                && let Some(configured_askpass) =
                    self.elevation.askpass.as_deref()
            {
                args.push(OsString::from("-A"));
                askpass = Some(configured_askpass);
            }
            args.extend(self.elevation.opts.iter().map(OsString::from));
        } else if program_name == "run0" {
            // Without a private PTY, run0 can transfer ownership of the
            // caller's terminal to root.
            args.push(OsString::from("--pty-late"));
        } else {
            // no care
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
        elevation: &'env Elevation,
        env: &'env Env,
    ) -> Result<std::process::Command> {
        let builder = Self::new(env.executable(), env, elevation)
            .elevate(true);
        let parts = builder.elevation_parts()?;
        let mut command = std::process::Command::new(parts.program);
        command
            .args(parts.args)
            .arg(env.executable())
            .args(env.arguments());
        if let Some(askpass) = parts.askpass {
            command.env("SUDO_ASKPASS", askpass);
        }
        Ok(command)
    }

    /// Self-elevates the current process by re-executing it with the
    /// configured elevation program.
    ///
    /// # Panics
    ///
    /// Panics if the process re-execution with elevated privileges fails.
    #[expect(
        clippy::panic,
        clippy::expect_used,
        reason = "re-exec failure is fatal; the `# Panics` section documents the contract"
    )]
    pub fn self_elevate(elevation: &'env Elevation, env: &'env Env) -> ! {
        let mut cmd = Self::self_elevate_cmd(elevation, env)
            .expect("Failed to create self-elevation command");
        debug!("{cmd:?}");

        let err = cmd.exec();
        panic!("{err}");
    }

    /// Run the configured command.
    ///
    /// # Errors
    ///
    /// Returns an error if the command fails to execute or returns a non-zero
    /// exit status.
    pub fn run(&self) -> Result<()> {
        let cmd = if self.elevate {
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

        if let Some(message) = &self.message {
            info!("{message}");
        }

        debug!(?cmd);

        if self.dry {
            return Ok(());
        }

        let msg = self
            .message
            .clone()
            .unwrap_or_else(|| "Command failed".to_owned());

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
                Err(err) => Err(err).context(msg).map_err(Into::into),
            }
        }
    }
}

#[derive(Debug)]
pub struct Build {
    message: Option<String>,
    target: BuildTarget,
    extra_args: Vec<OsString>,
    nom: bool,
}

impl Build {
    #[must_use]
    pub const fn new(target: BuildTarget) -> Self {
        Self {
            message: None,
            target,
            extra_args: vec![],
            nom: false,
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

        let base_command = NixCommand::new(CommandKind::Build)
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
        let env = Env::from_pairs([
            ("USER", "teapot"),
            ("HOME", "/home/teapot"),
            ("PATH", "/run/current-system/sw/bin"),
            ("NIX_CONFIG", "experimental-features = nix-command flakes"),
            ("NH_FLAKE", "/configuration"),
        ]);
        let elevation =
            Elevation::new(Some(ElevationStrategy::Auto), &env).unwrap();
        let command = Command::new("true", &env, &elevation);

        let vars = environment(&command, false);
        assert_eq!(vars.get("USER").map(String::as_str), Some("teapot"));
        assert_eq!(
            vars.get("HOME").map(String::as_str),
            Some("/home/teapot")
        );
        assert_eq!(
            vars.get("PATH").map(String::as_str),
            Some("/run/current-system/sw/bin")
        );
        assert!(vars.contains_key("NIX_CONFIG"));
        assert!(!vars.contains_key("NH_FLAKE"));
    }

    #[test]
    fn elevated_environment_honors_preservation_policy() {
        let env = Env::from_pairs([
            ("USER", "teapot"),
            ("HOME", "/home/teapot"),
            ("PATH", "/bin"),
            ("NIXOS_NO_CHECK", "1"),
            ("NH_PRESERVE_ENV", "0"),
        ]);
        let elevation =
            Elevation::new(Some(ElevationStrategy::Auto), &env).unwrap();
        let command = Command::new("true", &env, &elevation)
            .preserve_envs(["NIXOS_NO_CHECK"])
            .set_env("NIXOS_INSTALL_BOOTLOADER", "1");

        let vars = environment(&command, true);
        assert_eq!(vars.get("USER").map(String::as_str), Some("teapot"));
        assert!(!vars.contains_key("HOME"));
        assert!(!vars.contains_key("PATH"));
        assert!(!vars.contains_key("NIXOS_NO_CHECK"));
        assert_eq!(
            vars.get("NIXOS_INSTALL_BOOTLOADER").map(String::as_str),
            Some("1")
        );
    }

    #[test]
    fn explicit_environment_override_wins_over_captured_value() {
        let env = Env::from_pairs([("NIXOS_NO_CHECK", "0")]);
        let elevation =
            Elevation::new(Some(ElevationStrategy::Auto), &env).unwrap();
        let command = Command::new("true", &env, &elevation)
            .preserve_envs(["NIXOS_NO_CHECK"])
            .set_env("NIXOS_NO_CHECK", "1");

        let vars = environment(&command, true);
        assert_eq!(
            vars.get("NIXOS_NO_CHECK").map(String::as_str),
            Some("1")
        );
    }

    #[test]
    fn elevation_configuration_uses_preferred_shell_words() {
        let env = Env::from_pairs([
            ("NH_SUDOOPTS", "--preserve-env='NIX_CONFIG NIX_PATH'"),
            ("NIX_SUDOOPTS", "--legacy"),
            ("NH_SUDO_ASKPASS", "/bin/askpass"),
            ("NH_PRESERVE_ENV", "0"),
        ]);

        let elevation =
            Elevation::new(Some(ElevationStrategy::Auto), &env).unwrap();
        assert_eq!(
            elevation.opts,
            ["--preserve-env=NIX_CONFIG NIX_PATH"]
        );
        assert_eq!(elevation.askpass.as_deref(), Some("/bin/askpass"));
        assert!(!elevation.preserve_env);
    }

    #[test]
    fn disabled_elevation_fails_to_resolve() {
        let env = Env::from_pairs([("PATH", "/nonexistent")]);
        let elevation =
            Elevation::new(Some(ElevationStrategy::None), &env).unwrap();

        let error = elevation.program(&env).unwrap_err();
        assert!(error.to_string().contains("Elevation disabled"));
    }

    #[test]
    fn deprecated_env_var_fallback_selects_strategy() {
        let env = Env::from_pairs([("NH_ELEVATION_PROGRAM", "none")]);
        let elevation = Elevation::new(None, &env).unwrap();

        assert!(elevation.is_disabled());
    }

    #[cfg(unix)]
    #[test]
    fn command_failure_includes_captured_stderr() {
        let env = Env::from_pairs([("IGNORED", "")]);
        let elevation =
            Elevation::new(Some(ElevationStrategy::Auto), &env).unwrap();
        let error = Command::new("/bin/sh", &env, &elevation)
            .args(["-c", "printf failure >&2; exit 7"])
            .run()
            .unwrap_err();

        let message = error.to_string();
        assert!(message.contains("failure"));
        assert!(message.contains("exit status"));
    }

    #[test]
    fn elevation_program_prefix_is_parsed() {
        let parsed =
            "program:/path/to/bin".parse::<ElevationStrategy>().unwrap();
        assert_eq!(
            parsed,
            ElevationStrategy::Program(PathBuf::from("/path/to/bin"))
        );
    }
}
