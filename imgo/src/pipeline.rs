//! Pipeline orchestration: image discovery, parallel execution,
//! backup, and output resolution shared by all transcoder kinds.

use std::fs::create_dir_all;
use std::fs::rename;
use std::iter::repeat;
use std::num::NonZeroU64;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use anyhow::bail;
use anyhow::ensure;
use indicatif::ProgressBar;
use indicatif::ProgressStyle;
use ino_color::ceprintln;
use ino_color::fg::BrightBlue;
use ino_color::fg::Red;
use ino_color::fg::Yellow;
use itertools::izip;
use parking_lot::Mutex;
use rayon::ThreadPoolBuilder;
use tempfile::NamedTempFile;
use tracing::debug;
use tracing::debug_span;

use crate::BACKUP_DIR_NAME;
use crate::BaseSeqExt;
use crate::External;
use crate::Image;
use crate::ImageFormat;
use crate::Pixel;
use crate::RelAbs;
use crate::collect_images;

/// Type alias for the transcoder work closure passed through the pipeline.
type Work<'a> =
    dyn Fn(&Image, &Path) -> anyhow::Result<Vec<String>> + Sync + 'a;

/// Internal flag shared across worker threads: once one task fails,
/// remaining tasks observe `Cancel` and skip themselves.
enum Permit {
    Go,
    Cancel,
}

/// Reports a fatal task error: prints the message bar-aware, marks the
/// pipeline as cancelled, and advances the progress bar. Always returns
/// `None` so callers can write `fail(...)? `-style control flow with `Option`.
fn fail(
    permit: &Arc<Mutex<Permit>>,
    bar: &ProgressBar,
    msg: impl std::fmt::Display,
) {
    bar.suspend(|| ceprintln!(Red, "{msg}"));
    *permit.lock() = Permit::Cancel;
    bar.inc(1);
}

// ── Fail-aware helpers: return None after calling fail() ───────────
// These exist so the worker closure can use `?` for abort control flow.

/// Run the transcoder work and collect any warnings.
fn run_work(
    exec: &Work<'_>,
    permit: &Arc<Mutex<Permit>>,
    bar: &ProgressBar,
    image: &Image,
    temp: &Path,
) -> Option<Vec<String>> {
    match exec(image, temp) {
        Ok(w) => Some(w),
        Err(e) => {
            fail(
                permit,
                bar,
                format!(
                    "Failed to process {}: {e}",
                    image.path.original_path().display()
                ),
            );
            None
        }
    }
}

/// Move the source to backup, creating the backup directory tree first.
fn backup(
    permit: &Arc<Mutex<Permit>>,
    bar: &ProgressBar,
    image: &Image,
    input_path: &Path,
    backup_dir: &Path,
) -> Option<()> {
    let backup_path = image.path.backup_path_structure(backup_dir);
    if let Some(backup_parent) = backup_path.parent()
        && let Err(e) = create_dir_all(backup_parent)
    {
        fail(
            permit,
            bar,
            format!(
                "Failed to create backup dir {}: {e}",
                backup_parent.display()
            ),
        );
        return None;
    }
    if let Err(e) = rename(input_path, &backup_path) {
        fail(
            permit,
            bar,
            format!("Failed to backup {}: {e}", input_path.display()),
        );
        return None;
    }
    debug!("Backed up to {}", backup_path.display());
    Some(())
}

// ── Infallible helpers ────────────────────────────────────────────

/// Print any warnings the transcoder surfaced, bar-aware.
fn print_warnings(bar: &ProgressBar, warnings: &[String]) {
    for warning in warnings {
        bar.suspend(|| ceprintln!(Yellow, "{warning}"));
    }
}

/// Build the destination path, resolving conflicts by incrementing seq.
fn resolve_dest(
    dest_dir: &Path,
    image: &Image,
    output_ext: &str,
) -> PathBuf {
    let mut output_extra = image.extra.set_ext(&format!(".{output_ext}"));
    let mut dest_path = dest_dir.join(output_extra.to_filename());
    while dest_path.exists() {
        debug!(
            r#"Destination "{}" exists, incrementing seq"#,
            dest_path.display()
        );
        output_extra = output_extra.increment_seq();
        dest_path = dest_dir.join(output_extra.to_filename());
    }
    dest_path
}

