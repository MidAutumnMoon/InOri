use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result as AnyResult;
use anyhow::bail;
use anyhow::ensure;
use ino_path::PathExt;
use tap::Pipe;
use tracing::debug;

use crate::fs::collect_pictures;

mod avif;
mod despeckle;
mod fs;
mod jxl;
mod runner;

/// Name of the directory for storing original pictures.
pub const BACKUP_DIR_PREFIX: &str = ".backup";

/// Name of the directory for stashing temporary files.
/// The work directory should be on the same filesystem
/// as the root directory to avoid cross fs moving.
pub const WORK_DIR_NAME: &str = ".work";

/// Tag the transcoded pictures with this name in xattr.
pub const XATTR_TRANSCODE_OUTPUT: &str = "user.avxl-output";

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
    #[arg(long, short = 'r')]
    root_dir: Option<PathBuf>,

    #[arg(long, short = 'R')]
    no_recursive: bool,

    /// Leaving original pictures at the place after transcoding
    /// for manual comparison.
    #[arg(long, short = 'C')]
    #[arg(default_value_t = false)]
    compare: bool,

    /// (unimplemented) Number of parallel transcoding to run.
    #[arg(long, short = 'J')]
    #[arg(default_value = "1")]
    jobs: usize,

    /// Show logs from transcoders.
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
    pictures: Vec<(PathBuf, PicFormat)>,
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

        let backup_dir = root_dir.join(BACKUP_DIR_PREFIX);
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
                } else if let Some(format) = PicFormat::from_path(&path) {
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
            no_backup: opts.compare,
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
    fn input_format(&self) -> &'static [PicFormat];

    /// The picture format that this transcoder outputs.
    fn output_format(&self) -> PicFormat;

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

struct Picture {
    format: PicFormat,
    path: PicPath,
}

enum PicPath {
    Absolute { path: PathBuf },
    Relative { root: PathBuf, path: PathBuf },
}

/// Commonly encountered image formats.
#[derive(Debug)]
#[derive(strum::EnumIter)]
#[allow(clippy::upper_case_acronyms)]
enum PicFormat {
    PNG,
    JPG,
    WEBP,
    AVIF,
    JXL,
    GIF,
}

impl PicFormat {
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

    /// Guess the picture's format based on the extension of
    /// the path.
    #[inline]
    #[must_use]
    pub fn from_path(path: &Path) -> Option<Self> {
        use strum::IntoEnumIterator;
        if let Some(ext) = path.extension()
            && let Some(ext) = ext.to_str()
        {
            Self::iter().find(|fmt| fmt.exts().contains(&ext))
        } else {
            None
        }
    }
}

fn main() -> AnyResult<()> {
    ino_tracing::init_tracing_subscriber();
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
        .pipe(runner::run_app)
        .context("Error happended when running app")
}
