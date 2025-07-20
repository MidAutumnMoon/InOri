use std::path::Path;
use std::path::PathBuf;

use anyhow::bail;
use anyhow::Context;
use anyhow::Result as AnyResult;
use ino_path::PathExt;
use tap::Pipe;
use tracing::debug;
use tracing::debug_span;
use tracing::trace;
use walkdir::WalkDir;

use crate::Picture;
use crate::StaticStrs;
use crate::BACKUP_DIR_NAME;

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
    debug!( "list all files" );

    let mut input_files = vec![];
    for entry in WalkDir::new( topleve ).follow_links( false ) {
        let entry = entry.context( "Failed to read entry" )?;
        let path = entry.path();
        let _span = debug_span!( "inspect_path", ?path ).entered();

        if path.is_dir_no_traverse()? {
            trace!( "dir, ignore" );
            continue;
        }

        if let Some( ext ) = path.extension()
            && let Some( ext ) = ext.to_str()
            && input_extensions.contains( &ext )
        {
            trace!( ?path, "found picture" );
            input_files.push( path.to_owned() );
        } else {
            trace!( ?path, "ignore path, ext not supported" );
        }
    }

    let mut pictures = vec![];
    for input in input_files {
        let _span = debug_span!( "path_to_picture", ?input ).entered();
        let output = {
            let mut p = input.clone();
            if !p.set_extension( output_extension ) {
                bail!( "[BUG] Failed to set extension for {}", input.display() );
            }
            p
        };
        let backup = {
            let Ok( base ) = input.strip_prefix( topleve ) else {
                bail!( "[BUG] Failed to remove toplevel prefix" );
            };
            topleve
                .join( BACKUP_DIR_NAME )
                .join( base )
        };
        pictures.push( Picture { input, output, backup, } );
    }

    Ok( pictures )
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
