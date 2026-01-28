use std::fs::create_dir_all;
use std::fs::rename;
use std::iter::repeat;
use std::num::NonZeroU64;
use std::path::PathBuf;
use std::sync::Arc;

use rlimit::Resource;

use anyhow::Context;
use anyhow::bail;
use imgo::BACKUP_DIR_NAME;
use imgo::BaseSeqExt;
use imgo::Image;
use imgo::ImageFormat;
use imgo::RelAbs;
use imgo::Transcoder;
use imgo::avif::Avif;
use imgo::collect_images;
use imgo::jxl::Jxl;
use imgo::magick::CleanScan;
use imgo::magick::Despeckle;
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

    /// Despeckle using imagemagick `-despeckle` function.
    #[command(visible_alias = "d")]
    Despeckle {
        #[command(flatten)]
        transcoder: Despeckle,
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
    non_recursive: bool,

    /// Manually choose pictures to transcode.
    /// This also disables backup.
    // #[arg(last = true)]
    manual_selection: Option<Vec<PathBuf>>,
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
            CliOpts::Avif { transcoder, shared } => {
                (transcoder as &dyn Transcoder, shared)
            }
            CliOpts::Jxl { transcoder, shared } => {
                (transcoder as &dyn Transcoder, shared)
            }
            CliOpts::Despeckle { transcoder, shared } => {
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
                if shared_opts.non_recursive {
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
                    !shared_opts.non_recursive,
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
            !shared_opts.non_recursive,
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
            repeat(progress_bar),
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

    Ok(())
}
