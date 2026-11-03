//! Package diff between system generations, powered by `dix`.
//!
//! Shown after a rebuild (`build`, `switch`, `boot`, `test`) and before a
//! `rollback`, per the `--diff` mode.

use std::io::Write as _;
use std::path::Path;
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

/// Converts an error produced by `dix` into a [`rootcause::Report`].
///
/// `dix` reports failures as `misstep::Report`, which does not implement
/// `std::error::Error`; `ReportError` adapts it back into an error type that
/// rootcause accepts. This is the seam where such errors enter rootcause.
fn misstep_report(err: misstep::Report) -> rootcause::Report {
    rootcause::Report::from(misstep::ReportError::from(err))
}

struct QueriedDiff {
    old: dix::SnapshotDocument,
    new: dix::SnapshotDocument,
    report: dix::DiffReport,
}

impl QueriedDiff {
    fn write(&self) -> Result<()> {
        print_dix_header_raw(&self.old.path, &self.new.path);
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
    let store = dix::Store::open();

    let old = store
        .query_snapshot_document(old_generation)
        .map_err(misstep_report)?;
    let new = store
        .query_snapshot_document(new_generation)
        .map_err(misstep_report)?;

    let report = dix::diff_store_snapshots(&old.snapshot, &new.snapshot);

    QueriedDiff { old, new, report }.write()
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
    // Render every package diff; dix's default view hides packages that only
    // changed in amount or size below its threshold.
    let options = dix::RenderOptions {
        full: true,
        ..dix::RenderOptions::default()
    };

    let mut out = String::new();
    dix::write_diff_report(&mut out, report, options)?;

    std::io::stdout()
        .write_all(out.as_bytes())
        .context("Failed to write diff report to stdout")?;

    if report.diffs.is_empty() && report.size_old == report.size_new {
        info!("No version or size changes.");
    }

    Ok(())
}
