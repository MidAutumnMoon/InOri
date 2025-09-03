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
    #[arg(long, short = 'r')]
    root_dir: Option<PathBuf>,

    /// Skip putting original pictures into backup directory
    /// after transcoding.
    #[arg(long, short = 'N')]
    #[arg(default_value_t = false)]
    no_backup: bool,

    /// Allow processing pictures marked as already transcoded
    /// by ignoring the xattr check.
    // TODO: needed?
    // #[arg(long, short = 'i')]
    // #[arg(default_value_t = false)]
    // ignore_tag: bool,

    /// (unimplemented) Number of parallel transcoding to run.
    #[arg(long, short)]
    #[arg(default_value = "1")]
    jobs: NonZeroUsize,

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
    pictures: Vec<PathBuf>,
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
            for s in selection {
                let path =
                    if s.is_absolute() { s } else { root_dir.join(s) };
                if path.is_dir_no_traverse()? {
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

    if let CliOpts::Complete { shell } = &opts {
        use clap::CommandFactory;
        use clap_complete::generate;
        let mut cmd = CliOpts::command();
        generate(*shell, &mut cmd, "avxl", &mut std::io::stdout());
        return Ok(());
    }

    let app = App::try_from(opts)?;

    dbg!(app.pictures);

    Ok(())
}

fn main() {
    ino_tracing::init_tracing_subscriber();
    main_with_result().print_error_exit_process();

    // let dir_and_files = if dir_and_files.is_empty() {
    //     debug!( "CLI provided input is empty, use PWD" );
    //     vec![ std::env::current_dir()? ]
    // } else {
    //     dir_and_files
    // };
    //
    // let dir_and_files: Vec<DirOrFiles> = {
    //     let mut dirs: Vec<PathBuf> = vec![];
    //     let mut files: Vec<PathBuf> = vec![];
    //
    //     for it in dir_and_files {
    //         if it.is_dir() {
    //             let Some( basename ) = it.file_name() else { continue; };
    //             // skip the dir created by ourselves.
    //             if basename == ARCHIVE_DIR_NAME {
    //                 eprintln!(
    //                     "Skipping dir \"{}\" because it's named {ARCHIVE_DIR_NAME} \
    //                     which is used for storing original files after encoding.\
    //                     \n\
    //                     This should be a mistake, otherwise rename the directory \
    //                     to another name.",
    //                     it.display()
    //                 );
    //                 continue;
    //             }
    //             dirs.push( it );
    //         } else if it.is_file() {
    //             files.push( it );
    //         } else {
    //             eprintln!(
    //                 "\"{}\" is not a file nor dir, which is not supported.",
    //                 it.display()
    //             );
    //             std::process::exit( 1 )
    //         }
    //     }
    //
    //     Vec::with_capacity( dirs.len() + 1 )
    //         .tap_mut( |s| {
    //             let mut dirs = dirs.into_iter()
    //                 .map( DirOrFiles::Dir )
    //                 .collect();
    //             s.append( &mut dirs );
    //         } )
    //         .tap_mut( |s| {
    //             s.push( DirOrFiles::Files( files ) );
    //         } )
    // };
    //
    // debug!( ?dir_and_files );

    /*
     * Tasks and encoding
     */

    // let _span_of_daf =
    //     debug_span!( "encode_dir_and_files" ).entered();
    //
    // for dir_or_files in dir_and_files {
    //
    //     debug!( ?dir_or_files );
    //
    //     let archive_after_encode: bool;
    //     let archive_dir: Option<PathBuf>;
    //
    //     let files_to_encode: Vec<PathBuf>;
    //
    //     /*
    //      * Unwrap dir_and_files to construct tasks
    //      */
    //
    //     match dir_or_files {
    //         // If it is a dir, enable archive_after_encode
    //         // and collect files inside it
    //         DirOrFiles::Dir( dir ) => {
    //             eprintln!(
    //                 "Checking directory {}", dir.display()
    //             );
    //             archive_after_encode = true;
    //             archive_dir = Some( dir.join( ARCHIVE_DIR_NAME ) );
    //             files_to_encode = tool::filter_by_supported_exts(
    //                 &encoder, tool::find_files( &dir )?
    //             );
    //
    //         },
    //         // If it is file otherwise, the files are already the tasks.
    //         DirOrFiles::Files( files ) => {
    //             let files = tool::filter_by_supported_exts( &encoder, files );
    //             // ...so that app won't print "Checking 0 files"
    //             if files.is_empty() { continue }
    //             eprintln!(
    //                 "Chekcing {} files", files.len()
    //             );
    //             archive_after_encode = false;
    //             archive_dir = None;
    //             files_to_encode = files;
    //         }
    //     }
    //
    //     debug!( ?files_to_encode, ?archive_after_encode, ?archive_dir );
    //
    //     if files_to_encode.is_empty() {
    //         eprintln!( "No file need to be encoded" );
    //         continue;
    //     }
    //
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
