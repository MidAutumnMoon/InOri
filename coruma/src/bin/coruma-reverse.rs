use tracing::debug;
use tracing::trace;

use std::{
    collections::HashSet,
    path::PathBuf,
    path::Path,
};

use std::iter::Iterator;
use std::fmt::Display;
use std::fmt::Debug;

use anyhow::Context;


fn main() {
    use ino_result::ResultExt;
    ino_tracing::init_tracing_subscriber();
    <Application as clap::Parser>::parse()
        .run()
        .unwrap_print_error()
    ;
}

///  Find executable in $PATH, and print each ancestor in its symlink chain.
#[ derive( clap::Parser ) ]
#[ derive( Debug ) ]
struct Application {
    /// The name of executable to find in $PATH.
    program: String,

    /// Maximum symlink follows allowed, exceeding this value
    /// will terminate the application.
    #[ arg( long, short, default_value_t=32 ) ]
    max_symlink_follows: u64,
}

impl Application {
    #[ tracing::instrument ]
    fn run( &self ) -> anyhow::Result<()> {
        trace!( "Start application" );

        let starter = coruma::lookup_executable_in_path( &self.program )
            .first()
            .ok_or_else( ||
                anyhow::anyhow!( r#"Program "{}" not found"#, self.program )
            )?
            .to_owned() ;

        debug!( ?starter );

        SymlinkAncestor::new( &starter, self.max_symlink_follows )
            .collect::< Result< Vec<_>, _ > >()
                .context( "Unable to continue digging symlink ancestors" )?
            .into_iter()
            .for_each( |path| {
                Explainer::explain_path( &path.display() );
            } ) ;

        Ok(())
    }
}


#[ derive( Debug ) ]
struct SymlinkAncestor {
    current: Option<PathBuf>,
    visited_paths: HashSet<PathBuf>,
    max_symlink_follows: u64,
    symlink_followed: u64,
}

impl SymlinkAncestor {
    #[ tracing::instrument ]
    fn new( start: &Path, max_symlink_follows: u64, ) -> Self {
        Self {
            current: Some( start.to_owned() ),
            visited_paths: Default::default(),
            max_symlink_follows,
            symlink_followed: 0,
        }
    }
}

impl Iterator for SymlinkAncestor {
    type Item = anyhow::Result<PathBuf>;

    fn next( &mut self ) -> Option< Self::Item > {
        let _s = tracing::debug_span!( "next" ).entered();

        let current = self.current.take()?;
        debug!( ?current );

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

        trace!( "Read symlink metadata" );

        let metadata = match current
            .symlink_metadata()
            .context( "Error reading symlink metadata" )
        {
            Ok( m ) => m,
            Err( err ) => return Some( Err( err.into() ) )
        };

        if metadata.is_symlink() {
            debug!( "Found new symlink" );
            trace!( "Read symlink target" );

            let link_target = match current
                .read_link()
                .context( "Failed to read_link" )
            {
                Ok( it ) => it,
                Err( err ) => return Some( Err( err.into() ) )
            };

            self.current = Some( link_target );
        } else {
            trace!( "Not a symlink, the end of symlink chain reached" );
        }

        self.visited_paths.insert( current.clone() );
        return Some( Ok( current ) )
    }
}

#[ derive( Debug ) ]
struct Explainer;

impl Explainer {
    fn explain_path<P>( path: &P )
    where
        P: Display + Debug
    {
        println!( "{path}" )
    }
}
