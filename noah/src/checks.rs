
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
    fn verify_nix_environment_bypassed_with_nh_no_checks() {
        let _guard = EnvGuard::new("NH_NO_CHECKS", "1");

        let result = verify_nix_environment();

        assert!(
            result.is_ok(),
            "verify_nix_environment should succeed when NH_NO_CHECKS is set"
        );
    }
}
