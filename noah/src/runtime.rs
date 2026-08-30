use std::collections::HashMap;
use std::env;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::fmt;
use std::path::Path;
use std::path::PathBuf;

use ino_shell::Shell;
use ino_shell::cmd;
use rootcause::Result;
use rootcause::prelude::ResultExt as _;
use rootcause::report;
use rustix::system::uname;
use tracing::debug;

use crate::command::Elevation;
use crate::command::ElevationStrategy;
use crate::remote::SshConfig;

const NIX_CHILD_ENV: &[&str] = &[
    "LOCALE_ARCHIVE",
    "PATH",
    "NIX_SSHOPTS",
    "NIX_CONFIG",
    "NIX_PATH",
    "NIX_REMOTE",
    "NIX_SSL_CERT_FILE",
    "NIX_USER_CONF_FILES",
];

/// Immutable snapshot of the process environment and invocation.
///
/// Capture this once during startup. Application code reads configuration and
/// explicit child-process environment values from the snapshot instead of
/// consulting ambient process state at arbitrary points in execution.
pub struct Env {
    vars: HashMap<OsString, OsString>,
    executable: PathBuf,
    arguments: Vec<OsString>,
    current_dir: PathBuf,
    current_machine_hostname: String,
}

impl Env {
    /// Capture the current environment and invocation.
    ///
    /// # Errors
    ///
    /// Returns an error if the current executable or working directory cannot
    /// be determined.
    pub fn capture() -> rootcause::Result<Self> {
        Ok(Self {
            vars: env::vars_os().collect(),
            executable: env::current_exe()
                .context("Failed to determine the current executable")?,
            arguments: env::args_os().skip(1).collect(),
            current_dir: env::current_dir()
                .context("Failed to determine the current directory")?,
            current_machine_hostname: Self::hostname()?,
        })
    }

    pub fn hostname() -> rootcause::Result<String> {
        Ok(uname().nodename().to_str()?.to_owned())
    }

    /// Return a captured environment value as UTF-8.
    #[must_use]
    pub fn var(&self, name: &str) -> Option<&str> {
        self.var_os(name).and_then(OsStr::to_str)
    }

    /// Return a captured environment value without requiring UTF-8.
    #[must_use]
    pub fn var_os(&self, name: &str) -> Option<&OsStr> {
        self.vars.get(OsStr::new(name)).map(OsString::as_os_str)
    }

    /// Return a non-empty captured UTF-8 environment value.
    #[must_use]
    pub fn non_empty_var(&self, name: &str) -> Option<&str> {
        self.var(name).filter(|value| !value.is_empty())
    }

