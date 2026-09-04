//! Privilege elevation: choosing a strategy and resolving the program.
//!
//! The [`ElevationStrategy`] is what the user selects (via
//! `--elevation-strategy`); [`Elevation`] is the policy for a run, combining
//! the strategy with the sudo-related environment.

use std::cell::OnceCell;
use std::convert::Infallible;
use std::ffi::OsStr;
use std::path::PathBuf;
use std::str::FromStr;

use rootcause::Result;
use rootcause::bail;
use tracing::debug;
use tracing::warn;
use which::which_in;

use crate::runtime::Env;

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

    /// Extra arguments for the `sudo` elevation program.
    #[must_use]
    pub fn sudo_opts(&self) -> &[String] {
        &self.opts
    }

    /// Path to the askpass helper, if one is configured.
    #[must_use]
    pub fn askpass(&self) -> Option<&str> {
        self.askpass.as_deref()
    }

    /// Whether the child environment should be preserved across elevation.
    #[must_use]
    pub const fn preserves_env(&self) -> bool {
        self.preserve_env
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

        Err(rootcause::report!(
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

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::unwrap_used,
        reason = "Fine in tests"
    )]

    use super::*;

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
        assert_eq!(elevation.opts, ["--preserve-env=NIX_CONFIG NIX_PATH"]);
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
