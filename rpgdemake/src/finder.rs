use std::path::{
    Path,
    PathBuf,
};

use walkdir::WalkDir;

use rayon::prelude::*;

use crate::task::Validate;


#[ tracing::instrument ]
pub fn find_all( toplevel: &Path )
    -> anyhow::Result< Vec<PathBuf> >
{
    use itertools::Itertools;

    let files = WalkDir::new( toplevel )
        .into_iter()
        .process_results( |iter| {
            iter.par_bridge()
                .map( |entry| entry.path().to_owned() )
                .filter( |path| path.is_file() )
                .filter_map( |path| {
                    let ext = path.extension()?.to_str()?;
                    Validate::map_extension( ext )
                        .and( Some( path ) )
                } )
                .collect()
        } )?
    ;

    Ok( files )
}
