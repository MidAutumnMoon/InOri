use crate::Result;

/// Configure error reporting and tracing output.
///
/// # Errors
///
/// Returns an error if installing the error hook fails or if tracing filter
/// directives cannot be parsed.
pub fn setup_logging() -> Result<()> {
    color_eyre::config::HookBuilder::default()
        .display_location_section(true)
        .panic_section(
            "Please report the bug at https://github.com/nix-community/nh/issues",
        )
        .display_env_section(false)
        .install()?;

    Ok(())
}
