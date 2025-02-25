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
use tap::Pipe;

const MAX_SYMLINK_FOLLOWS: u64 = 64;

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
    /// If it starts with "/", "../" or "./", the symlink walk
    /// will start with it directly instead of lookup an executable in $PATH.
    program: String,
}

///
enum ProgramKind {
    Name( String ),
    Path( String ),
}

impl ProgramKind {
    fn new( input: &str ) -> Self {
        if [ "/", "./", "../" ]
            .into_iter()
            .any( |prefix| input.starts_with( prefix ) )
        {
            Self::Path( input.to_owned() )
        } else {
            Self::Name( input.to_owned() )
        }
    }
}

impl Application {
    #[ tracing::instrument ]
    fn run( &self ) -> anyhow::Result<()> {
        trace!( "Start application" );

        let starter = match ProgramKind::new( &self.program ) {
            ProgramKind::Name( name ) => {
                let errmsg =
                    || anyhow::anyhow!( r#"Program "{}" not found"#, &name );
                coruma::lookup_executable_in_path( &name )
                    .first()
                    .ok_or_else( errmsg )?
                    .to_owned()
            },
            ProgramKind::Path( it ) => PathBuf::from( it ),
        };

        debug!( ?starter );

        SymlinkAncestor::new( &starter )
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
    symlink_followed: u64,
}

impl SymlinkAncestor {
    fn new( starter: &Path ) -> Self {
        Self {
            current: Some( starter.to_owned() ),
            visited_paths: Default::default(),
            symlink_followed: 0,
        }
    }
}

impl Iterator for SymlinkAncestor {
    type Item = anyhow::Result<PathBuf>;

    fn next( &mut self ) -> Option< Self::Item > {
        let _s = tracing::debug_span!( "iter_next" ).entered();

        let current = self.current.take()?;
        debug!( ?current );

        if self.visited_paths.contains( &current ) {
            debug!( "Already visited this path" );
            let err = anyhow::anyhow!( "Symlink loop detected" );
            return Some( Err( err ) )
        }

        if self.symlink_followed + 1 > MAX_SYMLINK_FOLLOWS {
            return anyhow::anyhow!( "Exceeded the maximum symlink follows allowed" )
                .pipe( |it| Some( Err( it ) ) )
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
