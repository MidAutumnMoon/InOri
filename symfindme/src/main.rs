use tracing::debug;
use tracing::trace;

use std::{
    collections::HashSet,
    path::PathBuf,
    path::Path,
};

use anyhow::Context;

mod tool;

/// wow
#[ derive( clap::Parser ) ]
#[ derive( Debug ) ]
struct Application {
    /// The name of p
    program: String,

    #[ arg( long, short, default_value_t=32 ) ]
    max_symlink_follows: u64,
}

impl Application {

    #[ tracing::instrument ]
    fn run( &self ) -> anyhow::Result<()> {
        trace!( "Start application" );

        let findings = tool::lookup_executable_in_path( &self.program );

        let walker_start = findings.first()
            .ok_or_else( ||
                anyhow::anyhow!( "Executable \"{}\" not found", self.program )
            )?
        ;

        debug!( ?walker_start );

        let walker = SymlinkWalker::new( walker_start, self.max_symlink_follows );

        for path in walker {
            let path = path
                // TODO: better error message
                .context( "Can't walk path" )?;
            println!( "{}", path.display() );
        }

        Ok(())
    }

}

fn main() {
    ino_tracing::init_tracing_subscriber();

    trace!( "Parse command line options" );

    let _ = <Application as clap::Parser>::parse()
        .run()
        .inspect_err( |err| {
            eprintln!( "{err:?}" );
            std::process::exit( 1 )
        } )
    ;
}


#[ derive( Debug ) ]
struct SymlinkWalker {
    current: Option<PathBuf>,
    visited_paths: HashSet<PathBuf>,
    max_symlink_follows: u64,
    symlink_followed: u64,
}

impl SymlinkWalker {
    #[ tracing::instrument ]
    fn new( start: &Path, max_symlink_follows: u64, ) -> Self {
        trace!( "Create new symlink walker" );
        Self {
            current: Some( start.to_owned() ),
            visited_paths: Default::default(),
            max_symlink_follows,
            symlink_followed: 0,
        }
    }
}

impl std::iter::Iterator for SymlinkWalker {
    type Item = anyhow::Result<PathBuf>;

    #[ tracing::instrument ]
    fn next( &mut self ) -> Option< Self::Item > {
        let current = self.current.take()?;

        debug!( ?current );

        // NOTE: early return
        if self.visited_paths.contains( &current ) {
            debug!( "Already visited this path" );
            // TODO: better error message
            let err = anyhow::anyhow!( "Symlink loop!" );
            return Some( Err( err ) )
        }

        if self.symlink_followed + 1 > self.max_symlink_follows {
            // TODO: better error message
            let err = anyhow::anyhow!( "Max symlink follows reached" );
            return Some( Err(err) )
        } else {
            self.symlink_followed += 1;
        }

        trace!( "Read metadata" );
        let metadata = current.symlink_metadata()
            // TODO: better error message
            .context( "Failed to read metadata" )
            .ok()?
        ;

        if metadata.is_symlink() {
            debug!( "Found new symlink" );
            trace!( "Read symlink target" );
            let link_target = current.read_link()
                // TODO: better error message
                .context( "Failed to read_link" )
                .ok()?
            ;
            self.current = Some( link_target );
        } else {
            trace!( "Not a symlink, the end of symlink chain reached" );
        }

        self.visited_paths.insert( current.clone() );
        return Some( Ok( current ) )
    }
}

