use std::path::{
    Path,
    PathBuf,
};

use tracing::debug;

use crate::resource::Resource;

use walkdir::WalkDir;


#[ tracing::instrument ]
pub fn find_all( toplevel: &Path )
    -> anyhow::Result< Vec<PathBuf> >
{
    debug!( "find all files" );

    let mut files = Vec::new();

    for entry in WalkDir::new( toplevel ) {
        debug!( ?entry );

        let entry = entry?;
        let path = entry.path();

        if ! path.is_file() {
            debug!( "not file, skip" );
            continue
        }

        if Resource::real_extension( path ).is_some() {
            debug!( "found file" );
            files.push( path.to_owned() )
        }
    }

    debug!( ?files, "collected files" );

    Ok( files )
}
