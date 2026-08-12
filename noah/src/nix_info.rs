use std::collections::HashSet;
use std::sync::{LazyLock, OnceLock};

use color_eyre::eyre::Context;
use ino_shell::{Shell, cmd};
use regex::Regex;
use tracing::debug;

use crate::Result;

/// Variant of the system Nix. Determinate Nix is not supported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NixVariant {
    Nix,
    Lix,
}

static NIX_VERSION_OUTPUT: LazyLock<Option<String>> =
    LazyLock::new(|| {
        let shell = Shell::new().ok()?;
        cmd!(shell, "nix --version").read().ok()
    });
// Caching only on success so a transient failure is retried on the next call.
static NIX_EXPERIMENTAL_FEATURES: OnceLock<HashSet<String>> =
    OnceLock::new();

/// Fetches and caches the raw output from `nix --version`.
///
/// Returns `None` if the command cannot be run (for example, Nix is not
/// installed). Callers translate `None` into their own error messages.
fn nix_version_output() -> Option<&'static String> {
    NIX_VERSION_OUTPUT.as_ref()
}

/// Detects the variant of the installed Nix.
///
/// # Errors
///
/// Returns an error if `nix --version` cannot be run.
pub fn nix_variant() -> Result<NixVariant> {
    let output = nix_version_output().ok_or_else(|| {
        color_eyre::eyre::eyre!("Failed to run `nix --version`")
    })?;

    if output.to_lowercase().contains("lix") {
        Ok(NixVariant::Lix)
    } else {
        Ok(NixVariant::Nix)
    }
}

/// Retrieves the installed Nix version string.
///
/// This function does not perform any kind of validation; its sole purpose is
/// to get the version. To validate a version string, use
/// [`normalize_version_string`].
///
/// # Errors
///
/// Returns an error if `nix --version` produces no output or the output
/// contains no valid version string.
pub fn nix_version() -> Result<String> {
    let output = nix_version_output().ok_or_else(|| {
        color_eyre::eyre::eyre!("No output from `nix --version` command")
    })?;

    let version = output.lines().next().ok_or_else(|| {
        color_eyre::eyre::eyre!("No version string found")
    })?;

    Ok(version.to_string())
}

// Matches and captures major, minor, and optional patch numbers from semantic
// version strings, optionally followed by a "pre" pre-release suffix.
#[allow(clippy::expect_used)]
static VERSION_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(\d+)\.(\d+)(?:\.(\d+))?(?:pre\d*)?")
        .expect("VERSION_REGEX should be valid")
});

/// Normalizes a version string to be compatible with semver parsing.
///
/// This function handles, or at least tries to handle, various Nix vendors'
/// complex version formats by extracting just the semantic version part.
///
/// Examples of supported formats:
/// - "2.25.0-pre" -> "2.25.0"
/// - "2.24.14-1" -> "2.24.14"
/// - "`2.30pre20250521_76a4d4c2`" -> "2.30.0"
/// - "2.91.1" -> "2.91.1"
pub fn normalize_version_string(version: &str) -> String {
    if let Some(captures) = VERSION_REGEX.captures(version) {
        let major = captures.get(1).map_or_else(
            || {
                debug!("Failed to extract major version from '{version}'");
                version
            },
            |m| m.as_str(),
        );
        let minor = captures.get(2).map_or_else(
            || {
                debug!("Failed to extract minor version from '{version}'");
                version
            },
            |m| m.as_str(),
        );
        let patch = captures.get(3).map_or("0", |m| m.as_str());

        let normalized = format!("{major}.{minor}.{patch}");
        if version != normalized {
            debug!("Version normalized: '{version}' -> '{normalized}'");
        }

        return normalized;
    }

    // Fallback: split on common separators and take the first part
    let base_version = version
        .split(&['-', '+', 'p', '_'][..])
        .next()
        .unwrap_or(version);

    // Version should have all three components (major.minor.patch)
    let normalized =
        match base_version.split('.').collect::<Vec<_>>().as_slice() {
            [major] => format!("{major}.0.0"),
            [major, minor] => format!("{major}.{minor}.0"),
            _ => base_version.to_string(),
        };

    if version != normalized {
        debug!("Version normalized: '{version}' -> '{normalized}'");
    }

    normalized
}

