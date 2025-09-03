use std::collections::HashMap;
use std::ffi::{OsStr, OsString};

use color_eyre::{
    Result,
    eyre::{self, Context, bail},
};
use subprocess::{Exec, ExitStatus, Redirection};
use thiserror::Error;
use tracing::{debug, info};

use crate::nixos::NixBuildPassthroughArgs;

fn ssh_wrap(cmd: Exec, ssh: Option<&str>) -> Exec {
    if let Some(ssh) = ssh {
        Exec::cmd("ssh")
            .arg("-T")
            .arg(ssh)
            .stdin(cmd.to_cmdline_lossy().as_str())
    } else {
        cmd
    }
}

#[allow(dead_code)] // shut up
#[derive(Debug, Clone)]
pub enum EnvAction {
    /// Set an environment variable to a specific value
    Set(String),

    /// Preserve an environment variable from the current environment
    Preserve,

    /// Remove/unset an environment variable
    Remove,
}

#[derive(Debug)]
pub struct Command {
    dry: bool,
    message: Option<String>,
    command: OsString,
    args: Vec<OsString>,
    elevate: bool,
    ssh: Option<String>,
    show_output: bool,
    env_vars: HashMap<String, EnvAction>,
}

impl Command {
    pub fn new<S: AsRef<OsStr>>(command: S) -> Self {
        Self {
            dry: false,
            message: None,
            command: command.as_ref().to_os_string(),
            args: vec![],
            elevate: false,
            ssh: None,
            show_output: false,
            env_vars: HashMap::new(),
        }
    }

    /// Set whether to run the command with elevated privileges.
    #[must_use]
    pub fn elevate(mut self, elevate: bool) -> Self {
        self.elevate = elevate;
        self
    }

    /// Set whether to perform a dry run.
    #[must_use]
    pub fn dry(mut self, dry: bool) -> Self {
        self.dry = dry;
        self
    }

    /// Set whether to show command output.
    #[must_use]
    pub fn show_output(mut self, show_output: bool) -> Self {
        self.show_output = show_output;
        self
    }

