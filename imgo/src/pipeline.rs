//! Transactional image execution shared by manual commands and automation.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs::OpenOptions;
use std::fs::create_dir_all;
use std::fs::rename;
use std::num::NonZeroU64;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use bpaf::Parser;
use bpaf::construct;
use bpaf::long;
use bpaf::positional;
use indicatif::ProgressBar;
use indicatif::ProgressStyle;
use ino_color::ceprintln;
use ino_color::fg::BrightBlue;
use ino_color::fg::Red;
use ino_color::fg::Yellow;
use rayon::ThreadPoolBuilder;
use rayon::prelude::*;
use rootcause::bail;
use rootcause::option_ext::OptionExt as _;
use rootcause::prelude::ResultExt as _;
use rootcause::report_collection::ReportCollection;
use tempfile::Builder;
use tracing::debug;

use crate::fs::backup_path;
use crate::fs::collect_images;
use crate::fs::destination_path;
use crate::fs::selected_image;
use crate::img::Image;
use crate::img::ImageFormat;
use crate::recipe::Recipe;
use crate::transcoder::Pixel;

/// Shared CLI options for direct, single-recipe commands.
#[derive(Debug)]
pub struct SharedOpts {
    /// Starting point for discovery and the `.backup` tree. Defaults to PWD.
    pub workspace: Option<PathBuf>,

    /// Keep originals in place instead of moving them to `.backup`.
    pub no_backup: bool,

    /// Number of images processed concurrently. Encoders may use multiple
    /// threads internally, so expensive encoders default to one image at once.
    pub jobs: Option<NonZeroU64>,

    /// Only discover immediate children of each selected directory.
    pub no_recursive: bool,

    /// Explicit files or directories. Manual selection keeps originals.
    pub manual_selection: Option<Vec<PathBuf>>,
}

impl SharedOpts {
    #[must_use]
    pub fn skips_backup(&self) -> bool {
        self.no_backup || self.manual_selection.is_some()
    }
}

/// CLI parser for [`SharedOpts`].
#[must_use]
pub fn shared_cli() -> impl Parser<SharedOpts> {
    let workspace = long("workspace")
        .short('W')
        .argument::<PathBuf>("DIR")
        .help("Starting point for discovery and the `.backup` tree. Defaults to PWD")
        .optional();
    let no_backup = long("no-backup").short('N').switch().help(
        "Keep originals in place instead of moving them to `.backup`",
    );
    let jobs = long("jobs")
        .short('J')
        .argument::<NonZeroU64>("N")
        .help(
            "Number of images processed concurrently. Encoders may use \
             multiple threads internally, so expensive encoders default to \
             one image at once",
        )
        .optional();
    let no_recursive = long("no-recursive").short('R').switch().help(
        "Only discover immediate children of each selected directory",
    );
    let manual_selection = positional::<PathBuf>("PATH")
        .help("Explicit files or directories. Manual selection keeps originals")
        .some("expect at least one image path or directory")
        .optional();
    construct!(SharedOpts {
        workspace,
        no_backup,
        jobs,
        no_recursive,
        manual_selection,
    })
}

