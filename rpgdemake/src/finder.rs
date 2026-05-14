use std::path::Path;
use std::path::PathBuf;

use walkdir::WalkDir;

use rayon::prelude::*;

use crate::lore::map_encrypted_extension;

#[tracing::instrument]
pub fn find_all(toplevel: &Path) -> anyhow::Result<Vec<PathBuf>> {
    use itertools::Itertools;

    let files =
        WalkDir::new(toplevel).into_iter().process_results(|iter| {
            iter.par_bridge()
                .map(|entry| entry.path().to_owned())
                .filter(|path| path.is_file())
                .filter_map(|path| {
                    let ext = path.extension()?.to_str()?;
                    map_encrypted_extension(ext)?;
                    Some(path)
                })
                .collect()
        })?;

    Ok(files)
}
