use std::path::Path;
use std::path::PathBuf;

use tracing::debug;
use tracing::trace;
use tracing::trace_span;
use walkdir::DirEntry;
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
    for entry in WalkDir::new(root)
        // be more explicit
        .follow_links(false)
        .into_iter()
        .filter_entry(skip_backup_dir)
    {
        let Ok(entry) = entry else {
            trace!(?entry, "entry gives an error, ignored");
            continue;
        };
        if entry.file_type().is_dir() {
            trace!(?entry, "entry is a dir, ignored");
            continue;
        }
        if entry.file_type().is_symlink() {
            let path = entry.path();
            if let Ok(canon) = path.canonicalize() {
                if canon.is_dir() {
                    trace!(?path, "points to dir, ignored");
                    continue;
                }
            } else {
                trace!(?path, "error when canonicalizing, ignored");
                continue;
            }
        }

        let path = entry.path();
        let _s = trace_span!("process_path", ?path).entered();

        if let Some(ext) = path.extension()
            && let Some(ext) = ext.to_str()
        {
            if formats.iter().any(|f| f.exts().contains(&ext)) {
                debug!("found supported picture");
                collected.push(path.to_owned());
            } else {
                trace!("extension does not match, skipped");
            }
        } else {
            trace!("can not get entry's extension, ignored");
        }
    }

    collected
}

#[inline]
fn skip_backup_dir(entry: &DirEntry) -> bool {
    if entry.file_type().is_dir() {
        if let Some(basename) = entry.path().file_name()
            && let Some(basename) = basename.to_str()
            && basename == crate::BACKUP_DIR_NAME
        {
            // "false" tells walkdir to skip the entry
            false
        } else {
            // Mostly caused by path not having a basename,
            // in such case just let walkdir continue.
            true
        }
    } else {
        // It's very rare to have picture named as ".backup".
        // But anyway, it doesn't matter. Let walkdir continue.
        true
    }
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