    /// Set the SSH target for remote command execution.
    #[must_use]
    pub fn ssh(mut self, ssh: Option<String>) -> Self {
        self.ssh = ssh;
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

    /// Preserve multiple environment variables from the current environment
    #[must_use]
    pub fn preserve_envs<I, K>(mut self, keys: I) -> Self
    where
        I: IntoIterator<Item = K>,
        K: AsRef<str>,
    {
        for key in keys {
            let key_str = key.as_ref().to_string();
            self.env_vars.insert(key_str, EnvAction::Preserve);
        }
        self
    }

    /// Configure environment for Nix and NH operations
    #[must_use]
    pub fn with_required_env(mut self) -> Self {
        // Centralized list of environment variables to preserve
        // This is not a part of Nix's environment, but it might be necessary.
        // nixos-rebuild preserves it, so we do too.
        const PRESERVE_ENV: &[&str] = &[
            "LOCALE_ARCHIVE",
            // PATH needs to be preserved so that NH can invoke CLI utilities.
            "PATH",
            // Make sure NIX_SSHOPTS applies to nix commands that invoke ssh, such as `nix copy`
            "NIX_SSHOPTS",
            // This is relevant for Home-Manager systems
            "HOME_MANAGER_BACKUP_EXT",
            // Preserve other Nix-related environment variables
            // TODO: is this everything we need? Previously we only preserved *some* variables
            // and nh continued to work, but any missing vars might break functionality completely
            // unexpectedly. This list could change at any moment. This better be enough. Ugh.
            "NIX_CONFIG",
            "NIX_PATH",
            "NIX_REMOTE",
            "NIX_SSL_CERT_FILE",
            "NIX_USER_CONF_FILES",
        ];

        // Always explicitly set USER if present
        if let Ok(user) = std::env::var("USER") {
            self.env_vars
                .insert("USER".to_string(), EnvAction::Set(user));
        }

        // Only propagate HOME for non-elevated commands
        if !self.elevate
            && let Ok(home) = std::env::var("HOME")
        {
            self.env_vars
                .insert("HOME".to_string(), EnvAction::Set(home));
        }

        // Preserve all variables in PRESERVE_ENV if present
        for &key in PRESERVE_ENV {
            if std::env::var(key).is_ok() {
                self.env_vars.insert(key.to_string(), EnvAction::Preserve);
            }
        }

        // Explicitly set NH_* variables
        for (key, value) in std::env::vars() {
            if key.starts_with("NH_") {
                self.env_vars.insert(key, EnvAction::Set(value));
            }
        }

        debug!(
            "Configured envs: {}",
            self.env_vars
                .iter()
                .map(|(key, action)| match action {
                    EnvAction::Set(value) => format!("{key}={value}"),
                    EnvAction::Preserve => format!("{key}=<preserved>"),
                    EnvAction::Remove => format!("{key}=<removed>"),
                })
                .collect::<Vec<_>>()
                .join(", ")
        );

        self
    }

    fn apply_env_to_exec(&self, mut cmd: Exec) -> Exec {
        for (key, action) in &self.env_vars {
            match action {
                EnvAction::Set(value) => {
                    cmd = cmd.env(key, value);
                }
                EnvAction::Preserve => {
                    // Only preserve if present in current environment
                    if let Ok(value) = std::env::var(key) {
                        cmd = cmd.env(key, value);
                    }
                }
                EnvAction::Remove => {
                    // For remove, we'll handle this in the sudo construction
                    // by not including it in preserved variables
                }
            }
        }
        cmd
    }

    fn build_sudo_cmd(&self) -> Exec {
        let mut cmd = Exec::cmd("sudo");

        // Collect variables to preserve for sudo
        let mut preserve_vars = Vec::new();
        let mut explicit_env_vars = HashMap::new();

        for (key, action) in &self.env_vars {
            match action {
                EnvAction::Set(value) => {
                    explicit_env_vars.insert(key.clone(), value.clone());
                }
                EnvAction::Preserve => {
                    preserve_vars.push(key.as_str());
                }
                EnvAction::Remove => {
                    // Explicitly don't add to preserve_vars
                }
            }
        }

        // Platform-agnostic handling for preserve-env
        if !preserve_vars.is_empty() {
            // NH_SUDO_PRESERVE_ENV: set to "0" to disable --preserve-env, "1" to force, unset defaults to force
            let preserve_env_override =
                std::env::var("NH_SUDO_PRESERVE_ENV").ok();
            match preserve_env_override.as_deref() {
                Some("0") => {
                    cmd = cmd.arg("--set-home");
                }
                Some("1") | None => {
                    cmd = cmd.args(&[
                        "--set-home",
                        &format!(
                            "--preserve-env={}",
                            preserve_vars.join(",")
                        ),
                    ]);
                }
                _ => {
                    cmd = cmd.args(&[
                        "--set-home",
                        &format!(
                            "--preserve-env={}",
                            preserve_vars.join(",")
                        ),
                    ]);
                }
            }
        }

        // Use NH_SUDO_ASKPASS program for sudo if present
        if let Ok(askpass) = std::env::var("NH_SUDO_ASKPASS") {
            cmd = cmd.env("SUDO_ASKPASS", askpass).arg("-A");
        }

        // Insert 'env' command to explicitly pass environment variables to the elevated command
        if !explicit_env_vars.is_empty() {
            cmd = cmd.arg("env");
            for (key, value) in explicit_env_vars {
                cmd = cmd.arg(format!("{key}={value}"));
            }
        }

        cmd
    }

    /// Create a sudo command for self-elevation with proper environment handling
    ///
    /// # Errors
    ///
    /// Returns an error if the current executable path cannot be determined or sudo command cannot be built.
    pub fn self_elevate_cmd() -> Result<std::process::Command> {
        // Get the current executable path
        let current_exe = std::env::current_exe()
            .context("Failed to get current executable path")?;

        // Self-elevation with proper environment handling
        let cmd_builder =
            Self::new(&current_exe).elevate(true).with_required_env();

        let sudo_exec = cmd_builder.build_sudo_cmd();

        // Add the target executable and arguments to the sudo command
        let exec_with_args = sudo_exec.arg(&current_exe);
        let args: Vec<String> = std::env::args().skip(1).collect();
        let final_exec = exec_with_args.args(&args);

        // Convert Exec to std::process::Command by parsing the command line
        let cmdline = final_exec.to_cmdline_lossy();
        let parts: Vec<&str> = cmdline.split_whitespace().collect();

        if parts.is_empty() {
            bail!("Failed to build sudo command");
        }

        let mut std_cmd = std::process::Command::new(parts[0]);
        if parts.len() > 1 {
            std_cmd.args(&parts[1..]);
        }

        Ok(std_cmd)
    }

    /// Run the configured command.
    ///
    /// # Errors
    ///
    /// Returns an error if the command fails to execute or returns a non-zero exit status.
    ///
    /// # Panics
    ///
    /// Panics if the command result is unexpectedly None.
    pub fn run(&self) -> Result<()> {
        let cmd = if self.elevate {
            self.build_sudo_cmd().arg(&self.command).args(&self.args)
        } else {
            self.apply_env_to_exec(
                Exec::cmd(&self.command).args(&self.args),
            )
        };

        // Configure output redirection based on show_output setting
        let cmd = ssh_wrap(
            if self.show_output {
                cmd.stderr(Redirection::Merge)
            } else {
                cmd.stderr(Redirection::None).stdout(Redirection::None)
            },
            self.ssh.as_deref(),
        );

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
        let res = cmd.capture();
        match res {
            Ok(capture) => {
                let status = &capture.exit_status;
                if !status.success() {
                    let stderr = capture.stderr_str();
                    if stderr.trim().is_empty() {
                        return Err(eyre::eyre!(format!(
                            "{} (exit status {:?})",
                            msg, status
                        )));
                    }
                    return Err(eyre::eyre!(format!(
                        "{} (exit status {:?})\nstderr:\n{}",
                        msg, status, stderr
                    )));
                }
                Ok(())
            }
            Err(e) => Err(e).wrap_err(msg),
        }
    }
}

#[derive(Debug)]
pub struct Build {
    drv: String,
    message: Option<String>,
    extra_args: Vec<OsString>,
    builder: Option<String>,
}

impl Build {
    pub const fn new(drv: String) -> Self {
        Self {
            message: None,
            drv,
            extra_args: vec![],
            builder: None,
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
    pub fn builder(mut self, builder: Option<String>) -> Self {
        self.builder = builder;
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

        let base_command = Exec::cmd("nix")
            .arg("build")
            .arg(&self.drv)
            .args(&match &self.builder {
                Some(host) => {
                    vec![
                        "--builders".to_string(),
                        format!("ssh://{host} - - - 100"),
                    ]
                }
                None => vec![],
            })
            .args(&self.extra_args);

        let exit = {
            let cmd = base_command
                .stderr(Redirection::Merge)
                .stdout(Redirection::None);
            debug!(?cmd);
            cmd.join()
        };

        match exit? {
            ExitStatus::Exited(0) => (),
            other => bail!(ExitError(other)),
        }

        Ok(())
    }
}

#[derive(Debug, Error)]
#[error("Command exited with status {0:?}")]
pub struct ExitError(ExitStatus);
