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
/// execution, temp files, and output resolution. The `execute` closure
/// does the actual work (spawn command or transform pixels) and writes
/// its result to the given temp path.
///
/// `execute` receives `&ProgressBar` so it can emit progress-aware
/// warnings (e.g. lossy downconversion). It must not touch the
/// filesystem beyond the temp path.
fn orchestrate(
    workspace: &Path,
    images: Vec<Image>,
    no_backup: bool,
    jobs: NonZeroU64,
    output_format: ImageFormat,
    execute: impl Fn(&Image, &Path, &ProgressBar) -> anyhow::Result<()>
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
        enum Permit {
            Go,
            Cancel,
        }

        let permit = Arc::new(Mutex::new(Permit::Go));
        let exec = &execute;

        for (image, permit, bar, backup_dir) in izip!(
            images,
            repeat(permit),
            repeat(progress_bar.clone()),
            repeat(backup_dir),
        ) {
            scope.spawn(move |_| {
                if matches!(*permit.lock(), Permit::Cancel) {
                    debug!("Job cancelled");
                    return;
                }
                let _g = debug_span!("processing", ?image).entered();
                let input_path = image.path.original_path();

                bar.suspend(|| {
                    ceprintln!(
                        BrightBlue,
                        "Processing: {}",
                        input_path.display()
                    );
                });

                // Create temp file for the output.
                let temp_output = match NamedTempFile::with_suffix(
                    format!(".{output_ext}"),
                ) {
                    Ok(t) => t,
                    Err(e) => {
                        bar.suspend(|| {
                            ceprintln!(
                                Red,
                                "Failed to create tempfile for {}: {e}",
                                input_path.display()
                            );
                        });
                        *permit.lock() = Permit::Cancel;
                        bar.inc(1);
                        return;
                    }
                };

                // Execute the work (spawn command or transform pixels).
                if let Err(e) = exec(&image, temp_output.path(), &bar) {
                    bar.suspend(|| {
                        ceprintln!(
                            Red,
                            "Failed to process {}: {e}",
                            input_path.display()
                        );
                    });
                    *permit.lock() = Permit::Cancel;
                    bar.inc(1);
                    return;
                }

                // Get the destination directory (same as source).
                let Some(dest_dir) = image.path.parent_dir() else {
                    bar.suspend(|| {
                        ceprintln!(
                            Red,
                            "[BUG] Failed to get parent directory"
                        );
                    });
                    bar.inc(1);
                    return;
                };

                // Backup source BEFORE resolving destination path.
                // This frees up the original filename when source and
                // output have the same extension.
                if !no_backup {
                    let backup_path =
                        image.path.backup_path_structure(&backup_dir);
                    if let Some(backup_parent) = backup_path.parent()
                        && let Err(e) = create_dir_all(backup_parent)
                    {
                        bar.suspend(|| {
                            ceprintln!(
                                Red,
                                "Failed to create backup dir {}: {e}",
                                backup_parent.display()
                            );
                        });
                        *permit.lock() = Permit::Cancel;
                        bar.inc(1);
                        return;
                    }
                    if let Err(e) = rename(&input_path, &backup_path) {
                        bar.suspend(|| {
                            ceprintln!(
                                Red,
                                "Failed to backup {}: {e}",
                                input_path.display()
                            );
                        });
                        *permit.lock() = Permit::Cancel;
                        bar.inc(1);
                        return;
                    }
                    debug!("Backed up to {}", backup_path.display());
                }

                // Build output filename with new extension, resolving
                // conflicts by incrementing seq.
                let mut output_extra =
                    image.extra.set_ext(&format!(".{output_ext}"));
                let mut dest_path =
                    dest_dir.join(output_extra.to_filename());
                while dest_path.exists() {
                    debug!(
                        r#"Destination "{}" exists, incrementing seq"#,
                        dest_path.display()
                    );
                    output_extra = output_extra.increment_seq();
                    dest_path = dest_dir.join(output_extra.to_filename());
                }

                if let Err(e) =
                    std::fs::copy(temp_output.path(), &dest_path)
                {
                    bar.suspend(|| {
                        ceprintln!(
                            Red,
                            "Failed to copy output to {}: {e}",
                            dest_path.display()
                        );
                    });
                    bar.inc(1);
                    return;
                }

                bar.inc(1);
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

    let no_backup = shared.no_backup || shared.manual_selection.is_some();
    let jobs = shared.jobs.unwrap_or_else(|| transcoder.default_jobs());
    let output_format = transcoder.output_format();

    orchestrate(
        &workspace,
        images,
        no_backup,
        jobs,
        output_format,
        |image, temp, _bar| {
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
            Ok(())
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

    let no_backup = shared.no_backup || shared.manual_selection.is_some();
    let jobs = shared.jobs.unwrap_or_else(|| transcoder.default_jobs());
    let output_format = transcoder.output_format();

    orchestrate(
        &workspace,
        images,
        no_backup,
        jobs,
        output_format,
        |image, temp, bar| {
            let input_path = image.path.original_path();

            let img = image::open(&input_path).with_context(|| {
                format!("decode {}", input_path.display())
            })?;

            // ── Step 5 decision: warnings stay here (temporary) ──────
            // Per-image warnings (>8-bit downconversion, GIF first-frame)
            // need progress-bar access (`bar.suspend`). They live in this
            // closure rather than in `Pixel::transform` to keep the trait
            // sans-IO. This is a temporary wart; a future task will design
            // a proper warning channel (e.g. a tracing layer or a callback
            // hook on the orchestration).
            let bpp = img.color().bits_per_pixel();
            if bpp > 32 {
                bar.suspend(|| {
                    ceprintln!(
                        Yellow,
                        "{}: {bpp}-bit input, \
                         downconverting to 8-bit (lossy)",
                        input_path.display()
                    );
                });
            }
            if image.format == ImageFormat::GIF {
                bar.suspend(|| {
                    ceprintln!(
                        Yellow,
                        "{}: GIF, only first frame processed",
                        input_path.display()
                    );
                });
            }
            // ── End step 5 decision ────────────────────────────────────

            let mut rgba = img.to_rgba8();
            transcoder.transform(&mut rgba)?;
            rgba.save(temp)
                .with_context(|| format!("encode {}", temp.display()))?;
            Ok(())
        },
    )
}
