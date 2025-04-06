use std::path::{
    PathBuf,
    Path
};

use std::process::ExitStatus;

use tracing::{
    debug,
    debug_span,
};

mod avif;
mod jxl;
mod tool;
use tap::Tap;

/// Name of the directory for storing original pictures.
pub const ARCHIVE_DIR_NAME: &str = "original";


#[ derive( clap::Args, Debug ) ]
struct CliInput {
    dir_and_files: Vec<PathBuf>
}


/// A CLI tool for converting pictures to AVIF or JXL.
#[ derive( clap::Parser, Debug ) ]
enum CliOpts {
    /// Encode to both AVIF and JXL for the user to compare the outputs.
    /// *(unimplemented)*
    Compare {
        #[ command( flatten ) ]
        input: CliInput
    },

    /// Encode to **lossy** AVIF using `avifenc`.
    Avif {
        /// Disable constant quality mode.
        #[ arg( long, short, action, default_value_t=false ) ]
        no_cq: bool,

        /// Encode using Yuv444 instead of Yuv420.
        #[ arg( long, short, action, default_value_t=false ) ]
        yuv444: bool,

        /// Custom CQ level value.
        #[ arg( long, short ) ]
        cq_level: Option<u8>,

        #[ command( flatten ) ]
        input: CliInput,
    },

    /// Encode to **lossless** JXL using `cjxl`.
    Jxl {
        #[ command( flatten ) ]
        input: CliInput
    },
}


#[ derive( Debug ) ]
pub enum DirOrFiles {
    Dir( PathBuf ),
    Files( Vec<PathBuf> ),
}


trait Encoder {
    /// Files of such extensions the encoder supported to use as input.
    fn is_ext_supported( &self, input_ext: &str ) -> bool;

    /// Run the encoder on `picture`.
    fn perform_encode( &self, input: &Path )
        -> anyhow::Result< ExitStatus >;
}


fn main() -> anyhow::Result<()> {

    /*
     * Setup tracing
     */

    ino_tracing::init_tracing_subscriber();


    /*
     * Parse CLI options
     */

    debug!( "parse cliopts" );

    let cliopts = < CliOpts as clap::Parser >::parse();

    debug!( ?cliopts );


    /*
     * Get the encoder
     */

    let ( encoder, dir_and_files ): ( &dyn Encoder, _ ) = match cliopts {
        CliOpts::Avif { no_cq, yuv444, cq_level, input } => {
            debug!( "AVIF mode" );
            let default = &avif::Avif::default();
            (
                &avif::Avif {
                    no_cq, yuv444,
                    cq_level: cq_level.unwrap_or( default.cq_level )
                },
                input.dir_and_files
            )
        },
        CliOpts::Jxl { input } => {
            debug!( "JXL mode" );
            ( &jxl::Jxl, input.dir_and_files )
        },
        CliOpts::Compare { .. } => unimplemented!()
    };


    /*
     * Sanitize input
     */

    let dir_and_files = if dir_and_files.is_empty() {
        debug!( "CLI provided input is empty, use PWD" );
        vec![ std::env::current_dir()? ]
    } else {
        dir_and_files
    };

    let dir_and_files: Vec<DirOrFiles> = {
        let mut dirs: Vec<PathBuf> = vec![];
        let mut files: Vec<PathBuf> = vec![];

        for it in dir_and_files {
            if it.is_dir() {
                let Some( basename ) = it.file_name() else { continue; };
                // skip the dir created by ourselves.
                if basename == ARCHIVE_DIR_NAME {
                    eprintln!(
                        "Skipping dir \"{}\" because it's named {ARCHIVE_DIR_NAME} \
                        which is used for storing original files after encoding.\
                        \n\
                        This should be a mistake, otherwise rename the directory \
                        to another name.",
                        it.display()
                    );
                    continue;
                }
                dirs.push( it );
            } else if it.is_file() {
                files.push( it );
            } else {
                eprintln!(
                    "\"{}\" is not a file nor dir, which is not supported.",
                    it.display()
                );
                std::process::exit( 1 )
            }
        }

        Vec::with_capacity( dirs.len() + 1 )
            .tap_mut( |s| {
                let mut dirs = dirs.into_iter()
                    .map( DirOrFiles::Dir )
                    .collect();
                s.append( &mut dirs );
            } )
            .tap_mut( |s| {
                s.push( DirOrFiles::Files( files ) );
            } )
    };

    debug!( ?dir_and_files );


    /*
     * Tasks and encoding
     */

    let _span_of_daf =
        debug_span!( "encode_dir_and_files" ).entered();

    for dir_or_files in dir_and_files {

        debug!( ?dir_or_files );

        let archive_after_encode: bool;
        let archive_dir: Option<PathBuf>;

        let files_to_encode: Vec<PathBuf>;


        /*
         * Unwrap dir_and_files to construct tasks
         */

        match dir_or_files {
            // If it is a dir, enable archive_after_encode
            // and collect files inside it
            DirOrFiles::Dir( dir ) => {
                eprintln!(
                    "Checking directory {}", dir.display()
                );
                archive_after_encode = true;
                archive_dir = Some( dir.join( ARCHIVE_DIR_NAME ) );
                files_to_encode = tool::filter_by_supported_exts(
                    encoder, tool::find_files( &dir )?
                );

            },
            // If it is file otherwise, the files are already the tasks.
            DirOrFiles::Files( files ) => {
                let files = tool::filter_by_supported_exts( encoder, files );
                // ...so that app won't print "Checking 0 files"
                if files.is_empty() { continue }
                eprintln!(
                    "Chekcing {} files", files.len()
                );
                archive_after_encode = false;
                archive_dir = None;
                files_to_encode = files;
            }
        }

        debug!( ?files_to_encode, ?archive_after_encode, ?archive_dir );

        if files_to_encode.is_empty() {
            eprintln!( "No file need to be encoded" );
            continue;
        }


        /*
         * Create archive_dir is needed
         */

        if archive_after_encode {
            debug!( ?archive_dir );

            // UNWRAP: when archive_after_encode is set archive_dir is also set
            #[ allow( clippy::unwrap_used ) ]
            let dir = archive_dir.clone().unwrap();

            eprintln!(
                "Archive after encoding\
                \n\
                Create directory \"{}\"for archiving",
                dir.display()
            );

            if !dir.try_exists()? {
                std::fs::create_dir_all( dir )?;
            }
        }


        /*
         * Do collected tasks
         */


        let total_tasks = files_to_encode.len();

        for ( index, file ) in files_to_encode.iter().enumerate() {
            debug!( ?index, ?file );

            let _span = debug_span!( "encoding_tasks", ?file ).entered();

            let progress_percent = format!(
                "[{}/{total_tasks} {}]",
                index + 1,
                file.file_name()
                    .unwrap_or_default()
                    .to_string_lossy(),
            );

            eprintln!(
                "{progress_percent} Encode in progress..."
            );

            let encode_status = encoder.perform_encode( file )?;

            if !encode_status.success() {
                eprintln!(
                    "{progress_percent} Failed to encode!"
                );
                std::process::exit( 1 )
            }

            if archive_after_encode {
                eprintln!( "{progress_percent} Archive original file");
                let basename = file.file_name()
                    .expect( "It doesn't have a basename, how come?!" );
                // TODO: this is code smell, do something later
                #[ allow( clippy::unwrap_used ) ]
                let target = archive_dir.clone().unwrap().join( basename );
                std::fs::rename( file, target )?;
            }
        }

    }

    Ok(())

}