#[derive(Debug, Clone)]
pub struct Job {
    pub image: Image,
    pub recipe: Arc<Recipe>,
    /// Reviewed encoded bytes that already implement `recipe` for this image.
    /// Missing review artifact remains a normal recipe execution.
    pub reviewed_output: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BatchSummary {
    pub processed: usize,
    pub already_complete: usize,
}

#[derive(Debug)]
enum PreparedState {
    Complete,
    Pending(PendingJob),
}

#[derive(Debug)]
struct PendingJob {
    input: PathBuf,
    destination: PathBuf,
    source_commit: SourceCommit,
}

#[derive(Debug)]
enum SourceCommit {
    Keep,
    MoveToBackup(PathBuf),
}

/// Run heterogeneous recipe jobs.
///
/// All recipes, tools, paths, and output collisions are checked before the
/// first source is moved.
///
/// # Errors
///
/// Fails preflight on invalid recipes, missing tools, stale filesystem state,
/// or output collisions. After execution starts, returns a combined error if
/// any image failed; successful images remain committed and resumable.
pub fn run_jobs(
    workspace: &Path,
    jobs: Vec<Job>,
    no_backup: bool,
    parallelism: NonZeroU64,
) -> rootcause::Result<BatchSummary> {
    let mut tools = BTreeSet::new();
    for job in &jobs {
        job.recipe.validate_for(job.image.format).context_with(|| {
            format!("validate recipe for {}", job.image.path.display())
        })?;
        if let Some(reviewed) = &job.reviewed_output {
            if !std::fs::metadata(reviewed).is_ok_and(|metadata| {
                metadata.is_file() && metadata.len() > 0
            }) {
                bail!(
                    "reviewed output is missing or empty: {}",
                    reviewed.display()
                );
            }
        } else {
            job.recipe.required_tools(&mut tools);
        }
    }
    for tool in tools {
        tool.verify()?;
    }

    orchestrate(
        workspace,
        jobs,
        no_backup,
        parallelism,
        |job| &job.image,
        |job| job.recipe.validate_for(job.image.format),
        |job, input, output| {
            if let Some(reviewed) = &job.reviewed_output {
                std::fs::copy(reviewed, output).context_with(|| {
                    format!("reuse reviewed output {}", reviewed.display())
                })?;
                Ok(vec![format!(
                    "Reused reviewed candidate for {}",
                    job.image.path.display()
                )])
            } else {
                job.recipe
                    .execute(input, job.image.format, output)
                    .map(|()| Vec::new())
            }
        },
    )
}

/// Run one recipe over files discovered by the manual CLI surface.
///
/// # Errors
///
/// Returns an error when discovery, preflight, execution, backup, or atomic
/// output commit fails.
pub fn run_pipeline_recipe(
    shared: &SharedOpts,
    recipe: Recipe,
) -> rootcause::Result<BatchSummary> {
    let first = recipe
        .first_input_formats()
        .context("recipe has no first step")?;
    let (workspace, images) = collect_for(shared, first)?;
    let parallelism = shared.jobs.unwrap_or_else(|| recipe.default_jobs());
    let recipe = Arc::new(recipe);
    let jobs = images
        .into_iter()
        .map(|image| Job {
            image,
            recipe: Arc::clone(&recipe),
            reviewed_output: None,
        })
        .collect();
    run_jobs(&workspace, jobs, shared.skips_backup(), parallelism)
}

/// Run an in-process pixel transform through the same preflight, backup, and
/// atomic-commit path used by external recipes.
///
/// # Errors
///
/// Returns an error when discovery, decoding, transformation, backup, or
/// atomic output commit fails.
pub fn run_pipeline_pixel(
    shared: &SharedOpts,
    transcoder: &dyn Pixel,
) -> rootcause::Result<BatchSummary> {
    ceprintln!(Yellow, "[Transcoder is {}]", transcoder.id());
    ceprintln!(
        Yellow,
        "Note: metadata (EXIF/ICC) is stripped; GIF uses first frame only."
    );
    let (workspace, images) =
        collect_for(shared, transcoder.input_formats())?;
    let parallelism =
        shared.jobs.unwrap_or_else(|| transcoder.default_jobs());
    let output_format = transcoder.output_format();

    orchestrate(
        &workspace,
        images,
        shared.skips_backup(),
        parallelism,
        |image| image,
        |_| Ok(output_format),
        |image, input, output| {
            let decoded = image::open(input)
                .context_with(|| format!("decode {}", input.display()))?;
            let mut warnings = Vec::new();
            let bits = decoded.color().bits_per_pixel();
            if bits > 32 {
                warnings.push(format!(
                    "{}: {bits}-bit input, downconverting to 8-bit",
                    image.path.display()
                ));
            }
            if image.format == ImageFormat::GIF {
                warnings.push(format!(
                    "{}: only the first GIF frame is processed",
                    image.path.display()
                ));
            }
            let mut rgba = decoded.to_rgba8();
            transcoder.transform(&mut rgba)?;
            rgba.save(output)
                .context_with(|| format!("encode {}", output.display()))?;
            Ok(warnings)
        },
    )
}

fn collect_for(
    shared: &SharedOpts,
    input_formats: &[ImageFormat],
) -> rootcause::Result<(PathBuf, Vec<Image>)> {
    let workspace = shared.workspace.clone().map_or_else(
        std::env::current_dir,
        Ok::<PathBuf, std::io::Error>,
    )?;
    let workspace = workspace.canonicalize().context_with(|| {
        format!("resolve workspace {}", workspace.display())
    })?;

    let mut images = if let Some(selections) = &shared.manual_selection {
        let mut selected = Vec::new();
        for selection in selections {
            let path = if selection.is_absolute() {
                selection.clone()
            } else {
                workspace.join(selection)
            };
            if path.is_dir() {
                selected.extend(collect_images(
                    &path,
                    input_formats,
                    !shared.no_recursive,
                )?);
            } else {
                selected.push(selected_image(
                    &workspace,
                    selection,
                    input_formats,
                )?);
            }
        }
        selected
    } else {
        collect_images(&workspace, input_formats, !shared.no_recursive)?
    };

    images.sort_by(|left, right| left.path.cmp(&right.path));
    images.dedup_by(|left, right| left.path == right.path);
    Ok((workspace, images))
}

fn orchestrate<T, ImageOf, OutputOf, Execute>(
    workspace: &Path,
    items: Vec<T>,
    no_backup: bool,
    parallelism: NonZeroU64,
    image_of: ImageOf,
    output_of: OutputOf,
    execute: Execute,
) -> rootcause::Result<BatchSummary>
where
    T: Send + Sync,
    ImageOf: Fn(&T) -> &Image + Send + Sync,
    OutputOf: Fn(&T) -> rootcause::Result<ImageFormat> + Send + Sync,
    Execute: Fn(&T, &Path, &Path) -> rootcause::Result<Vec<String>>
        + Send
        + Sync,
{
    if items.is_empty() {
        ceprintln!(Yellow, "No images to process.");
        return Ok(BatchSummary::default());
    }

    let prepared = preflight_states(
        workspace, &items, no_backup, &image_of, &output_of,
    )?;
    let bar = progress_bar(items.len())?;
    let thread_count = usize::try_from(parallelism.get())?;
    let pool = ThreadPoolBuilder::new()
        .num_threads(thread_count)
        .build()
        .context("build image worker pool")?;

    let results = pool.install(|| {
        items
            .into_par_iter()
            .zip(prepared.into_par_iter())
            .map(|(item, state)| {
                let image = image_of(&item);
                let result = match state {
                    PreparedState::Complete => Ok(false),
                    PreparedState::Pending(pending) => process_one(
                        image,
                        pending,
                        &bar,
                        |input, temporary| {
                            execute(&item, input, temporary)
                        },
                    )
                    .map(|()| true),
                };
                if let Err(error) = &result {
                    bar.suspend(|| {
                        ceprintln!(
                            Red,
                            "Failed to process {}: {error}",
                            image.path.display()
                        );
                    });
                }
                bar.inc(1);
                result
            })
            .collect::<Vec<_>>()
    });
    bar.finish();

    let mut summary = BatchSummary::default();
    let mut failures = Vec::new();
    for result in results {
        match result {
            Ok(true) => summary.processed += 1,
            Ok(false) => summary.already_complete += 1,
            Err(error) => failures.push(error),
        }
    }
    if !failures.is_empty() {
        let total = failures.len();
        let overflow = total.saturating_sub(10);
        let shown: ReportCollection =
            failures.into_iter().take(10).collect();
        let suffix = if overflow > 0 {
            format!(" (showing the first 10 of {total})")
        } else {
            String::new()
        };
        return Err(shown
            .context(format!(
                "{total} image(s) failed; completed outputs remain \
                 resumable{suffix}",
            ))
            .into());
    }
    Ok(summary)
}

fn preflight_states<T, ImageOf, OutputOf>(
    workspace: &Path,
    items: &[T],
    no_backup: bool,
    image_of: &ImageOf,
    output_of: &OutputOf,
) -> rootcause::Result<Vec<PreparedState>>
where
    ImageOf: Fn(&T) -> &Image,
    OutputOf: Fn(&T) -> rootcause::Result<ImageFormat>,
{
    let mut destinations: BTreeMap<PathBuf, PathBuf> = BTreeMap::new();
    let mut states = Vec::with_capacity(items.len());
    for item in items {
        let image = image_of(item);
        let destination = destination_path(&image.path, output_of(item)?);
        if let Some(previous) =
            destinations.insert(destination.clone(), image.path.clone())
            && previous != image.path
        {
            bail!(
                "{} and {} both map to output {}",
                previous.display(),
                image.path.display(),
                destination.display()
            );
        }
        states.push(prepare_state(
            workspace,
            image,
            destination,
            no_backup,
        )?);
    }
    Ok(states)
}

fn prepare_state(
    workspace: &Path,
    image: &Image,
    destination: PathBuf,
    no_backup: bool,
) -> rootcause::Result<PreparedState> {
    let source = &image.path;
    let backup = backup_path(workspace, source);
    let source_exists = source.is_file();
    let backup_exists = backup.is_file();
    let same_path = destination == *source;
    let destination_exists = destination.is_file();

    if no_backup {
        if !source_exists {
            bail!("source is missing: {}", source.display());
        }
        if !same_path && destination_exists {
            bail!("output already exists: {}", destination.display());
        }
        return Ok(PreparedState::Pending(PendingJob {
            input: source.clone(),
            destination,
            source_commit: SourceCommit::Keep,
        }));
    }

    if same_path {
        return match (source_exists, backup_exists) {
            (true, true) => {
                if std::fs::metadata(source)?.len() == 0 {
                    bail!(
                        "completed output is empty: {}",
                        source.display()
                    );
                }
                Ok(PreparedState::Complete)
            }
            (true, false) => Ok(PreparedState::Pending(PendingJob {
                input: source.clone(),
                destination,
                source_commit: SourceCommit::MoveToBackup(backup),
            })),
            (false, true) => Ok(PreparedState::Pending(PendingJob {
                input: backup,
                destination,
                source_commit: SourceCommit::Keep,
            })),
            (false, false) => {
                bail!(
                    "source and backup are missing: {}",
                    source.display()
                )
            }
        };
    }

    match (source_exists, backup_exists, destination_exists) {
        (true, false, false) => Ok(PreparedState::Pending(PendingJob {
            input: source.clone(),
            destination,
            source_commit: SourceCommit::MoveToBackup(backup),
        })),
        (false, true, true) => {
            if std::fs::metadata(&destination)?.len() == 0 {
                bail!(
                    "completed output is empty: {}",
                    destination.display()
                );
            }
            Ok(PreparedState::Complete)
        }
        (false, true, false) => Ok(PreparedState::Pending(PendingJob {
            input: backup,
            destination,
            source_commit: SourceCommit::Keep,
        })),
        (true, _, true) => bail!(
            "refusing to overwrite existing output {} while source {} remains",
            destination.display(),
            source.display()
        ),
        (true, true, false) => bail!(
            "both source and backup exist for {}; resolve the stale backup first",
            source.display()
        ),
        (false, false, true) => bail!(
            "output {} exists but neither source nor backup can prove it belongs to the job",
            destination.display()
        ),
        (false, false, false) => {
            bail!("source and backup are missing: {}", source.display())
        }
    }
}

fn process_one(
    image: &Image,
    pending: PendingJob,
    bar: &ProgressBar,
    execute: impl FnOnce(&Path, &Path) -> rootcause::Result<Vec<String>>,
) -> rootcause::Result<()> {
    let PendingJob {
        input,
        destination,
        source_commit,
    } = pending;
    bar.suspend(|| {
        ceprintln!(BrightBlue, "Processing: {}", image.path.display());
    });
    let parent = destination
        .parent()
        .context("destination has no parent directory")?;
    let suffix = destination
        .extension()
        .and_then(|extension| extension.to_str())
        .map_or_else(String::new, |extension| format!(".{extension}"));
    let temporary = Builder::new()
        .prefix(".imgo-")
        .suffix(&suffix)
        .tempfile_in(parent)
        .context_with(|| {
            format!("create output beside {}", destination.display())
        })?;

    let warnings = execute(&input, temporary.path())?;
    for warning in warnings {
        bar.suspend(|| ceprintln!(Yellow, "{warning}"));
    }
    let metadata = std::fs::metadata(temporary.path())?;
    if metadata.len() == 0 {
        bail!(
            "encoder produced an empty output for {}",
            image.path.display()
        );
    }
    let source_permissions = std::fs::metadata(&input)?.permissions();
    std::fs::set_permissions(temporary.path(), source_permissions)?;
    // External tools may replace the path's inode. Reopen the encoded path
    // rather than syncing NamedTempFile's original descriptor.
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(temporary.path())
        .context_with(|| {
            format!("open encoded output for {}", image.path.display())
        })?
        .sync_all()
        .context_with(|| {
            format!("flush encoded output for {}", image.path.display())
        })?;

    if let SourceCommit::MoveToBackup(backup) = source_commit {
        if let Some(backup_parent) = backup.parent() {
            create_dir_all(backup_parent).context_with(|| {
                format!(
                    "create backup directory {}",
                    backup_parent.display()
                )
            })?;
        }
        rename(&input, &backup).context_with(|| {
            format!("move {} to {}", input.display(), backup.display())
        })?;
        debug!(path = %backup.display(), "source backed up");
    }

    temporary
        .persist(&destination)
        .map_err(|error| error.error)
        .context_with(|| {
            format!("commit encoded output to {}", destination.display())
        })?;
    Ok(())
}

fn progress_bar(length: usize) -> rootcause::Result<ProgressBar> {
    let bar = ProgressBar::new(u64::try_from(length)?);
    let style = ProgressStyle::with_template(
        "{spinner:.green} [{elapsed_precise}] [{bar:40.blue/gray}] {pos}/{len} ({eta})",
    )?
    .progress_chars("#>-");
    bar.set_style(style);
    bar.enable_steady_tick(Duration::from_millis(100));
    Ok(bar)
}

#[cfg(test)]
#[expect(clippy::panic_in_result_fn, reason = "test assertions")]
#[expect(clippy::unwrap_used, reason = "test verifies a batch error")]
mod tests {
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;

