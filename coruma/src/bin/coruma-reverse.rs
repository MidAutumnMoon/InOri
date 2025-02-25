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
                .context( "Unable to walk through symlink" )?
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

        // N.B. self.current became None after take()
        // it stays None as long as not set again
        let current = self.current.take()?;
        debug!( ?current );

        // Step 1. Check for symlink loop

        if self.visited_paths.contains( &current ) {
            debug!( "Already visited this path" );
            let errmsg = anyhow::anyhow!(
                r#"Symlink loop detected, path: "{}""#,
                current.display()
            );
            return Some( Err( errmsg ) )
        }

        // Step 2. Guard against limitation

        if self.symlink_followed + 1 > MAX_SYMLINK_FOLLOWS {
            return anyhow::anyhow!( "Exceeded the maximum symlink follows allowed" )
                .pipe( |it| Some( Err( it ) ) )
        } else {
            self.symlink_followed += 1;
        }

        trace!( "Read symlink metadata" );

        // Step 3. Prepare for next iteration

        // is_symlink() does not traverse symlink
        if current.is_symlink() {
            debug!( "Found new symlink" );
            let errmsg = || format!(
                r#"Error reading symlink "{}""#,
                current.display()
            );
            let symlink_target = match current
                .read_link()
                .with_context( errmsg )
            {
                Ok( it ) => it,
                Err( err ) => return Some( Err( err.into() ) )
            };
            // Sets self.current to Some,
            // so that the next iteration will happend
            self.current = Some( symlink_target );
        } else {
            // Here, self.current is not set and stays None,
            // which skips next iteration
            trace!( "Not a symlink, the end of symlink chain is reached" );
        }

        // Step 4. Book current as traversed and yield

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
