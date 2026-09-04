//! Package diff between system generations, powered by `dix`.
//!
//! Shown after a rebuild (`build`, `switch`, `boot`, `test`) and before a
//! `rollback`, per the `--diff` mode.

use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;

use super::CURRENT_PROFILE;
use rootcause::Result;
use rootcause::prelude::ResultExt as _;
use tracing::debug;
use tracing::info;
use tracing::warn;
use yansi::Paint;

#[derive(Clone, Default, Debug)]
pub enum DiffMode {
    /// Display a package diff when the current and deployed configurations are
    /// comparable.
    #[default]
    Auto,
    /// Always display a package diff.
    Always,
    /// Never display a package diff.
    Never,
}

impl FromStr for DiffMode {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value {
            "auto" => Ok(Self::Auto),
            "always" => Ok(Self::Always),
            "never" => Ok(Self::Never),
            other => Err(format!(
                "expected one of `auto`, `always`, `never`, got `{other}`"
            )),
        }
    }
}

impl std::fmt::Display for DiffMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Auto => "auto",
            Self::Always => "always",
            Self::Never => "never",
        })
    }
}

/// Converts an error produced by an external crate into a
/// [`rootcause::Report`].
///
/// Dependencies that don't speak rootcause (i.e. `dix`) report their failures
/// through the boxed-error representation; this is the seam where such
/// errors enter rootcause.
fn external_report<E>(err: E) -> rootcause::Report
where
    Box<dyn std::error::Error + Send + Sync>: From<E>,
{
    rootcause::report!(Box::<dyn std::error::Error + Send + Sync>::from(
        err
    ))
    .into()
}

struct QueriedDiff {
    old_label: PathBuf,
    new_label: PathBuf,
    report: dix::DiffReport,
}

impl QueriedDiff {
    fn write(&self) -> Result<()> {
        print_dix_header_raw(&self.old_label, &self.new_label);
        write_dix_report(&self.report)
    }
}

/// Prints the difference between two generations in terms of paths and closure
/// sizes.
///
/// # Errors
///
/// Returns an error if querying the store or writing the diff report fails.
fn print_dix_report(
    old_generation: &Path,
    new_generation: &Path,
) -> Result<()> {
    let report =
        dix::query_diff_report(old_generation, new_generation, true)
            .map_err(external_report)?;

    QueriedDiff {
        old_label: display_path(old_generation),
        new_label: display_path(new_generation),
        report,
    }
    .write()
}

/// Handles NixOS system diffing for a rebuild.
///
/// `target_profile` is the store path of the profile to compare against the
/// currently booted system.
///
/// # Errors
///
/// Returns an error if the local store snapshot queries or the diff report
/// writing fail.
pub fn run(diff: &DiffMode, target_profile: &Path) -> Result<()> {
    let current_profile = Path::new(CURRENT_PROFILE);

    match diff {
        DiffMode::Never => {
            debug!("Not running dix as the --diff flag is set to never.");
            return Ok(());
        }
        DiffMode::Auto if !current_profile.exists() => {
            warn!(
                "current profile {} does not exist, skipping dix diffing",
                current_profile.display()
            );
            return Ok(());
        }
        DiffMode::Auto => {
            debug!(
                "Comparing current profile {} with target profile: {}",
                current_profile.display(),
                target_profile.display()
            );
        }
        DiffMode::Always => {}
    }

    print_dix_report(current_profile, target_profile)
}

fn display_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn print_dix_header_raw(old_label: &Path, new_label: &Path) {
    println!(
        "{arrows} {old}",
        arrows = Paint::new("<<<").bold(),
        old = old_label.display(),
    );
    println!(
        "{arrows} {new}",
        arrows = Paint::new(">>>").bold(),
        new = new_label.display(),
    );
}

fn write_dix_report(report: &dix::DiffReport) -> Result<()> {
    let mut out = String::new();
    let wrote = dix::write_diff_report(&mut out, report)?;

    std::io::stdout()
        .write_all(out.as_bytes())
        .context("Failed to write diff report to stdout")?;

    if wrote == 0 && report.size_old() == report.size_new() {
        info!("No version or size changes.");
    }

    Ok(())
}