    use super::*;
    use crate::recipe::Step;
    use crate::transcoder::avif::Avif;

    #[test]
    fn commits_once_and_resumes_from_backup_state() -> rootcause::Result<()>
    {
        let temporary = tempfile::tempdir()?;
        let workspace = temporary.path();
        let source = workspace.join("page.png");
        std::fs::write(&source, b"original")?;
        let image = Image {
            path: source.clone(),
            format: ImageFormat::PNG,
        };
        let calls = AtomicUsize::new(0);

        let initial_summary = orchestrate(
            workspace,
            vec![image.clone()],
            false,
            NonZeroU64::MIN,
            |image| image,
            |_| Ok(ImageFormat::AVIF),
            |_, input, output| {
                assert_eq!(input, source);
                calls.fetch_add(1, Ordering::Relaxed);
                std::fs::write(output, b"encoded")?;
                Ok(Vec::new())
            },
        )?;
        assert_eq!(
            initial_summary,
            BatchSummary {
                processed: 1,
                already_complete: 0,
            }
        );
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert!(!source.exists());
        assert_eq!(
            std::fs::read(workspace.join(".backup/page.png"))?,
            b"original"
        );
        assert_eq!(
            std::fs::read(workspace.join("page.avif"))?,
            b"encoded"
        );

        let resumed_summary = orchestrate(
            workspace,
            vec![image],
            false,
            NonZeroU64::MIN,
            |image| image,
            |_| Ok(ImageFormat::AVIF),
            |_, _, _| {
                calls.fetch_add(1, Ordering::Relaxed);
                rootcause::bail!("completed work must not execute again")
            },
        )?;
        assert_eq!(resumed_summary.processed, 0);
        assert_eq!(resumed_summary.already_complete, 1);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        Ok(())
    }

