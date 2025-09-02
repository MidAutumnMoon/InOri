use color_eyre::Result;

/// Checks if core NH environment variables are set correctly. This was previously
/// `setup_environment()`, but the setup logic has been moved away.
///
/// # Returns
///
/// - `Result<()>` - Ok under all conditions. The user will only receive
///   a warning when their variable is determined to be outdated.
pub fn verify_variables() -> Result<()> {
    if let Ok(f) = std::env::var("FLAKE") {
        // Set NH_FLAKE if it's not already set
        if std::env::var("NH_FLAKE").is_err() {
            unsafe {
                std::env::set_var("NH_FLAKE", f);
            }

            // Only warn if FLAKE is set and we're using it to set NH_FLAKE
            // AND none of the command-specific env vars are set
            if std::env::var("NH_OS_FLAKE").is_err() {
                tracing::warn!(
                    "nh now uses NH_FLAKE instead of FLAKE, please update your configuration",
                );
            }
        }
    }

    Ok(())
}
