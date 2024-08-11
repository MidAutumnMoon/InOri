//! omnimv
//!
//! It orginates from a simple shell script
//! for fetching files from the download directory
//! to CWD, so "omnifetch" may be a more
//! proper name but... whatever.

use std::path::{
    Path,
    PathBuf,
};

use tracing::{
    debug,
    debug_span,
};

use anyhow::{
    ensure,
    bail,
};

use itertools::Itertools;


/// Arbitrary value based on the current amount
/// of files in my download folder :P
const AVERAGE_AMOUNT: usize = 70;


/// Move files from other places to current
/// working directory.
#[ derive( Debug, clap::Parser ) ]
struct CmdOpts {
    /// directory to search from.
    /// can be specified multiple times.
    #[ arg( long = "dir", short = 'd' ) ]
    searchdirs: Vec<PathBuf>,

    /// list filenames in configured directories,
    /// useful for doing shell completion.
    #[ arg( long, short = 'l' ) ]
    listing: bool,

    /// names of files to be moved,
    /// use "--" to escape special filenames.
    ///
    /// Note: in case of files presented
    /// in multiple search directories,
    /// only the first one will be moved.
    #[ arg( id = "filenames" ) ]
    // arg_name undocumented
    feedle_names: Vec<String>,
}

impl CmdOpts {
    #[ tracing::instrument ]
    fn new() -> anyhow::Result<Self> {
        let opts = <Self as clap::Parser>::parse();

        debug!( ?opts, "cmdopts" );

        ensure! { ! opts.searchdirs.is_empty(),
            "At least one --dir must be specified.\
            \n\n\
            Run with --help for more information."
        }

        ensure! { opts.listing || ! opts.feedle_names.is_empty(),
            "No files to be moved.\
            \n\n\
            Run with --help for more information."
        }

        let searchdirs = opts.searchdirs.clone()
            .into_iter()
            .unique()
            .collect_vec();

        debug!( ?searchdirs, "deduped search dirs" );

        Ok( Self { searchdirs, ..opts } )
    }
}

/// Feedle: file needle
#[derive( Debug )]
struct Feedle {
    name: String,
    origin: PathBuf,
}

impl Feedle {
    /// Helper function for collecting Feedles from `dir`.
    #[tracing::instrument]
    fn from_dir( dir: &Path )
        -> anyhow::Result<Vec<Self>>
    {
        debug!( "looking for files" );

        let mut collected =
            Vec::with_capacity( AVERAGE_AMOUNT );

        for entry in dir.read_dir()? {
            let _s = debug_span!( "maybe_entry", ?entry ).entered();
            let entry = entry?;

            // ignore symlinks for now
            if entry.file_type()?.is_file() {
                debug!( "found file" );
                let name = entry.file_name()
                    .to_string_lossy()
                    .into_owned();
                let origin = entry.path();
                collected.push( Self { name, origin } )
            } else {
                debug!( "not file, skip" );
                continue;
            }
        }
        Ok( collected )
    }

    #[tracing::instrument]
    fn move_to( &self, dest: &Path ) -> anyhow::Result<()> {
        use std::process::Command;
        debug!( "move file" );

        println!( "Move \"{}\"", self.origin.display() );

        let result = Command::new( "mv" )
            .args([ "--verbose", "--no-clobber" ])
            .arg( self.origin.as_path() )
            .arg( dest )
            .output()?;

        debug!( ?result, "command result" );

        ensure! { result.status.success(),
            "Move failed\n\nStderr: {}",
            String::from_utf8_lossy( &result.stderr ).trim()
        }
        Ok(())
    }
}


#[derive( Debug )]
struct Haystack {
    inner: Vec<Feedle>,
}

impl Haystack {
    #[tracing::instrument]
    fn new() -> Self {
        Self { inner: Vec::with_capacity( AVERAGE_AMOUNT * 2 ) }
    }

    #[tracing::instrument]
    fn append( &mut self, other: &mut Vec<Feedle> ) {
        self.inner.append( other )
    }

    #[tracing::instrument]
    fn feedle_names( &self ) -> Vec<&str> {
        self.inner.iter()
            .map( |e| e.name.as_str() )
            .collect()
    }

    #[tracing::instrument]
    fn find( &self, name: &str ) -> Vec<&Feedle> {
        self.inner.iter()
            .filter( |e| e.name == name )
            .collect_vec()
    }
}


fn main() -> anyhow::Result<()> {

    // Enable tracing

    ino_tracing::init_tracing_subscriber();


    // Get cmd options

    let cmd_opts = CmdOpts::new()?;

    let CmdOpts {
        feedle_names,
        searchdirs,
        ..
    } = &cmd_opts;


    // Collect feedles

    let haystack: Haystack = {
        let _s = debug_span!( "haystack" ).entered();

        debug!( "collect feedles to make haystack" );

        let mut haystack = Haystack::new();

        for schdir in searchdirs {
            let _s = debug_span!( "inside", ?schdir ).entered();
            haystack.append(
                &mut Feedle::from_dir( schdir )?
            );
        }

        haystack
    };

    debug!( ?haystack, "final haystack" );


    // Listing

    if cmd_opts.listing {
        let _s = debug_span!( "listing" ).entered();
        debug!( "listing mode" );

        println! { "{}",
            haystack.feedle_names().join( "\n" )
        };

        return Ok(())
    }


    // Moving files

    let _s = debug_span!( "moving" ).entered();

    debug!( ?feedle_names );

    let current_dir = std::env::current_dir()?;

    debug!( ?current_dir );


    for name in feedle_names {
        let _s = debug_span!( "feedle_name", ?name ).entered();

        let found = match haystack.find( name )[..] {
            [] => {
                debug!( "not found" );
                bail!( "File \"{name}\" not found in searchdirs" );
            },
            [ feedle ] => {
                debug!( "found one" );
                feedle
            },
            ref all @ [ .. ] => {
                debug!( "find multiple, take first" );
                all.first().unwrap()
            },
        };

        debug!( ?found );

        let feedle_dest = current_dir.join( name );

        debug!( ?feedle_dest, "prepare to move" );

        debug!( "check for collinsion" );

        ensure! { ! feedle_dest.try_exists()?,
            "{name} already exists under current directory",
        };

        found.move_to( &feedle_dest )?
    }

    Ok(())
}