    /// Captured variables that affect Nix child processes.
    pub fn nix_child_env(
        &self,
    ) -> impl Iterator<Item = (&'static str, &str)> + '_ {
        NIX_CHILD_ENV
            .iter()
            .filter_map(|&key| self.var(key).map(|value| (key, value)))
    }

    /// Captured identity and Nix variables for a non-elevated child process.
    pub fn child_env(
        &self,
    ) -> impl Iterator<Item = (&'static str, &str)> + '_ {
        ["USER", "HOME"]
            .into_iter()
            .filter_map(|key| self.var(key).map(|value| (key, value)))
            .chain(self.nix_child_env())
    }

    /// Parse a shell-word list from a preferred variable or a legacy fallback.
    ///
    /// An explicitly empty preferred variable disables the fallback.
    ///
    /// # Errors
    ///
    /// Returns an error when the selected value has unmatched shell quoting.
    pub fn shell_words(
        &self,
        preferred: &'static str,
        fallback: &'static str,
    ) -> Result<Vec<String>> {
        let selected = self
            .var(preferred)
            .map(|value| (preferred, value))
            .or_else(|| self.var(fallback).map(|value| (fallback, value)));
        let Some((name, value)) = selected else {
            return Ok(Vec::new());
        };
        if value.is_empty() {
            return Ok(Vec::new());
        }

        shlex::split(value).ok_or_else(|| {
            report!("{name} contains unmatched shell quoting")
        })
    }

    /// Absolute path of the running executable captured at startup.
    #[must_use]
    pub fn executable(&self) -> &Path {
        &self.executable
    }

    /// Invocation arguments, excluding `argv[0]`, captured at startup.
    #[must_use]
    pub fn arguments(&self) -> &[OsString] {
        &self.arguments
    }

    /// Working directory captured at startup.
    #[must_use]
    pub fn current_dir(&self) -> &Path {
        &self.current_dir
    }

    #[cfg(test)]
    pub(crate) fn from_pairs<I, K, V>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<OsString>,
        V: Into<OsString>,
    {
        Self {
            vars: pairs
                .into_iter()
                .map(|(key, value)| (key.into(), value.into()))
                .collect(),
            executable: PathBuf::from("/proc/self/exe"),
            arguments: Vec::new(),
            current_dir: PathBuf::from("/"),
            current_machine_hostname: String::from("test-host"),
        }
    }
}

impl fmt::Debug for Env {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Env")
            .field("variable_count", &self.vars.len())
            .field("executable", &self.executable)
            .field("argument_count", &self.arguments.len())
            .field("current_dir", &self.current_dir)
            .finish()
    }
}

/// Variant of the system Nix. Determinate Nix is not supported.
#[derive(Debug)]
pub enum NixVariant {
    Nix,
    Lix,
}

/// Configuration for a single run, assembled once after CLI parsing.
#[derive(Debug)]
pub struct Config {
    pub env: Env,
    pub elevation: Elevation,
    pub ssh: SshConfig,
    pub nix_variant: NixVariant,
}

impl Config {
    /// Assemble the run's configuration from a captured [`Env`].
    ///
    /// # Errors
    ///
    /// Returns an error if the elevation policy or SSH settings cannot be
    /// derived from the environment.
    pub fn from_env(
        env: Env,
        elevation_strategy: Option<ElevationStrategy>,
    ) -> Result<Self> {
        let elevation = Elevation::new(elevation_strategy, &env)?;
        let ssh = SshConfig::from_env(&env)?;

        Ok(Self {
            env,
            elevation,
            ssh,
            nix_variant: nix_variant()?,
        })
    }
}

fn nix_variant() -> rootcause::Result<NixVariant> {
    let variant = guess_nix_variant_from_version_output()?;
    ensure_features_needed_are_set()?;
    Ok(variant)
}

fn guess_nix_variant_from_version_output() -> rootcause::Result<NixVariant>
{
    let shell = Shell::new()?;
    let version_output = cmd!(shell, "nix --version")
        .read()
        .context("Failed to run `nix --version`")?;

    if version_output.to_lowercase().contains("lix") {
        Ok(NixVariant::Lix)
    } else {
        Ok(NixVariant::Nix)
    }
}

fn ensure_features_needed_are_set() -> rootcause::Result<()> {
    let shell = Shell::new()?;
    let expr_features =
        cmd!(shell, "nix config show experimental-features")
            .read()
            .context("Failed to read enabled experimental features")?;

    debug!(expr_features);

    if expr_features.contains("flakes")
        && expr_features.contains("nix-command")
    {
        Ok(())
    } else {
        rootcause::bail!(
            "Required flake features (nix-command, flakes) are not enabled"
        )
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "Fine in tests")]

    use super::*;

    #[test]
    fn preferred_shell_words_override_legacy_value() {
        let env = Env::from_pairs([
            ("NH_OPTS", "--option 'two words'"),
            ("NIX_OPTS", "--legacy"),
        ]);

        assert_eq!(
            env.shell_words("NH_OPTS", "NIX_OPTS").unwrap(),
            ["--option", "two words"]
        );
    }

    #[test]
    fn empty_preferred_shell_words_disable_legacy_value() {
        let env =
            Env::from_pairs([("NH_OPTS", ""), ("NIX_OPTS", "--legacy")]);

        assert!(
            env.shell_words("NH_OPTS", "NIX_OPTS").unwrap().is_empty()
        );
    }

    #[test]
    fn malformed_shell_words_are_rejected() {
        let env = Env::from_pairs([("NH_OPTS", "'unterminated")]);

        let error = env.shell_words("NH_OPTS", "NIX_OPTS").unwrap_err();
        assert!(error.to_string().contains("NH_OPTS"));
    }

    #[test]
    fn debug_output_does_not_expose_environment_values() {
        let env = Env::from_pairs([("SECRET_TOKEN", "hunter2")]);

        let rendered = format!("{env:?}");
        assert!(rendered.contains("variable_count"));
        assert!(!rendered.contains("SECRET_TOKEN"));
        assert!(!rendered.contains("hunter2"));
    }
}
