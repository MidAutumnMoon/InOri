use std::path::{
    Path,
    PathBuf,
};

use crate::resource::Resource;

use walkdir::WalkDir;


#[ tracing::instrument ]
pub fn find_all( toplevel: &Path )
    -> anyhow::Result< Vec<PathBuf> >
{
    let files = WalkDir::new( toplevel )
        .into_iter()
            .collect::< Result<Vec<_>, _> >()?
        .into_iter()
            .map( |e| e.path().to_owned() )
            .filter( |p| p.is_file() )
            .filter( |f| Resource::real_extension( f ).is_some() )
        .collect()
    ;
    Ok( files )
}