/// Processes a single image: temp → work → warnings → backup →
/// resolve dest → finalize. Returns `None` if the task was cancelled
/// or failed (in which case `fail()` has already been called).
fn process_one(
    permit: &Arc<Mutex<Permit>>,
    bar: &ProgressBar,
    exec: &Work<'_>,
    image: &Image,
    backup_dir: &Path,
    no_backup: bool,
    output_ext: &str,
) -> Option<()> {
    if matches!(*permit.lock(), Permit::Cancel) {
        debug!("Job cancelled");
        return None;
    }
    let _g = debug_span!("processing", ?image).entered();
    let input_path = image.path.original_path();

    bar.suspend(|| {
        ceprintln!(BrightBlue, "Processing: {}", input_path.display());
    });

    let temp_output =
        match NamedTempFile::with_suffix(format!(".{output_ext}")) {
            Ok(t) => t,
            Err(e) => {
                fail(
                    permit,
                    bar,
                    format!(
                        "Failed to create tempfile for {}: {e}",
                        input_path.display()
                    ),
                );
                return None;
            }
        };

    let warnings = run_work(exec, permit, bar, image, temp_output.path())?;
    print_warnings(bar, &warnings);

    let dest_dir = image.path.parent_dir().or_else(|| {
        fail(permit, bar, "[BUG] Failed to get parent directory");
        None
    })?;

    if !no_backup {
        backup(permit, bar, image, &input_path, backup_dir)?;
    }

    let dest_path = resolve_dest(&dest_dir, image, output_ext);

    if let Err(e) = std::fs::copy(temp_output.path(), &dest_path) {
        fail(
            permit,
            bar,
            format!(
                "Failed to copy output to {}: {e}",
                dest_path.display()
            ),
        );
        return None;
    }

    bar.inc(1);
    Some(())
}

/// Shared CLI options common to every transcoder subcommand.
#[derive(clap::Args)]
#[derive(Debug)]
pub struct SharedOpts {
    /// The starting point for finding images. Also the backup
    /// folder will be created here.
    /// Defaults to `PWD`.
    #[arg(long, short = 'W')]
    pub workspace: Option<PathBuf>,

    /// Leaving original pictures at the place after transcoding
    /// skipping backup.
    #[arg(long, short = 'N')]
    #[arg(default_value_t = false)]
    pub no_backup: bool,

    /// Number of parallel transcoding to run.
    /// The default job count is transcoder dependent.
    #[arg(long, short = 'J')]
    pub jobs: Option<NonZeroU64>,

    /// Do not recurse into subdirectories when collecting images.
    /// Only images from the workspace or current directory will be processed.
    #[arg(long, short = 'R')]
    #[arg(default_value_t = false)]
    pub no_recursive: bool,

    /// Manually choose pictures to transcode.
    /// This also disables backup.
    // #[arg(last = true)]
    pub manual_selection: Option<Vec<PathBuf>>,
}

impl SharedOpts {
    /// Effective "skip backup" flag: explicit `--no-backup` OR manual
    /// selection (which always disables backup).
    #[inline]
    #[must_use]
    pub fn skips_backup(&self) -> bool {
        self.no_backup || self.manual_selection.is_some()
    }
}
/// Collect images for the given input formats, honoring `SharedOpts`
/// for workspace, recursion, and manual selection.
fn collect_for(
    shared: &SharedOpts,
    input_formats: &[ImageFormat],
) -> anyhow::Result<(PathBuf, Vec<Image>)> {
    let workspace = {
        let pwd = std::env::current_dir()?;
        shared.workspace.as_ref().map_or(pwd, Clone::clone)
    };

    let images = if let Some(man_sel) = &shared.manual_selection {
        debug!("Use manually chosen images");
        let mut accu = vec![];
        for sel in man_sel {
            if sel.is_dir() {
                if shared.no_recursive {
                    debug!(
                        "{} is a directory, skipping in non-recursive mode",
                        sel.display()
                    );
                    continue;
                }
                debug!(
                    "Selection {} is a directory, collecting images",
                    sel.display()
                );
                let collected = collect_images(
                    sel,
                    input_formats,
                    !shared.no_recursive,
                )
                .with_context(|| {
                    format!(
                        "Failed to collect images from {}",
                        sel.display()
                    )
                })?;
                accu.extend(collected);
            } else {
                let path = RelAbs::from_path(&workspace, sel)?;
                let Some(format) = ImageFormat::from_path(sel) else {
                    bail!(
                        "The format of {} is not supported",
                        sel.display()
                    );
                };
                ensure!(
                    input_formats.contains(&format),
                    "Format {:?} of {} is not accepted by this transcoder",
                    format,
                    sel.display()
                );
                let extra = BaseSeqExt::try_from(sel.as_ref())?;
                accu.push(Image {
                    path,
                    format,
                    extra,
                });
            }
        }
        accu
    } else {
        debug!(
            "No manual selection, collect images from {} of {:?}",
            workspace.display(),
            input_formats
        );
        collect_images(&workspace, input_formats, !shared.no_recursive)
            .context("Failed to collect images")?
    };

    Ok((workspace, images))
}