    #[test]
    fn one_failure_does_not_cancel_independent_images()
    -> rootcause::Result<()> {
        let temporary = tempfile::tempdir()?;
        let workspace = temporary.path();
        let failed = workspace.join("a.png");
        let successful = workspace.join("b.png");
        std::fs::write(&failed, b"a")?;
        std::fs::write(&successful, b"b")?;
        let images = [&failed, &successful]
            .into_iter()
            .map(|path| Image {
                path: path.clone(),
                format: ImageFormat::PNG,
            })
            .collect();

        let result = orchestrate(
            workspace,
            images,
            false,
            NonZeroU64::MIN,
            |image| image,
            |_| Ok(ImageFormat::AVIF),
            |image, _, output| {
                if image.path == failed {
                    rootcause::bail!("deliberate failure");
                }
                std::fs::write(output, b"encoded")?;
                Ok(Vec::new())
            },
        );
        result.unwrap_err();
        assert!(failed.exists());
        assert!(!workspace.join(".backup/a.png").exists());
        assert!(!successful.exists());
        assert!(workspace.join(".backup/b.png").exists());
        assert_eq!(std::fs::read(workspace.join("b.avif"))?, b"encoded");
        Ok(())
    }

    #[test]
    fn reviewed_candidate_commits_without_reencoding()
    -> rootcause::Result<()> {
        let temporary = tempfile::tempdir()?;
        let workspace = temporary.path();
        let source = workspace.join("page.png");
        let reviewed = workspace.join("reviewed.avif");
        std::fs::write(&source, b"original")?;
        std::fs::write(&reviewed, b"reviewed encoded bytes")?;

        let summary = run_jobs(
            workspace,
            vec![Job {
                image: Image {
                    path: source.clone(),
                    format: ImageFormat::PNG,
                },
                recipe: Arc::new(Recipe::single(Step::Avif(
                    Avif::default(),
                ))),
                reviewed_output: Some(reviewed),
            }],
            false,
            NonZeroU64::MIN,
        )?;

        assert_eq!(summary.processed, 1);
        assert_eq!(
            std::fs::read(workspace.join("page.avif"))?,
            b"reviewed encoded bytes"
        );
        assert!(!source.exists());
        assert_eq!(
            std::fs::read(workspace.join(".backup/page.png"))?,
            b"original"
        );
        Ok(())
    }
}
