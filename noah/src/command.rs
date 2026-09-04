//! Running external commands, optionally through an elevation program.
//!
//! [`Command`] is a builder for an external invocation (e.g.
//! `switch-to-configuration`, `ln`): it decides which environment the child
//! sees, and whether the call is wrapped in the configured elevation
//! program.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::os::unix::process::CommandExt as _;
use std::path::PathBuf;

use rootcause::Result;
use rootcause::prelude::ResultExt as _;
use rootcause::report;
use subprocess::Exec;
use subprocess::Redirection;
use tracing::debug;
use tracing::info;

use crate::elevation::Elevation;
use crate::runtime::Env;

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
            if self.elevation.preserves_env() {
                for (key, value) in self.env.nix_child_env() {
                    vars.push((key, value));
                }
            }
        } else {
            for (key, value) in self.env.child_env() {
                vars.push((key, value));
            }
        }

        if !elevated || self.elevation.preserves_env() {
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
                && let Some(configured_askpass) = self.elevation.askpass()
            {
                args.push(OsString::from("-A"));
                askpass = Some(configured_askpass);
            }
            args.extend(
                self.elevation.sudo_opts().iter().map(OsString::from),
            );
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
        // `parts` borrows the builder, so it must stay bound.
        let builder =
            Self::new(env.executable(), env, elevation).elevate(true);
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
                rootcause::bail!(format!(
                    "{} (exit status {:?})",
                    msg, exit_status
                ));
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
                            rootcause::bail!(format!(
                                "{} (exit status {:?})",
                                msg, status
                            ));
                        }
                        rootcause::bail!(format!(
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

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::unwrap_used,
        reason = "Fine in tests"
    )]

    use std::collections::BTreeMap;

    use super::*;
    use crate::elevation::ElevationStrategy;

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
}