/// Shared orchestration core: owns backup, progress, parallel
/// execution, temp files, and output resolution. The `execute`
/// closure does the actual work (spawn command or transform pixels)
/// and writes its result to the given temp path.
///
/// `execute` returns a list of warning messages (e.g. lossy
/// downconversion notices) which `orchestrate` prints bar-aware. It
/// must not touch the filesystem beyond the temp path.
fn orchestrate(
    workspace: &Path,
    images: Vec<Image>,
    no_backup: bool,
    jobs: NonZeroU64,
    output_format: ImageFormat,
    execute: impl Fn(&Image, &Path) -> anyhow::Result<Vec<String>>
    + Send
    + Sync,
) -> anyhow::Result<()> {
    if images.is_empty() {
        ceprintln!(Yellow, "No images to process.");
        return Ok(());
    }

    let backup_dir = Arc::new({
        let dir = workspace.join(BACKUP_DIR_NAME);
        if !no_backup {
            std::fs::create_dir_all(&dir)?;
        }
        dir
    });

    let progress_bar = {
        let bar = ProgressBar::new(images.len() as u64);
        let style = ProgressStyle::with_template(
            "{spinner:.green} [{elapsed_precise}] [{bar:40.blue/gray}] {pos}/{len} ({eta})",
        )?
        .progress_chars("#>-");
        bar.set_style(style);
        bar.enable_steady_tick(Duration::from_millis(100));
        bar
    };

    #[expect(clippy::cast_possible_truncation)]
    let thread_pool = ThreadPoolBuilder::new()
        .num_threads(jobs.get() as usize)
        .build()?;

    let Some(output_ext) = output_format.exts().first() else {
        bail!("[BUG] Output format has no ext");
    };

    thread_pool.scope(|scope| -> anyhow::Result<()> {
        let permit = Arc::new(Mutex::new(Permit::Go));
        let exec: &Work<'_> = &execute;

        for (image, permit, bar, backup_dir) in izip!(
            images,
            repeat(permit),
            repeat(progress_bar.clone()),
            repeat(backup_dir),
        ) {
            scope.spawn(move |_| {
                let _ = process_one(
                    &permit,
                    &bar,
                    exec,
                    &image,
                    &backup_dir,
                    no_backup,
                    output_ext,
                );
            });
        }

        Ok(())
    })?;

    progress_bar.finish();
    Ok(())
}

/// Runs the external (shell-out) transcoder pipeline.
///
/// # Errors
///
/// Returns an error if image collection fails, the transcoder
/// command cannot be spawned, or the command exits with a non-zero
/// status.
pub fn run_pipeline_external(
    shared: &SharedOpts,
    transcoder: &dyn External,
) -> anyhow::Result<()> {
    ceprintln!(Yellow, "[Transcoder is {}]", transcoder.id());

    let (workspace, images) =
        collect_for(shared, transcoder.input_formats())?;

    orchestrate(
        &workspace,
        images,
        shared.skips_backup(),
        shared.jobs.unwrap_or_else(|| transcoder.default_jobs()),
        transcoder.output_format(),
        |image, temp| {
            let input_path = image.path.original_path();
            let mut cmd = transcoder.transcode(&input_path, temp);
            let output = cmd
                .output()
                .with_context(|| format!("spawn {}", transcoder.id()))?;
            if !output.status.success() {
                bail!(
                    "{} failed for {} (exit {:?}):\nstdout: {}\nstderr: {}",
                    transcoder.id(),
                    input_path.display(),
                    output.status.code(),
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr),
                );
            }
            Ok(vec![])
        },
    )
}

/// Runs the in-process pixel transcoder pipeline.
///
/// # Errors
///
/// Returns an error if image collection fails, the input image
/// cannot be decoded, the transcoder's `transform` fails, or the
/// output PNG cannot be encoded.
pub fn run_pipeline_pixel(
    shared: &SharedOpts,
    transcoder: &dyn Pixel,
) -> anyhow::Result<()> {
    ceprintln!(Yellow, "[Transcoder is {}]", transcoder.id());
    ceprintln!(
        Yellow,
        "Note: metadata (EXIF/ICC) is stripped; GIF uses first frame only."
    );

    let (workspace, images) =
        collect_for(shared, transcoder.input_formats())?;

    orchestrate(
        &workspace,
        images,
        shared.skips_backup(),
        shared.jobs.unwrap_or_else(|| transcoder.default_jobs()),
        transcoder.output_format(),
        |image, temp| {
            let input_path = image.path.original_path();
            let img = image::open(&input_path).with_context(|| {
                format!("decode {}", input_path.display())
            })?;

            let mut warnings = Vec::new();

            let bpp = img.color().bits_per_pixel();
            if bpp > 32 {
                warnings.push(format!(
                    "{}: {bpp}-bit input, downconverting to 8-bit (lossy)",
                    input_path.display()
                ));
            }
            if image.format == ImageFormat::GIF {
                warnings.push(format!(
                    "{}: GIF, only first frame processed",
                    input_path.display()
                ));
            }

            let mut rgba = img.to_rgba8();
            transcoder.transform(&mut rgba)?;
            rgba.save(temp)
                .with_context(|| format!("encode {}", temp.display()))?;
            Ok(warnings)
        },
    )
}
