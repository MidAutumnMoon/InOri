use std::fs::create_dir_all;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result as AnyResult;
use anyhow::bail;
use anyhow::ensure;
use ino_path::PathExt;
use ino_result::ResultExt;
use rand::Rng;
use tap::Pipe;
use tracing::debug;

use crate::fs::collect_pictures;

mod avif;
mod despeckle;
mod fs;
mod jxl;

/// Name of the directory for storing original pictures.
pub const BACKUP_DIR_NAME: &str = ".backup";

/// Name of the directory for stashing temporary files.
/// The work directory should be on the same filesystem
/// as the root directory to avoid cross fs moving.
pub const WORK_DIR_NAME: &str = ".work";

/// Tag the transcoded pictures with this name in xattr.
pub const XATTR_TRANSCODE_OUTPUT: &str = "user.avxl-output";

// ...unused
pub const XATTR_BACKUP_DIR: &str = "user.avxl-backup-dir";
pub const XATTR_WORK_DIR: &str = "user.avxl-work-dir";

/// Batch converting pictures between formats.
#[derive(Debug)]
#[derive(clap::Parser)]
#[command(disable_help_subcommand = true)]
enum CliOpts {
    /// (Lossy) Encode pictures into AVIF.
    Avif {
        #[command(flatten)]
        transcoder: avif::Avif,
        #[command(flatten)]
        shared: SharedCliOpts,
    },

    /// (Lossless) Encode pictures into JXL.
    Jxl {
        #[command(flatten)]
        transcoder: jxl::Jxl,
        #[command(flatten)]
        shared: SharedCliOpts,
    },

    /// Despeckle pictures using imagemagick.
    Despeckle {
        #[command(flatten)]
        transcoder: despeckle::Despeckle,
        #[command(flatten)]
        shared: SharedCliOpts,
    },

    /// Sharpen poorly scanned manga to have crispy dots.
    SharpenScan,

    /// (unimplemented) Print various information for scripting.
    Print,

    /// Generate shell completion.
    Complete {
        #[arg(long, short)]
        shell: clap_complete::Shell,
    },
    // Dwebp?
    // Pipeline?
}

#[derive(clap::Args, Debug)]
struct SharedCliOpts {
    /// (to write...)
    /// Defaults to PWD.
    #[arg(long, short = 'R')]
    root_dir: Option<PathBuf>,

    /// Skip putting original pictures into backup directory
    /// after transcoding.
    #[arg(long, short = 'B')]
    #[arg(default_value_t = false)]
    no_backup: bool,

    /// (unimplemented) Number of parallel transcoding to run.
    #[arg(long, short)]
    #[arg(default_value = "1")]
    jobs: usize,

    /// Display logs from transcoders.
    #[arg(long, short = 'L')]
    #[arg(default_value_t = false)]
    show_logs: bool,

    /// Manually choose pictures to transcode.
    #[arg(last = true)]
    manual_selection: Option<Vec<PathBuf>>,
}

impl CliOpts {
    fn unwrap(self) -> AnyResult<(Box<dyn Transcoder>, SharedCliOpts)> {
        // TODO: reduce the boilerplate?
        let (t, s) = match self {
            Self::Avif { transcoder, shared } => {
                (Box::new(transcoder) as Box<dyn Transcoder>, shared)
            }
            Self::Jxl { transcoder, shared } => {
                (Box::new(transcoder) as Box<dyn Transcoder>, shared)
            }
            Self::Despeckle { transcoder, shared } => {
                (Box::new(transcoder) as Box<dyn Transcoder>, shared)
            }
            Self::SharpenScan => todo!(),
            Self::Print => todo!(),
            Self::Complete { .. } => {
                bail!("[BUG] Shouldn't unwrap Complete")
            }
        };
        Ok((t, s))
    }

    fn parse() -> Self {
        <Self as clap::Parser>::parse()
    }
}

struct App {
    transcoder: Box<dyn Transcoder>,
    root_dir: PathBuf,
    backup_dir: PathBuf,
    work_dir: PathBuf,
    no_backup: bool,
    show_logs: bool,
    pictures: Vec<(PathBuf, PictureFormat)>,
}

impl TryFrom<CliOpts> for App {
    type Error = anyhow::Error;

