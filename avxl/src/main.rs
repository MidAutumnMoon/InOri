use std::fs::create_dir_all;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::process::ExitStatus;

use anyhow::Context;
use anyhow::Result as AnyResult;
use anyhow::bail;
use anyhow::ensure;
use ino_path::PathExt;
use ino_result::ResultExt;
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
    /// (unimplemented) Abort transcoding when first error occurred.
    // #[arg(long)]
    // abort_on_error: bool,

    /// (to write...)
    /// Defaults to PWD.
    #[arg(long, short = 'R')]
    root_dir: Option<PathBuf>,

    /// Skip putting original pictures into backup directory
    /// after transcoding.
    #[arg(long, short = 'N')]
    #[arg(default_value_t = false)]
    no_backup: bool,

    // Allow processing pictures marked as already transcoded
    // by ignoring the xattr check.
    // TODO: needed?
    // #[arg(long, short = 'i')]
    // #[arg(default_value_t = false)]
    // ignore_tag: bool,
    /// (unimplemented) Number of parallel transcoding to run.
    #[arg(long, short)]
    #[arg(default_value = "1")]
    jobs: NonZeroUsize,

    /// Display logs from transcoders.
    #[arg(long, short = 'L')]
    #[arg(default_value_t = false)]
    show_logs: bool,

    /// Manually choose pictures to transcode.
    #[arg(last = true)]
    selection: Option<Vec<PathBuf>>,
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
    pictures: Vec<PathBuf>,
}

impl App {
    fn run(self) -> AnyResult<()> {
        let Self {
            transcoder,
            root_dir,
            backup_dir,
            work_dir,
            no_backup,
            show_logs,
            pictures,
            ..
        } = self;

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

        todo!()
    }
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

        let pictures = if let Some(selection) = opts.selection {
            debug!("process manual selection");
            let mut accu = vec![];
            // TODO: reuse collect_pictures
            for s in selection {
                let path =
                    if s.is_absolute() { s } else { root_dir.join(s) };
                if path.is_dir_no_traverse()? {
                    // If the selection is a dir, collect pictures under it.
                    accu.append(&mut collect_pictures(
                        &path,
                        transcoder.input_format(),
                    ));
                } else if let Some(ext) = path.extension()
                    && let Some(ext) = ext.to_str()
                    && transcoder
                        .input_format()
                        .iter()
                        .any(|fmt| fmt.ext_matches(ext))
                {
                    // If it is just a supported picture,
                    // then pick it up as-is.
                    accu.push(path);
                } else {
                    debug!(
                        ?path,
                        "path is not valid or extension is not supported, ignored"
                    );
                }
            }
            accu
        } else {
            debug!("no selection provided, auto collect pictures");
            collect_pictures(&root_dir, transcoder.input_format())
        };

        ensure! { pictures.iter().all(|p| p.is_file()),
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

#[derive(Debug)]
pub struct Task {
    src: PathBuf,
    dst: PathBuf,
}

/// A transcoder with its various information.
trait Transcoder {
    /// A short and descriptive name for this transcoder.
    fn id(&self) -> &'static str;

    /// The picture formats that this transcoder accepts as input.
    fn input_format(&self) -> &'static [PictureFormat];

    /// The picture format that this transcoder outputs.
    fn output_format(&self) -> PictureFormat;

    /// Do the transcoding.
    // TODO: Get rid of ExitStatus
    #[allow(clippy::missing_errors_doc)]
    fn transcode(&self, task: Task) -> AnyResult<ExitStatus>;
}

/// Commonly encountered image formats.
#[derive(Debug)]
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
        .run()
        .context("Error happended when running app")?;

    Ok(())
}

fn main() {
    ino_tracing::init_tracing_subscriber();
    main_with_result().print_error_exit_process();

    //
    //     /*
    //      * Create archive_dir is needed
    //      */
    //
    //     if archive_after_encode {
    //         debug!( ?archive_dir );
    //
    //         // UNWRAP: when archive_after_encode is set archive_dir is also set
    //         #[ allow( clippy::unwrap_used ) ]
    //         let dir = archive_dir.clone().unwrap();
    //
    //         eprintln!(
    //             "Archive after encoding\
    //             \n\
    //             Create directory \"{}\"for archiving",
    //             dir.display()
    //         );
    //
    //         if !dir.try_exists()? {
    //             std::fs::create_dir_all( dir )?;
    //         }
    //     }
    //
    //     /*
    //      * Do collected tasks
    //      */
    //
    //     let total_tasks = files_to_encode.len();
    //
    //     for ( index, file ) in files_to_encode.iter().enumerate() {
    //         debug!( ?index, ?file );
    //
    //         let _span = debug_span!( "encoding_tasks", ?file ).entered();
    //
    //         let progress_percent = format!(
    //             "[{}/{total_tasks} {}]",
    //             index + 1,
    //             file.file_name()
    //                 .unwrap_or_default()
    //                 .to_string_lossy(),
    //         );
    //
    //         eprintln!(
    //             "{progress_percent} Encode in progress..."
    //         );
    //
    //         let encode_status = encoder.transcode( file )?;
    //
    //         if !encode_status.success() {
    //             eprintln!(
    //                 "{progress_percent} Failed to encode!"
    //             );
    //             std::process::exit( 1 )
    //         }
    //
    //         if archive_after_encode {
    //             eprintln!( "{progress_percent} Archive original file");
    //             let basename = file.file_name()
    //                 .expect( "It doesn't have a basename, how come?!" );
    //             // TODO: this is code smell, do something later
    //             #[ allow( clippy::unwrap_used ) ]
    //             let target = archive_dir.clone().unwrap().join( basename );
    //             std::fs::rename( file, target )?;
    //         }
    //     }
    //
    // }
}
