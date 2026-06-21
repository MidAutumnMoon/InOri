use std::fs::create_dir_all;
use std::fs::rename;
use std::iter::repeat;
use std::num::NonZeroU64;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use rlimit::Resource;

use anyhow::Context;
use anyhow::bail;
use anyhow::ensure;
use imgo::BACKUP_DIR_NAME;
use imgo::BaseSeqExt;
use imgo::External;
use imgo::Image;
use imgo::ImageFormat;
use imgo::Pixel;
use imgo::RelAbs;
use imgo::Transcoder;
use imgo::avif::Avif;
use imgo::collect_images;
use imgo::jxl::Jxl;
use imgo::magick::CleanScan;
use imgo::magick::Denoise;
use imgo::scramble_image;
use indicatif::ProgressBar;
use indicatif::ProgressStyle;
use ino_color::ceprintln;
use ino_color::fg::BrightBlue;
use ino_color::fg::Red;
use ino_color::fg::Yellow;
use itertools::izip;
use parking_lot::Mutex;
use rayon::ThreadPoolBuilder;
use std::time::Duration;
use tempfile::NamedTempFile;
use tracing::debug;
use tracing::debug_span;

/// Batch converting pictures between formats.
#[derive(Debug)]
#[derive(clap::Parser)]
#[command(disable_help_subcommand = true)]
enum CliOpts {
    /// (Lossy) Encode pictures into AVIF.
    #[command(visible_alias = "a")]
    Avif {
        #[command(flatten)]
        transcoder: Avif,
        #[clap(flatten)]
        shared: SharedOpts,
    },

    /// (Lossless) Encode pictures into JXL.
    #[command(visible_alias = "j")]
    Jxl {
        #[command(flatten)]
        transcoder: Jxl,
        #[clap(flatten)]
        shared: SharedOpts,
    },

    #[command(visible_alias = "d")]
    Denoise {
        #[command(flatten)]
        transcoder: Denoise,
        #[clap(flatten)]
        shared: SharedOpts,
    },

    /// Sharpen poorly scanned manga to have crispy dots.
    #[command(visible_alias = "c")]
    CleanScan {
        #[command(flatten)]
        transcoder: CleanScan,
        #[clap(flatten)]
        shared: SharedOpts,
    },

    /// 番茄图: scramble/descramble images via a Gilbert-curve pixel
    /// permutation. Output is always PNG (lossless).
    #[command(visible_alias = "t")]
    Tomato {
        #[command(flatten)]
        tomato: TomatoOpts,
        #[clap(flatten)]
        shared: SharedOpts,
    },

    /// Generate shell completion.
    GenComplete {
        #[clap(short, long)]
        shell: clap_complete::Shell,
    },
}

#[derive(clap::Args)]
#[derive(Debug)]
struct SharedOpts {
    /// The starting point for finding images. Also the backup
    /// folder will be created here.
    /// Defaults to `PWD`.
    #[arg(long, short = 'W')]
    workspace: Option<PathBuf>,

    /// Leaving original pictures at the place after transcoding
    /// skipping backup.
    #[arg(long, short = 'N')]
    #[arg(default_value_t = false)]
    no_backup: bool,

    /// Number of parallel transcoding to run.
    /// The default job count is transcoder dependent.
    #[arg(long, short = 'J')]
    jobs: Option<NonZeroU64>,

    /// Do not recurse into subdirectories when collecting images.
    /// Only images from the workspace or current directory will be processed.
    #[arg(long, short = 'R')]
    #[arg(default_value_t = false)]
    no_recursive: bool,

    /// Manually choose pictures to transcode.
    /// This also disables backup.
    // #[arg(last = true)]
    manual_selection: Option<Vec<PathBuf>>,
}

/// Options for the `tomato` subcommand.
#[derive(clap::Args, Debug)]
struct TomatoOpts {
    /// Scramble (obfuscate) the image. Exactly one of `--encrypt` /
    /// `--decrypt` must be given.
    #[arg(long)]
    encrypt: bool,

    /// Descramble (restore) the image. Exactly one of `--encrypt` /
    /// `--decrypt` must be given.
    #[arg(long)]
    decrypt: bool,

    /// Key controlling the offset along the Gilbert curve.
    /// The same key is required to reverse scrambling.
    #[arg(long, default_value_t = 1.0)]
    key: f64,
}

impl TomatoOpts {
    /// Resolves the encrypt/decrypt pair, erroring if not exactly one
    /// is set.
    fn mode(&self) -> anyhow::Result<bool> {
        match (self.encrypt, self.decrypt) {
            (true, false) => Ok(true),
            (false, true) => Ok(false),
            (false, false) => {
                bail!("Exactly one of --encrypt / --decrypt is required")
            }
            (true, true) => {
                bail!("--encrypt and --decrypt are mutually exclusive")
            }
        }
    }
}

