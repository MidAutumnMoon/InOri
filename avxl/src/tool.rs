use std::path::{ Path, PathBuf };

use anyhow::Result as AnyResult;
use tracing::debug;

#[ tracing::instrument ]
pub fn find_files( parent: &Path ) -> AnyResult<Vec<PathBuf>> {
    debug!( "collect files" );

    let mut collected: Vec<PathBuf> = vec![];

    for entry in parent.read_dir()? {
        let path = entry?.path();
        if path.is_file() { collected.push( path ) }
    }

    Ok( collected )
}


#[ tracing::instrument( skip_all ) ]
pub fn filter_by_supported_exts(
    encoder: &Box<dyn crate::Transcoder>,
    paths: Vec<PathBuf>
) -> Vec<PathBuf> {
    let mut collected: Vec<PathBuf> =
        Vec::with_capacity( paths.len() );

    for p in paths {
        let _span = tracing::debug_span!( "path", ?p ).entered();

        let Some( ext ) = p.extension() else {
            debug!( "no extension" );
            continue;
        };

        let Some( ext ) = ext.to_str() else {
            debug!( "failed OsStr to str convertion" );
            continue;
        };

        let ext = ext.to_lowercase();

        if encoder.supported_extension( &ext ) {
            debug!( "ext .{ext} ok" );
            collected.push( p );
        } else {
            debug!( "ext .{ext} is not supported" );
        }
    }

    collected
}
