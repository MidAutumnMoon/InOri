use std::collections::HashMap;
use std::env;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::fmt;
use std::path::Path;
use std::path::PathBuf;

use rootcause::Result;
use rootcause::prelude::ResultExt as _;
use rootcause::report;

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
#[expect(
    clippy::module_name_repetitions,
    reason = "It's shared across multiple modules, Runtime* makes it clear"
)]
pub struct RuntimeEnv {
    vars: HashMap<OsString, OsString>,
    executable: PathBuf,
    arguments: Vec<OsString>,
    current_dir: PathBuf,
}

impl RuntimeEnv {
    /// Capture the current environment and invocation.
    ///
    /// # Errors
    ///
    /// Returns an error if the current executable or working directory cannot
    /// be determined.
    pub fn capture() -> Result<Self> {
        Ok(Self {
            vars: env::vars_os().collect(),
            executable: env::current_exe()
                .context("Failed to determine the current executable")?,
            arguments: env::args_os().skip(1).collect(),
            current_dir: env::current_dir()
                .context("Failed to determine the current directory")?,
        })
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
        }
    }
}

impl fmt::Debug for RuntimeEnv {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuntimeEnv")
            .field("variable_count", &self.vars.len())
            .field("executable", &self.executable)
            .field("argument_count", &self.arguments.len())
            .field("current_dir", &self.current_dir)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "Fine in tests")]

    use super::*;

    #[test]
    fn preferred_shell_words_override_legacy_value() {
        let env = RuntimeEnv::from_pairs([
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
        let env = RuntimeEnv::from_pairs([
            ("NH_OPTS", ""),
            ("NIX_OPTS", "--legacy"),
        ]);

        assert!(
            env.shell_words("NH_OPTS", "NIX_OPTS").unwrap().is_empty()
        );
    }

    #[test]
    fn malformed_shell_words_are_rejected() {
        let env = RuntimeEnv::from_pairs([("NH_OPTS", "'unterminated")]);

        let error = env.shell_words("NH_OPTS", "NIX_OPTS").unwrap_err();
        assert!(error.to_string().contains("NH_OPTS"));
    }

    #[test]
    fn debug_output_does_not_expose_environment_values() {
        let env = RuntimeEnv::from_pairs([("SECRET_TOKEN", "hunter2")]);

        let rendered = format!("{env:?}");
        assert!(rendered.contains("variable_count"));
        assert!(!rendered.contains("SECRET_TOKEN"));
        assert!(!rendered.contains("hunter2"));
    }
}
