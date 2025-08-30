use std::path::Path;
use std::path::PathBuf;

use tracing::debug;
use tracing::trace;
use tracing::trace_span;
use walkdir::WalkDir;

use crate::PictureFormat;

/// Find all pictures under toplevel matching given formats.
// TODO: don't swallow errors?
#[tracing::instrument]
pub fn collect_pictures(
    root: &Path,
    formats: &[PictureFormat],
) -> Vec<PathBuf> {
    debug!("collect pictures");
    let mut collected = vec![];

    // TODO: cleanup
    for entry in WalkDir::new(root).follow_links(false) {
        let Ok(entry) = entry else {
            trace!(?entry, "entry gives an error, ignored");
            continue;
        };
        if entry.file_type().is_dir() {
            trace!(?entry, "entry is a dir, ignored");
            continue;
        }
        let path = entry.path();
        let _s = trace_span!("process_path", ?path).entered();

        if let Some(ext) = path.extension()
            && let Some(ext) = ext.to_str()
        {
            if formats.iter().all(|f| f.exts().contains(&ext)) {
                debug!("found supported picture");
                collected.push(path.to_owned());
            } else {
                trace!("extension does not match, ignored");
            }
        } else {
            trace!("can not get entry's extension, ignored");
        }
    }

    collected
}

/// Partition a list of paths into untagged and tagged ones.
/// The first element of the returned tuple contains the untagged pictures
/// while the second one contains the tagged pictures.
/// Symlinks are not followed.
pub fn partition_tagged_picture(
    pictures: Vec<PathBuf>,
) -> (Vec<PathBuf>, Vec<PathBuf>) {
    todo!()
}
