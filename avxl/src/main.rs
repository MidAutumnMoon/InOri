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
        #[ arg( long, short, action, default_value_t=true ) ]
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
            (
                &avif::Avif {
                    no_cq, yuv444,
                    cq_level: cq_level.unwrap_or_default()
                },
                input.dir_and_files
            )
        },
        CliOpts::Jxl { input } => {
            debug!( "JXL mode" );
            ( &jxl::Jxl, input.dir_and_files )
        },
        _ => unimplemented!(),
    };


    /*
     * Sanitize input
     */

    let dir_and_files: Vec<DirOrFiles> = {
        let them = if dir_and_files.is_empty() {
            debug!( "CLI provided input is empty, use PWD" );
            vec![ std::env::current_dir()? ]
        } else {
            dir_and_files
        };

        let mut collected: Vec<DirOrFiles> = vec![];
        let mut files: Vec<PathBuf> = vec![];

        for it in them {
            if it.is_dir() {
                collected.push( DirOrFiles::Dir( it ) )
            } else if it.is_file() {
                files.push( it )
            } else {
                colour::e_red_ln!(
                    "\"{}\" is not a file nor dir, which is not supported.",
                    it.display()
                );
                std::process::exit( 1 )
            }
        }

        collected.push( DirOrFiles::Files( files ) );
        collected
    };

    debug!( ?dir_and_files );


    /*
     * Tasks and encoding
     */

    let span_of_daf = debug_span!( "process_daf" ).entered();

    for daf in dir_and_files {

        debug!( ?daf );

        let archive_after_encode: bool;
        let archive_dir: Option<PathBuf>;

        let tasks: Vec<PathBuf>;


        /*
         * Unwrap dir_and_files to construct tasks
         */

        match daf {
            // If it is a dir, enable archive_after_encode
            // and collect files inside it
            DirOrFiles::Dir( dir ) => {
                colour::e_yellow_ln_bold!(
                    "\nWorking on directory {}", dir.display()
                );
                archive_after_encode = true;
                archive_dir = Some( dir.join( ARCHIVE_DIR_NAME ) );
                tasks = tool::filter_by_supported_exts(
                    encoder, tool::find_files( &dir )?
                );

            },
            // If it is file otherwise, the files are
            // already the tasks.
            DirOrFiles::Files( files ) => {
                let files = tool::filter_by_supported_exts( encoder, files );
                colour::e_yellow_ln_bold!(
                    "\nWorking on {} files", files.len()
                );
                archive_after_encode = false;
                archive_dir = None;
                tasks = files;
            }
        }

        debug!( ?tasks, ?archive_after_encode, ?archive_dir );

        if tasks.is_empty() {
            colour::e_blue_ln!( "Empty, no task" );
            continue;
        }


        /*
         * Create archive_dir is needed
         */

        if archive_after_encode {
            debug!( ?archive_dir, "try create archive_dir" );

            // UNWRAP: when archive_after_encode is set archive_dir is also set
            let dir = archive_dir.clone().unwrap();
            if !dir.try_exists()? { std::fs::create_dir_all( dir )?; }
        }


        /*
         * Do collected tasks
         */

        let span_of_task =
            debug_span!( "encoding_tasks" ).entered();

        let total_tasks = tasks.len();

        for ( index, task ) in tasks.iter().enumerate() {
            debug!( ?index, ?task );

            let progress_msg = format!(
                "[{}/{total_tasks} {:?}]",
                index + 1,
                task.file_name().unwrap_or_default(),
            );

            colour::e_blue_ln!( "{progress_msg} Encode in progress..." );

            let encode_status = encoder.perform_encode( task )?;

            if !encode_status.success() {
                colour::e_red_ln!( "{progress_msg} Failed to encode!" );
                std::process::exit( 1 )
            }

            if archive_after_encode {
                colour::e_blue_ln!( "{progress_msg} Archive original file" );
                let basename = task.file_name()
                    .expect( "It doesn't have a basename, how come?!" );
                let target = archive_dir.clone().unwrap().join( basename );
                std::fs::rename( task, target )?;
            }
        }

        drop( span_of_task );

    }

    drop( span_of_daf );

    Ok(())

}
