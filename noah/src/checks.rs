use std::{cmp::Ordering, env};

use nh::EyreRootcauseBridge;
use semver::Version;
use tracing::{debug, warn};

use crate::nix_info::{
    NixVariant, missing_experimental_features, nix_variant, nix_version,
    normalize_version_string,
};

/// Verifies if the installed Nix version meets requirements.
///
/// # Returns
///
/// * `Result<()>` - Ok if version requirements are met, error otherwise
///
/// # Errors
///
/// Returns an error if the Nix version cannot be determined or parsed.
pub fn check_nix_version() -> rootcause::Result<()> {
    // XXX: Both Nix and Lix follow semantic versioning (semver). Update the
    // versions below once latest stable for either of those packages change.
    // We *also* cannot (or rather, will not) make this check for non-nixpkgs
    // Nix variants, since there is no good baseline for what to support
    // without the understanding of stable/unstable branches.
    // TODO: Set up a CI to automatically update those in the future.
    const MIN_LIX_VERSION: &str = "2.93.3";
    const MIN_NIX_VERSION: &str = "2.31.2";

    if env::var("NH_NO_CHECKS").is_ok() {
        return Ok(());
    }

    let variant = nix_variant()?;
    let version = nix_version().into_rootcause()?;
    let version_normal = normalize_version_string(&version);

    let min_version = match variant {
        NixVariant::Lix => MIN_LIX_VERSION,
        NixVariant::Nix => MIN_NIX_VERSION,
    };

    let current = match Version::parse(&version_normal) {
        Ok(ver) => ver,
        Err(e) => {
            warn!(
                "Failed to parse Nix version '{version_normal}': {e}. \
                 Skipping version check.",
            );
            return Ok(());
        }
    };

    let required = Version::parse(min_version)?;

    match current.cmp(&required) {
        Ordering::Less => {
            let binary_name = match variant {
                NixVariant::Lix => "Lix",
                NixVariant::Nix => "Nix",
            };
            warn!(
                "Warning: {binary_name} version {version} is older than the \
                 recommended minimum version {min_version}. You may encounter \
                 issues."
            );
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Checks if core NH environment variables are set correctly. This was
/// previously `setup_environment()`, but the setup logic has been moved away.
///
/// # Returns
///
/// - `Result<()>` - Ok under all conditions. The user will only receive a
///   warning when their variable is determined to be outdated.
// clippy warning suppressed to allow for this function to returning meaningful
// errors in the future
#[allow(clippy::unnecessary_wraps, clippy::missing_errors_doc)]
pub fn verify_variables() -> color_eyre::Result<()> {
    if let Ok(flake) = std::env::var("FLAKE") {
        // Set NH_FLAKE if it's not already set
        if std::env::var("NH_FLAKE").is_err() {
            // SAFETY: Runs during startup before any threads are spawned, so
            // there is no concurrent access to the environment.
            unsafe {
                std::env::set_var("NH_FLAKE", flake);
            }

            // Only warn if FLAKE is set and we're using it to set NH_FLAKE
            // AND none of the command-specific env vars are set
            if std::env::var("NH_OS_FLAKE").is_err()
                && std::env::var("NH_HOME_FLAKE").is_err()
                && std::env::var("NH_DARWIN_FLAKE").is_err()
            {
                tracing::warn!(
                    "nh {} now uses NH_FLAKE instead of FLAKE, please update \
                     your configuration",
                    crate::NH_VERSION
                );
            }
        }
    }

    Ok(())
}

/// Consolidate all necessary checks for Nix functionality into a single
/// function. This will be executed in the main function, but can be executed
/// before critical commands to double-check if necessary.
///
/// NOTE: Experimental feature checks are now done per-command to avoid
/// redundant error messages for features not needed by the specific command.
///
/// # Returns
///
/// * `Result<()>` - Ok if all checks pass, error otherwise
///
/// # Errors
///
/// Returns an error if any required Nix environment checks fail.
pub fn verify_nix_environment() -> rootcause::Result<()> {
    if env::var("NH_NO_CHECKS").is_ok() {
        return Ok(());
    }

    // Only check version globally. Features are checked per-command now.
    // This function is kept as is for backwards compatibility.
    check_nix_version()?;
    Ok(())
}

const FLAKE_FEATURES: &[&str] = &["nix-command", "flakes"];
const LEGACY_LIX_REPL_FEATURES: &[&str] =
    &["nix-command", "flakes", "repl-flake"];

fn required_os_repl_features(is_flake: bool) -> &'static [&'static str] {
    if !is_flake {
        return &[];
    }

    // Lix versions before 2.93 gate flake REPL support behind `repl-flake`.
    if matches!(nix_variant(), Ok(NixVariant::Lix))
        && let Ok(version) = nix_version()
        && let Ok(current) =
            Version::parse(&normalize_version_string(&version))
        && let Ok(threshold) = Version::parse("2.93.0")
        && current < threshold
    {
        return LEGACY_LIX_REPL_FEATURES;
    }

    FLAKE_FEATURES
}

/// Trait for types that have feature requirements.
pub trait FeatureRequirements {
    /// Returns the list of required experimental features.
    fn required_features(&self) -> &'static [&'static str];

    /// Checks if all required features are enabled
    ///
    /// # Errors
    ///
    /// Returns an error if any required Nix features are not enabled.
    fn check_features(&self) -> color_eyre::Result<()> {
        if env::var("NH_NO_CHECKS").is_ok() {
            return Ok(());
        }

        let required = self.required_features();
        if required.is_empty() {
            return Ok(());
        }

        debug!("Required Nix features: {}", required.join(", "));

        let missing = missing_experimental_features(required).unwrap();
        if !missing.is_empty() {
            return Err(color_eyre::eyre::eyre!(
                "Missing required experimental features for this command: {}",
                missing.join(", ")
            ));
        }

        debug!("All required Nix features are enabled");
        Ok(())
    }
}

/// Feature requirements for commands that use flakes
#[derive(Debug)]
pub struct FlakeFeatures;

impl FeatureRequirements for FlakeFeatures {
    fn required_features(&self) -> &'static [&'static str] {
        FLAKE_FEATURES
    }
}