/// Retrieves all enabled experimental features in Nix (cached).
///
/// This function executes the `nix config show experimental-features` command
/// once, caches the result, and returns a `HashSet` of the enabled features.
///
/// # Errors
///
/// Returns an error if `nix config show experimental-features` fails to
/// execute.
pub fn nix_experimental_features() -> Result<HashSet<String>> {
    if let Some(features) = NIX_EXPERIMENTAL_FEATURES.get() {
        return Ok(features.clone());
    }

    let shell = Shell::new()?;
    let output = cmd!(shell, "nix config show experimental-features")
        .read()
        .context(
            "Failed to run `nix config show experimental-features`",
        )?;

    let enabled: HashSet<String> =
        output.split_whitespace().map(String::from).collect();

    let _ = NIX_EXPERIMENTAL_FEATURES.set(enabled.clone());
    Ok(enabled)
}

/// Gets the missing experimental features from a required list.
///
/// # Errors
///
/// Returns an error if retrieving the list of enabled experimental features
/// fails.
pub fn missing_experimental_features(
    required: &[&str],
) -> Result<Vec<String>> {
    let enabled = nix_experimental_features()?;

    let missing: Vec<String> = required
        .iter()
        .filter(|&feature| !enabled.contains(*feature))
        .map(|&feature| feature.to_string())
        .collect();

    Ok(missing)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        #[test]
        fn normalize_version_string_handles_various_formats(
            major in 1u32..10,
            minor in 0u32..99,
            patch in 0u32..99
        ) {
            let basic = format!("{major}.{minor}.{patch}");
            prop_assert_eq!(&normalize_version_string(&basic), &basic);

            let pre_release = format!("{major}.{minor}.{patch}-pre");
            prop_assert_eq!(&normalize_version_string(&pre_release), &basic);

            let distro = format!("{major}.{minor}.{patch}-1");
            prop_assert_eq!(&normalize_version_string(&distro), &basic);

            let no_patch = format!("{major}.{minor}");
            prop_assert_eq!(
                normalize_version_string(&no_patch),
                format!("{major}.{minor}.0")
            );

            let complex = format!("{major}.{minor}pre20250521_76a4d4c2");
            prop_assert_eq!(
                normalize_version_string(&complex),
                format!("{major}.{minor}.0")
            );
        }
    }

    #[test]
    fn normalize_version_string_with_real_nix_versions() {
        assert_eq!(
            normalize_version_string("2.30pre20250521_76a4d4c2"),
            "2.30.0"
        );

        assert_eq!(normalize_version_string("2.25.0-pre"), "2.25.0");
        assert_eq!(normalize_version_string("2.24.14-1"), "2.24.14");
        assert_eq!(normalize_version_string("2.91.1"), "2.91.1");
        assert_eq!(normalize_version_string("2.18"), "2.18.0");

        assert_eq!(normalize_version_string("3.0dev"), "3.0.0");
        assert_eq!(normalize_version_string("2.22rc1"), "2.22.0");
        assert_eq!(normalize_version_string("2.19_git_abc123"), "2.19.0");

        assert_eq!(normalize_version_string("1.2-beta"), "1.2.0");
        assert_eq!(normalize_version_string("3.4+build.1"), "3.4.0");
        assert_eq!(normalize_version_string("5.6_alpha"), "5.6.0");

        assert_eq!(normalize_version_string("2-rc1"), "2.0.0");
        assert_eq!(normalize_version_string("4+build"), "4.0.0");
        assert_eq!(normalize_version_string("7_dev"), "7.0.0");
    }
}
