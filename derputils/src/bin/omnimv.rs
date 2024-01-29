//! omnimv
//!
//! It orginates from a simple shell script
//! for fetching files from the download directory
//! to CWD, so "omnifetch" may be a more
//! proper name but... whatever.
//!

use std::path::PathBuf;

use tracing::{
    debug,
    debug_span,
};

use anyhow::bail;

use itertools::Itertools;


//
// CmdOpts struct
//

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

    /// enable debug logging
    #[argh( switch )]
    debug: bool,

    /// names of files to be moved,
    /// use "--" to escape special filenames.
    /// Note: in case of files presented
    /// in multiple search directories,
    /// only the first one will be moved.
    #[argh( positional )]
    filenames: Vec<PathBuf>,
}

impl CmdOpts {

    fn new()
        -> anyhow::Result<Self>
    {
        let opts = argh::from_env::<Self>();

        if opts.searchdirs.is_empty() {
            bail!(
                "At least one --dir must be specified.\
                \n\n\
                Run omnimv --help for more information."
            )
        }

        // dedup searchdirs
        let searchdirs = opts.searchdirs.clone()
            .into_iter()
            .unique()
            .collect_vec();

        Ok( Self {
            searchdirs,
            ..opts
        } )
    }

}


fn main() -> anyhow::Result<()> {

    //
    // Get cmd options
    //

    let opts = CmdOpts::new()?;

    let CmdOpts {
        filenames,
        searchdirs,
        ..
    } = &opts;


    //
    // Enable tracing
    //

    {
        use tracing::Level;
        use tracing_subscriber::fmt::fmt;

        let level = match opts.debug {
            true => Level::TRACE,
            false => Level::WARN,
        };

        fmt()
            .with_writer( std::io::stderr )
            .with_ansi( true )
            .with_max_level( level )
            .init();
    }

    debug!( ?opts, "Commandline options" );

    debug!( ?searchdirs, "Deduped" );


    //
    // Listing mode
    //

    if opts.listing {

        let _span = debug_span!( "listing" ).entered();

        debug!( "Listing files" );

        // "70" is an arbitrary value based on
        // the current count of files in my download
        // folder :P, but crucially it's also about
        // the average based on my experience.
        let mut collected =
            Vec::<String>::with_capacity( 70 );

        for dir in searchdirs {
            let _span =
                debug_span!( "dir", path = dir.to_str() )
                .entered();

            debug!( ?dir, "Read directory" );

            for entry in dir.read_dir()? {
                let _span = debug_span!( "", ?entry )
                    .entered();

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

    let _span = debug_span!( "moving" )
        .entered();

    debug!( ?filenames, "Files to move" );

    debug!( "Try to get CWD" );

    let cwd = std::env::current_dir()?;

    debug!( ?cwd );


    if filenames.is_empty() {
        debug!( "No files to move" )
    }


    for fname in filenames {

        let _span = debug_span!( "for_file", ?fname )
            .entered();


        let destination = cwd.join( fname );

        debug!( "Check for existence in CWD" );

        if destination.try_exists()? {
            bail!( format!(
                "\"{}\" already exists in CWD \"{}\"",
                fname.display(),
                cwd.display(),
            ) )
        }


        let mut collected = Vec::<PathBuf>::new();

        for dir in searchdirs {
            let _span = debug_span!( "search_in", ?dir )
                .entered();

            let fullpath = dir.join( fname );

            debug!( ?fullpath );

            if fullpath.try_exists()? {
                collected.push( fullpath )
            } else {
                debug!( "Not exists" )
            }
        }

        debug!( ?collected, "Check founds" );

        if collected.is_empty() {
            bail!( format!(
                "\"{}\" not found in specified directories.",
                fname.display()
            ) )
        }

        if collected.len() > 1 {
            debug!(
                "Found multiple same-named files, take first"
            )
        }

        // Safety: bails when empty, so "collected"
        // guaranteed to hold at least one item
        let origin = &collected.first().unwrap();

        debug!( ?origin, ?destination, "Ready to move" );

        println!(
            "Move {} -> CWD ({})",
            &origin.display(),
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
        // the read() and write() syscall.

        use std::process::Command;

        debug!( "Launch mv" );

        let output = Command::new( "mv" )
            .args([ "--verbose", "--no-clobber" ])
            .arg( origin )
            .arg( destination )
            .output()?;

        debug!( ?output );

        if ! &output.status.success() {
            bail!( format!(
                "Move failed.\n\n{}",
                String::from_utf8_lossy( &output.stderr ).trim()
            ) )
        }

    }


    Ok(())

}
