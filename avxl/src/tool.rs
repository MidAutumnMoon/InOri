use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result as AnyResult;
use tap::Pipe;
use tracing::debug;

use crate::Picture;
use crate::StaticStrs;

// #[ tracing::instrument ]
// pub fn find_files( parent: &Path ) -> AnyResult<Vec<PathBuf>> {
//     debug!( "collect files" );
//
//     let mut collected: Vec<PathBuf> = vec![];
//
//     for entry in parent.read_dir()? {
//         let path = entry?.path();
//         if path.is_file() { collected.push( path ) }
//     }
//
//     Ok( collected )
// }

// #[ tracing::instrument( skip_all ) ]
// pub fn filter_by_supported_exts(
//     encoder: &Box<dyn crate::Transcoder>,
//     paths: Vec<PathBuf>
// ) -> Vec<PathBuf> {
//     let mut collected: Vec<PathBuf> =
//         Vec::with_capacity( paths.len() );
//
//     for p in paths {
//         let _span = tracing::debug_span!( "path", ?p ).entered();
//
//         let Some( ext ) = p.extension() else {
//             debug!( "no extension" );
//             continue;
//         };
//
//         let Some( ext ) = ext.to_str() else {
//             debug!( "failed OsStr to str convertion" );
//             continue;
//         };
//
//         let ext = ext.to_lowercase();
//
//         if encoder.input_extensions( &ext ) {
//             debug!( "ext .{ext} ok" );
//             collected.push( p );
//         } else {
//             debug!( "ext .{ext} is not supported" );
//         }
//     }
//
//     collected
// }

#[ tracing::instrument( skip_all ) ]
pub fn list_pictures_recursively(
    topleve: &Path,
    input_extensions: StaticStrs,
    output_extension: &'static str,
)
    -> AnyResult<Vec<Picture>>
{
    todo!()
}

pub trait UnwrapOrCwd {
    fn unwrap_or_cwd( self ) -> AnyResult<PathBuf>;
}

impl UnwrapOrCwd for Option<PathBuf> {
    fn unwrap_or_cwd( self ) -> AnyResult<PathBuf> {
        match self {
            Some( w ) => w,
            None => std::env::current_dir()
                .context( "Failed to get current directory" )?,
        }.pipe( Ok )
    }
}