/// Feature requirements for legacy (non-flake) commands.
///
/// XXX: There are actually no experimental feature requirements for legacy
/// (nix2) CLI but since move-fast-break-everything is a common mantra among
/// Nix & Nix-adjacent software, I've implemented this. Do not remove, this is
/// simply for futureproofing.
#[derive(Debug)]
pub struct LegacyFeatures;

impl FeatureRequirements for LegacyFeatures {
    fn required_features(&self) -> &'static [&'static str] {
        &[]
    }
}

/// Feature requirements for OS repl commands
#[derive(Debug)]
pub struct OsReplFeatures {
    pub is_flake: bool,
}

impl FeatureRequirements for OsReplFeatures {
    fn required_features(&self) -> &'static [&'static str] {
        required_os_repl_features(self.is_flake)
    }
}

/// Feature requirements for commands that don't need experimental features
#[derive(Debug)]
pub struct NoFeatures;

impl FeatureRequirements for NoFeatures {
    fn required_features(&self) -> &'static [&'static str] {
        &[]
    }
}

#[cfg(test)]
#[expect(clippy::expect_used, reason = "Fine in tests")]
mod tests {
    use std::env;

    use serial_test::serial;

    use super::*;

    // This helps set environment variables safely in tests
    struct EnvGuard {
        key: String,
        original: Option<String>,
    }

    impl EnvGuard {
        fn new(key: &str, value: &str) -> Self {
            let original = env::var(key).ok();
            unsafe {
                env::set_var(key, value);
            }
            Self {
                key: key.to_string(),
                original,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.original {
                    Some(val) => env::set_var(&self.key, val),
                    None => env::remove_var(&self.key),
                }
            }
        }
    }