/// Shared orchestration core: owns backup, progress, parallel
/// execution, temp files, and output resolution. The `execute` closure
/// does the actual work (spawn command or transform pixels) and writes
/// its result to the given temp path.
///
/// `execute` receives `&ProgressBar` so it can emit progress-aware
/// warnings (e.g. lossy downconversion). It must not touch the
/// filesystem beyond the temp path.
#[expect(dead_code)] // Wired up in step 6
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
#[expect(dead_code)] // Wired up in step 6
fn run_pipeline_external(
    workspace: &Path,
    images: Vec<Image>,
    shared: &SharedOpts,
    transcoder: &dyn External,
) -> anyhow::Result<()> {
    let no_backup = shared.no_backup || shared.manual_selection.is_some();
    let jobs = shared.jobs.unwrap_or_else(|| transcoder.default_jobs());
    let output_format = transcoder.output_format();

    ceprintln!(Yellow, "[Transcoder is {}]", transcoder.id());

    orchestrate(
        workspace,
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
#[expect(dead_code)] // Wired up in step 6
fn run_pipeline_pixel(
    workspace: &Path,
    images: Vec<Image>,
    shared: &SharedOpts,
    transcoder: &dyn Pixel,
) -> anyhow::Result<()> {
    let no_backup = shared.no_backup || shared.manual_selection.is_some();
    let jobs = shared.jobs.unwrap_or_else(|| transcoder.default_jobs());
    let output_format = transcoder.output_format();

    ceprintln!(Yellow, "[Transcoder is {}]", transcoder.id());

    orchestrate(
        workspace,
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

fn main() -> anyhow::Result<()> {
    ino_tracing::init_tracing_subscriber();
    let cliopts = <CliOpts as clap::Parser>::parse();

    // Raise nofile limit to max to avoid "too many open files" errors
    if let Ok((_, hard)) = Resource::NOFILE.get() {
        let _ = Resource::NOFILE.set(hard, hard);
    }

    // Get transcoder and opts from cliopts.
    let (transcoder, shared_opts): (&dyn Transcoder, &SharedOpts) =
        match &cliopts {
            CliOpts::GenComplete { shell } => {
                debug!("Generate shell completion");
                clap_complete::generate(
                    *shell,
                    &mut <CliOpts as clap::CommandFactory>::command(),
                    "i",
                    &mut std::io::stdout(),
                );
                std::process::exit(0);
            }
            CliOpts::Tomato { tomato, shared } => {
                let mode = tomato.mode()?;
                return run_tomato(tomato.key, mode, shared);
            }
            CliOpts::Avif { transcoder, shared } => {
                (transcoder as &dyn Transcoder, shared)
            }
            CliOpts::Jxl { transcoder, shared } => {
                (transcoder as &dyn Transcoder, shared)
            }
            CliOpts::Denoise { transcoder, shared } => {
                (transcoder as &dyn Transcoder, shared)
            }
            CliOpts::CleanScan { transcoder, shared } => {
                (transcoder as &dyn Transcoder, shared)
            }
        };

    ceprintln!(Yellow, "[Transcoder is {}]", transcoder.id());

    // Initialize states
    let workspace = {
        let pwd = std::env::current_dir()?;
        shared_opts.workspace.as_ref().map_or(pwd, Clone::clone)
    };

    let input_formats = transcoder.input_formats();
    let output_format = transcoder.output_format();

    // Try to collect images
    let images = if let Some(man_sel) = &shared_opts.manual_selection {
        debug!("Use manually chosen images");
        let mut accu = vec![];
        for sel in man_sel {
            if sel.is_dir() {
                if shared_opts.no_recursive {
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
                    !shared_opts.no_recursive,
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
        collect_images(
            &workspace,
            input_formats,
            !shared_opts.no_recursive,
        )
        .context("Failed to collect images")?
    };

    // Backup dir
    let backup_dir = Arc::new({
        let dir = workspace.join(BACKUP_DIR_NAME);
        if shared_opts.manual_selection.is_none()
            && !shared_opts.no_backup
            && !images.is_empty()
        {
            std::fs::create_dir_all(&dir)?;
        }
        dir
    });

    let no_backup =
        shared_opts.no_backup || shared_opts.manual_selection.is_some();

    // Execute the transcoding tasks
    let jobs = shared_opts
        .jobs
        .unwrap_or_else(|| transcoder.default_jobs());

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

    // Pre-compute all transcoding tasks before entering thread pool scope
    // to avoid Sync requirement on transcoder
    let Some(output_ext) = output_format.exts().first() else {
        bail!("[BUG] Output format has no ext");
    };

    let tasks: Vec<_> = images
        .into_iter()
        .map(|i| -> anyhow::Result<_> {
            let temp_output =
                NamedTempFile::with_suffix(format!(".{output_ext}"))
                    .context("Failed to create tempfile")?;
            debug!(
                "Temporary output path {}",
                temp_output.path().display()
            );

            let input_path = i.path.original_path();
            let cmd =
                transcoder.transcode(&input_path, temp_output.path());

            Ok((i, input_path, temp_output, cmd))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    thread_pool.scope(|scope| -> anyhow::Result<()> {
        enum Permit {
            Go,
            Cancel,
        }

        let permit = Arc::new(Mutex::new(Permit::Go));

        for (
            (image, input_path, temp_output, mut cmd),
            permit,
            bar,
            backup_dir,
        ) in izip!(
            tasks,
            repeat(permit),
            repeat(progress_bar.clone()),
            repeat(backup_dir)
        ) {
            scope.spawn(move |_| {
                if matches!(*permit.lock(), Permit::Cancel) {
                    debug!("Transcode jobs cancelled");
                    return;
                }
                let _g = debug_span!("transcoding", ?image).entered();

                bar.suspend(|| {
                    ceprintln!(
                        BrightBlue,
                        "Transcoding: {}",
                        input_path.display()
                    );
                });

                let output = match cmd.output() {
                    Ok(output) => output,
                    Err(e) => {
                        bar.suspend(|| {
                            ceprintln!(
                                Red,
                                "Failed to spawn transcoder. Error: {e}"
                            );
                        });
                        *permit.lock() = Permit::Cancel;
                        bar.inc(1);
                        return;
                    }
                };

                if !output.status.success() {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    bar.suspend(|| {
                        ceprintln!(
                            Red,
                            "Transcoding failed for {}:\nstdout: {}\nstderr: {}",
                            input_path.display(),
                            stdout,
                            stderr
                        );
                    });
                    *permit.lock() = Permit::Cancel;
                    bar.inc(1);
                    return;
                }

                // Get the destination directory (same as source)
                let Some(dest_dir) = image.path.parent_dir() else {
                    bar.suspend(|| {
                        ceprintln!(Red, "[BUG] Failed to get parent directory");
                    });
                    bar.inc(1);
                    return;
                };

                // Backup source BEFORE resolving destination path
                // This frees up the original filename when source and output have the same extension
                if !no_backup {
                    let backup_path = image.path.backup_path_structure(&backup_dir);

                    // Create backup directory structure
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

                    // Move source to backup
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

                // Build output filename with new extension, resolving conflicts
                let mut output_extra =
                    image.extra.set_ext(&format!(".{output_ext}"));
                let mut dest_path =
                    dest_dir.join(output_extra.to_filename());

                // Handle filename conflicts by incrementing seq
                while dest_path.exists() {
                    debug!(
                        r#"Destination "{}" exists, incrementing seq to avoid conflict"#,
                        dest_path.display()
                    );
                    output_extra = output_extra.increment_seq();
                    dest_path = dest_dir.join(output_extra.to_filename());
                }

                debug!(
                    r#"Copy output from "{}" to "{}""#,
                    temp_output.path().display(),
                    dest_path.display()
                );

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

/// Image formats the `tomato` subcommand can decode (via the `image`
/// crate). AVIF/JXL are excluded because the `image` crate isn't built
/// with those decoders here.
const TOMATO_INPUT_FORMATS: [ImageFormat; 4] = [
    ImageFormat::PNG,
    ImageFormat::JPG,
    ImageFormat::WEBP,
    ImageFormat::GIF,
];

/// Output extension for the tomato subcommand (always lossless PNG).
const TOMATO_OUTPUT_EXT: &str = "png";

/// Runs the 番茄图 scramble/descramble pipeline.
///
/// `encrypt == true` scrambles, `false` descrambles. Output is always
/// PNG so the permutation survives lossless round-trips.
fn run_tomato(
    key: f64,
    encrypt: bool,
    shared: &SharedOpts,
) -> anyhow::Result<()> {
    let action = if encrypt {
        "Scrambling"
    } else {
        "Descrambling"
    };
    ensure!(
        key.is_finite() && key >= 0.0,
        "--key must be finite and non-negative (got {key})"
    );
    ceprintln!(Yellow, "[Tomato: {action}, key={key}]");
    ceprintln!(
        Yellow,
        "Note: metadata (EXIF/ICC) is stripped; GIF uses first frame only."
    );

    let workspace = {
        let pwd = std::env::current_dir()?;
        shared.workspace.as_ref().map_or(pwd, Clone::clone)
    };

    // Collect images (same discovery logic as the transcoder path).
    let images = if let Some(man_sel) = &shared.manual_selection {
        debug!("Use manually chosen images");
        let mut accu = vec![];
        for sel in man_sel {
            if sel.is_dir() {
                if shared.no_recursive {
                    continue;
                }
                let collected = collect_images(
                    sel,
                    &TOMATO_INPUT_FORMATS,
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
                    TOMATO_INPUT_FORMATS.contains(&format),
                    "Tomato can't decode {} (supported: PNG/JPG/WEBP/GIF)",
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
        collect_images(
            &workspace,
            &TOMATO_INPUT_FORMATS,
            !shared.no_recursive,
        )
        .context("Failed to collect images")?
    };

    if images.is_empty() {
        ceprintln!(Yellow, "No images to process.");
        return Ok(());
    }

    let no_backup = shared.no_backup || shared.manual_selection.is_some();

    let backup_dir = Arc::new({
        let dir = workspace.join(BACKUP_DIR_NAME);
        if !no_backup {
            std::fs::create_dir_all(&dir)?;
        }
        dir
    });

    let jobs = shared.jobs.unwrap_or_else(|| {
        NonZeroU64::new(
            std::thread::available_parallelism()
                .map_or(1, |n| n.get() as u64),
        )
        .unwrap_or(NonZeroU64::MIN)
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

    thread_pool.scope(|scope| -> anyhow::Result<()> {
        for (image, bar, backup_dir) in izip!(
            images,
            repeat(progress_bar.clone()),
            repeat(backup_dir),
        ) {
            scope.spawn(move |_| {
                let _g = debug_span!("tomato", ?image).entered();
                let input_path = image.path.original_path();

                bar.suspend(|| {
                    ceprintln!(
                        BrightBlue,
                        "{action}: {}",
                        input_path.display()
                    );
                });

                // GIF: `image::open` loads only the first frame.
                if image.format == ImageFormat::GIF {
                    bar.suspend(|| {
                        ceprintln!(
                            Yellow,
                            "{}: GIF, only first frame processed",
                            input_path.display()
                        );
                    });
                }

                let report_err = |bar: &ProgressBar, msg: String| {
                    bar.suspend(|| ceprintln!(Red, "{msg}"));
                    bar.inc(1);
                };

                // Decode -> RGBA8 -> scramble -> encode PNG into a temp file.
                let temp_output = match NamedTempFile::with_suffix(
                    format!(".{TOMATO_OUTPUT_EXT}"),
                ) {
                    Ok(t) => t,
                    Err(e) => {
                        report_err(
                            &bar,
                            format!(
                                "Failed to create tempfile for {}: {e}",
                                input_path.display()
                            ),
                        );
                        return;
                    }
                };

                let mut rgba = match image::open(&input_path) {
                    Ok(img) => {
                        // Warn about lossy downconversion for >8-bit
                        // inputs: the algorithm is lossless, but the
                        // RGBA8 pipeline truncates deep pixels.
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
                        img.to_rgba8()
                    }
                    Err(e) => {
                        report_err(
                            &bar,
                            format!(
                                "Failed to decode {}: {e}",
                                input_path.display()
                            ),
                        );
                        return;
                    }
                };

                scramble_image(&mut rgba, key, encrypt);

                if let Err(e) = rgba.save(temp_output.path()) {
                    report_err(
                        &bar,
                        format!(
                            "Failed to encode PNG for {}: {e}",
                            input_path.display()
                        ),
                    );
                    return;
                }

                // Resolve destination dir + filename (PNG), backing up
                // the source first when not disabled.
                let Some(dest_dir) = image.path.parent_dir() else {
                    report_err(
                        &bar,
                        format!(
                            "[BUG] Failed to get parent directory for {}",
                            input_path.display()
                        ),
                    );
                    return;
                };

                if !no_backup {
                    let backup_path =
                        image.path.backup_path_structure(&backup_dir);
                    if let Some(backup_parent) = backup_path.parent()
                        && let Err(e) = create_dir_all(backup_parent)
                    {
                        report_err(
                            &bar,
                            format!(
                                "Failed to create backup dir {}: {e}",
                                backup_parent.display()
                            ),
                        );
                        return;
                    }
                    if let Err(e) = rename(&input_path, &backup_path) {
                        report_err(
                            &bar,
                            format!(
                                "Failed to backup {}: {e}",
                                input_path.display()
                            ),
                        );
                        return;
                    }
                    debug!("Backed up to {}", backup_path.display());
                }

                let mut output_extra =
                    image.extra.set_ext(&format!(".{TOMATO_OUTPUT_EXT}"));
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
                    report_err(
                        &bar,
                        format!(
                            "Failed to copy output to {}: {e}",
                            dest_path.display()
                        ),
                    );
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