    #[tracing::instrument(name = "app_from_cliopts", skip_all)]
    fn try_from(cliopts: CliOpts) -> AnyResult<Self> {
        let (transcoder, opts) = cliopts.unwrap()?;

        let root_dir = opts.root_dir.unwrap_or(
            std::env::current_dir().context("Failed to get pwd")?,
        );
        ensure! { root_dir.is_absolute(),
            r#"`root_dir` must be abosulte, but got "{}""#,
            root_dir.display()
        };

        let backup_dir = root_dir.join(BACKUP_DIR_NAME);
        let work_dir = root_dir.join(WORK_DIR_NAME);

        let pictures = if let Some(selection) = opts.manual_selection {
            debug!("normalize manual selection");
            let mut accu = vec![];
            for sel in selection {
                let path = if sel.is_absolute() {
                    sel
                } else {
                    root_dir.join(sel)
                };
                if path.is_dir_no_traverse()? {
                    accu.append(&mut collect_pictures(
                        &path,
                        transcoder.input_format(),
                    ));
                } else if let Some(format) =
                    PictureFormat::from_path(&path)
                {
                    accu.push((path, format));
                } else {
                    debug!(?path, "path skipped");
                }
            }
            accu
        } else {
            debug!("no selection, collect pictures");
            collect_pictures(&root_dir, transcoder.input_format())
        };

        ensure! { pictures.iter().all(|(pic, _)| pic.is_absolute()),
            "[BUG] Some picture paths are not absolute"
        };

        ensure! { pictures.iter().all(|(pic, _)| pic.is_file()),
            "[BUG] Some picture paths are not file"
        };

        Ok(Self {
            transcoder,
            root_dir,
            backup_dir,
            work_dir,
            no_backup: opts.no_backup,
            show_logs: opts.show_logs,
            pictures,
        })
    }
}

/// A transcoder with its various information.
trait Transcoder {
    /// A short and descriptive name for this transcoder.
    fn id(&self) -> &'static str;

    /// The picture formats that this transcoder accepts as input.
    fn input_format(&self) -> &'static [PictureFormat];

    /// The picture format that this transcoder outputs.
    fn output_format(&self) -> PictureFormat;

    /// Build the command to do transcoding.
    // This does count as some sort of sans-io lol
    // TODO: Switch to async?
    #[allow(clippy::missing_errors_doc)]
    fn generate_command(
        &self,
        input: &Path,
        output: &Path,
    ) -> pty_process::blocking::Command;
}

/// Commonly encountered image formats.
#[derive(Debug)]
#[derive(strum::EnumIter)]
pub enum PictureFormat {
    PNG,
    JPG,
    WEBP,
    AVIF,
    JXL,
    GIF,
}

impl PictureFormat {
    /// Extensions of each image format.
    #[must_use]
    #[inline]
    pub fn exts(&self) -> &'static [&'static str] {
        match self {
            Self::PNG => &["png"],
            Self::JPG => &["jpg", "jpeg"],
            Self::WEBP => &["webp"],
            Self::AVIF => &["avif"],
            Self::JXL => &["jxl"],
            Self::GIF => &["gif"],
        }
    }

    #[must_use]
    #[inline]
    pub fn ext_matches(&self, theirs: &str) -> bool {
        self.exts().contains(&theirs)
    }

    /// Guess the picture's format based on the extension of
    /// the path.
    #[inline]
    #[must_use]
    pub fn from_path(path: &Path) -> Option<Self> {
        use strum::IntoEnumIterator;
        if let Some(ext) = path.extension()
            && let Some(ext) = ext.to_str()
        {
            Self::iter().find(|fmt| fmt.ext_matches(ext))
        } else {
            None
        }
    }
}

fn main_with_result() -> AnyResult<()> {
    let opts = CliOpts::parse();

    if let CliOpts::Complete { shell } = opts {
        use clap::CommandFactory;
        use clap_complete::generate;
        debug!("generate shell completion");
        let mut cmd = CliOpts::command();
        // TODO: don't hardcode program name
        generate(shell, &mut cmd, "avxl", &mut std::io::stdout());
        return Ok(());
    }

    App::try_from(opts)
        .context("Failed to create app")?
        .pipe(run_app)
        .context("Error happended when running app")?;

    Ok(())
}

fn main() {
    ino_tracing::init_tracing_subscriber();
    main_with_result().print_error_exit_process();
}

fn run_app(app: App) -> AnyResult<()> {
    let App {
        transcoder,
        root_dir,
        backup_dir,
        work_dir,
        no_backup,
        show_logs,
        pictures,
        ..
    } = app;

    if !pictures.is_empty() {
        if !backup_dir.try_exists_no_traverse()? {
            debug!("create backup dir");
            create_dir_all(&backup_dir)?;
        }
        if !work_dir.try_exists_no_traverse()? {
            debug!("create work dir");
            create_dir_all(&work_dir)?;
        }
    }

    // TODO: async?
    for (pic, _format) in pictures {
        // If the picture is under root_dir then
        // strip the prefix to make the paths shorter in backup_dir.
        // If not, just give up.
        let backup = pic.strip_prefix(&root_dir).map_or_else(
            |_| backup_dir.join(&pic),
            |suffix| backup_dir.join(suffix),
        );

        let [output_ext, ..] = transcoder.output_format().exts() else {
            bail!("[BUG] Transcoder implements no output format")
        };
        let tempfile = tempfile_in_workdir(&work_dir, output_ext);

        let cmd = transcoder.generate_command(&pic, &tempfile);
    }

    todo!()
}

// TODO: Name clash is not handled, but on real hardware
// it probably won't happen within the lifespan of Rust.
#[inline]
fn tempfile_in_workdir(work_dir: &Path, ext: &str) -> PathBuf {
    use rand::distr::Alphanumeric;
    let prefix = rand::rng()
        .sample_iter(Alphanumeric)
        .take(8)
        .map(char::from)
        .collect::<String>();
    work_dir.join(format!("{prefix}.{ext}"))
}