    #[test]
    fn flake_features_require_nix_command_and_flakes() {
        assert_eq!(
            FlakeFeatures.required_features(),
            ["nix-command", "flakes"]
        );
    }

    #[test]
    fn legacy_features_require_nothing() {
        assert!(LegacyFeatures.required_features().is_empty());
    }

    #[test]
    fn no_features_require_nothing() {
        assert!(NoFeatures.required_features().is_empty());
    }

    #[test]
    fn non_flake_repl_requires_nothing() {
        assert!(
            OsReplFeatures { is_flake: false }
                .required_features()
                .is_empty()
        );
    }

    #[test]
    fn flake_repl_requires_base_features() {
        let features =
            OsReplFeatures { is_flake: true }.required_features();

        // Always nix-command + flakes; old Lix additionally needs repl-flake.
        assert!(features.contains(&"nix-command"));
        assert!(features.contains(&"flakes"));
        assert!(features.len() == 2 || features.contains(&"repl-flake"));
    }

    #[test]
    #[serial]
    fn setup_environment_flake_to_nh_flake_migration() {
        unsafe {
            env::remove_var("FLAKE");
            env::remove_var("NH_FLAKE");
            env::remove_var("NH_OS_FLAKE");
            env::remove_var("NH_HOME_FLAKE");
            env::remove_var("NH_DARWIN_FLAKE");
        }

        let _guard = EnvGuard::new("FLAKE", "/test/flake");

        let result = verify_variables();

        assert!(
            result.is_ok(),
            "Should warn when migrating FLAKE to NH_FLAKE"
        );
        assert_eq!(
            env::var("NH_FLAKE").expect("NH_FLAKE should be set by test"),
            "/test/flake"
        );
    }

    #[test]
    #[serial]
    fn setup_environment_no_migration_when_nh_flake_exists() {
        unsafe {
            env::remove_var("FLAKE");
            env::remove_var("NH_FLAKE");
            env::remove_var("NH_OS_FLAKE");
            env::remove_var("NH_HOME_FLAKE");
            env::remove_var("NH_DARWIN_FLAKE");
        }

        let _guard1 = EnvGuard::new("FLAKE", "/test/flake");
        let _guard2 = EnvGuard::new("NH_FLAKE", "/existing/flake");

        let result = verify_variables();

        assert!(
            result.is_ok(),
            "Should not warn when NH_FLAKE already exists"
        );
        assert_eq!(
            env::var("NH_FLAKE").expect("NH_FLAKE should be set by test"),
            "/existing/flake"
        );
    }

    #[test]
    #[serial]
    fn setup_environment_no_migration_when_specific_flake_vars_exist() {
        unsafe {
            env::remove_var("FLAKE");
            env::remove_var("NH_FLAKE");
            env::remove_var("NH_OS_FLAKE");
            env::remove_var("NH_HOME_FLAKE");
            env::remove_var("NH_DARWIN_FLAKE");
        }

        let _guard1 = EnvGuard::new("FLAKE", "/test/flake");
        let _guard2 = EnvGuard::new("NH_OS_FLAKE", "/os/flake");

        let result = verify_variables();

        assert!(
            result.is_ok(),
            "Should not warn when specific flake vars exist"
        );
        assert_eq!(
            env::var("NH_FLAKE").expect("NH_FLAKE should be set by test"),
            "/test/flake"
        );
    }

    #[test]
    #[serial]
    fn check_features_bypassed_with_nh_no_checks() {
        let _guard = EnvGuard::new("NH_NO_CHECKS", "1");

        let features = FlakeFeatures;
        let result = features.check_features();

        assert!(
            result.is_ok(),
            "check_features should succeed when NH_NO_CHECKS is set"
        );
    }

    #[test]
    #[serial]
    fn verify_nix_environment_bypassed_with_nh_no_checks() {
        let _guard = EnvGuard::new("NH_NO_CHECKS", "1");

        let result = verify_nix_environment();

        assert!(
            result.is_ok(),
            "verify_nix_environment should succeed when NH_NO_CHECKS is set"
        );
    }
}
