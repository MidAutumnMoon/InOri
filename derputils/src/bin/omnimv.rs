//! omnimv
//!
//! It orginates from a simple shell script
//! for fetching files from the download directory
//! to CWD, so "omnifetch" may be a more
//! proper name but... whatever.

use std::path::PathBuf;

use tracing::{
    debug,
    debug_span,
};

use anyhow::ensure;

use itertools::Itertools;


/// Move files from other places to current
/// working directory.
#[derive( Debug, argh::FromArgs )]
struct CmdOpts {
    /// directory to search from.
    /// can be specified multiple times.
    #[argh( option, long = "dir", short = 'd' )]
    searchdirs: Vec<PathBuf>,

    /// list filenames in configured directories,
    /// useful for doing shell completion.
    /// Note: results are deduplicated
    #[argh( switch, short = 'l' )]
    listing: bool,

    /// names of files to be moved,
    /// use "--" to escape special filenames.
    /// Note: in case of files presented
    /// in multiple search directories,
    /// only the first one will be moved.
    #[argh( positional, arg_name = "filenames" )] // arg_name undocumented
    files_to_move: Vec<PathBuf>,
}

impl CmdOpts {

    #[tracing::instrument]
    fn new() -> anyhow::Result<Self> {
        let opts = argh::from_env::<Self>();

        debug!( ?opts, "Parsed cmdopts" );

        ensure! { ! opts.searchdirs.is_empty(),
            "At least one --dir must be specified.\
            \n\n\
            Run omnimv --help for more information."
        }

        // dedup searchdirs
        let searchdirs = opts.searchdirs.clone()
            .into_iter()
            .unique()
            .collect_vec();

        debug!( ?searchdirs, "Deduped search dirs" );

        Ok( Self { searchdirs, ..opts } )
    }

}


fn main() -> anyhow::Result<()> {

    //
    // Enable tracing
    //

    ino_tracing::init_tracing_subscriber();


    //
    // Get cmd options
    //

    let opts = CmdOpts::new()?;

    let CmdOpts {
        files_to_move,
        searchdirs,
        ..
    } = &opts;


    //
    // Listing mode
    //

    if opts.listing {
        let _span = debug_span!( "listing" ).entered();

        debug!( "Listing files" );

        // "70" is an arbitrary value based on
        // the current number of files in my download
        // folder :P, and also it's about the average amount.
        let mut collected =
            Vec::<String>::with_capacity( 70 );

        for srchdir in searchdirs {
            let _span = debug_span!( "inside", ?srchdir ).entered();

            debug!( "Try read_dir" );

            for entry in srchdir.read_dir()? {
                let _span = debug_span!( "on_entry", ?entry ).entered();

                let entry = entry?;
                let ftype = &entry.file_type()?;

                // Not considering for symlinks,
                // they're not common in my workflow.
                // May be tweakable in the future.
                if ! ftype.is_file() {
                    debug!( "Not file, skip" );
                    continue
                }

                let fname = entry.file_name()
                    .to_string_lossy()
                    .into_owned();
                debug!( ?fname, "Found file" );
                collected.push( fname );
            }
        }

        let output = collected.into_iter()
            .unique()
            .join( "\n" );

        println!( "{output}" );

        return Ok(())
    }


    //
    // Actually moving files
    //

    let _span = debug_span!( "moving" ).entered();


    debug!( ?files_to_move, "Files to move" );

    debug!( "Try to get CWD" );

    let cwd = std::env::current_dir()?;

    debug!( ?cwd );


    if files_to_move.is_empty() {
        debug!( "No files to move" )
    }

    for filename in files_to_move {
        let _span = debug_span!( "for_name", ?filename ).entered();


        let mv_dest = cwd.join( filename );

        debug!( "Ensure no collinsion" );

        ensure! { ! mv_dest.try_exists()?,
            "\"{}\" already exists under CWD",
            filename.display(),
        }


        let mut collected = Vec::<PathBuf>::new();

        for srchdir in searchdirs {
            let _span = debug_span!( "search_in", ?srchdir ).entered();

            let path = srchdir.join( filename );

            debug!( ?path, "Try path" );

            if path.try_exists()? {
                debug!( ?path, "Found" );
                collected.push( path )
            } else {
                debug!( "Not exists" )
            }
        }

        debug!( ?collected, "Collected source files" );


        ensure! { ! collected.is_empty(),
            "\"{}\" not found in specified directories.",
            filename.display()
        }

        if collected.len() > 1 {
            debug!( "Found duplicated files, take first" )
        }

        // Safety: bails when empty, so "collected"
        // guaranteed to hold at least one item
        let mv_orig = &collected.first().unwrap();


        let _span_from_to = debug_span!( "from_to", ?mv_orig, ?mv_dest )
            .entered();

        debug!( "Ready to move" );

        println!( "Move {} -> CWD ({})",
            &mv_orig.display(),
            &cwd.display()
        );

        // coreutils is more well equiped
        // to deal with all sorts of edge cases.
        //
        // Namely:
        //
        // 1) rename*() doesn't work across mountpoints
        //
        // 2) Rust's fs::copy will try to
        // preserve metadata with no way to control
        // which will fail on some NFS or WSL's
        // drvfs on which linux permission is not
        // available.
        //
        // 3) Some deadly wild animals sneaking behind
        // the read() and write() syscalls.

        use std::process::Command;

        debug!( "Calling mv to move files" );

        let output = Command::new( "mv" )
            .args([ "--verbose", "--no-clobber" ])
            .arg( mv_orig )
            .arg( mv_dest )
            .output()?;

        debug!( ?output, "mv output" );

        ensure! { output.status.success(),
            "Move failed.\n\n{}",
            String::from_utf8_lossy( &output.stderr ).trim()
        }
    }


    Ok(())

}
