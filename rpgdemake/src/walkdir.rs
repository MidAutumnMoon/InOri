use std::path::{
    Path,
    PathBuf,
};

use tracing::debug;

use crate::asset::Asset;

use walkdir::WalkDir;


/// Find files in `toplevel` which [`Asset::is_rpgmv_file`].
#[ tracing::instrument ]
pub fn find_all( toplevel: &Path )
    -> anyhow::Result< Vec<PathBuf> >
{

    debug!( "find all files" );

    let mut files =
        Vec::with_capacity( crate::EYEBALLED_AVERAGE );

    for entry in WalkDir::new( toplevel ) {
        debug!( ?entry );

        let entry = entry?;
        let path = entry.path();

        if ! path.is_file() {
            debug!( "not file, skip" );
            continue
        }

        if Asset::real_extension( path ).is_some() {
            debug!( "found file" );
            files.push( path.to_owned() )
        }
    }

    debug!( ?files, "collected files" );

    Ok( files )

}
