use tracing::debug;
use tracing::trace;

use std::path::PathBuf;

/// Walk through all directories in $PATH, search for
/// the executable of `name` in each one. Returns a list
/// of paths that have it.
#[ tracing::instrument ]
pub fn lookup_executable_in_path( program: &str ) -> Vec<PathBuf> {
    debug!( "Try find executable in $PATH" );

    let env_path = std::env::var_os( "PATH" )
        .expect( "Can't read $PATH!?" )
    ;

    debug!( ?env_path );

    let mut findings = Vec::with_capacity( 10 );

    for dir in std::env::split_paths( &env_path ) {
        use is_executable::IsExecutable;

        trace!( ?dir, "Look into directory" );
        let full_path = dir.join( program );
        trace!( ?full_path );

        if full_path.is_executable() {
            debug!( ?full_path, "Found executable" );
            findings.push( full_path );
        }
    }

    findings
}