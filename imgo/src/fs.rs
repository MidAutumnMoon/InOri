use std::path::Path;
use std::path::PathBuf;

use rootcause::bail;
use rootcause::option_ext::OptionExt as _;
use rootcause::prelude::ResultExt as _;
use tracing::debug;
use tracing::instrument;
use walkdir::DirEntry;
use walkdir::WalkDir;

use crate::BACKUP_DIR_NAME;
use crate::REVIEW_DIR_NAME;
use crate::img::Image;
use crate::img::ImageFormat;

/// Collect images under `workspace` in deterministic natural-path order.
/// Generated backups and review bundles are never re-ingested.
///
/// # Errors
///
/// Returns an error for an empty format set, an inaccessible/non-directory
/// workspace, or a traversal failure.
#[instrument]
pub fn collect_images(
    workspace: &Path,
    formats: &[ImageFormat],
    recursive: bool,
) -> rootcause::Result<Vec<Image>> {
    if formats.is_empty() {
        bail!("image formats cannot be empty");
    }
    let workspace = workspace.canonicalize().context_with(|| {
        format!("resolve workspace {}", workspace.display())
    })?;
    if !workspace.is_dir() {
        bail!("workspace is not a directory");
    }

    let walker = WalkDir::new(&workspace).follow_links(false);
    let walker = if recursive {
        walker
    } else {
        walker.max_depth(1)
    };

    let include_entry = |entry: &DirEntry| {
        if !entry.file_type().is_dir() {
            return true;
        }
        let name = entry.file_name().to_str();
        name != Some(BACKUP_DIR_NAME) && name != Some(REVIEW_DIR_NAME)
    };

    let mut images = Vec::new();
    for entry in walker.into_iter().filter_entry(include_entry) {
        let entry = entry.context("walk image workspace")?;
        #[expect(
            clippy::filetype_is_file,
            reason = "only regular files are valid image inputs; symlinks and special files are skipped"
        )]
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.into_path();
        if let Some(format) = ImageFormat::from_path(&path)
            && formats.contains(&format)
        {
            debug!(?path, ?format, "discovered image");
            images.push(Image { path, format });
        }
    }

    images.sort_by(|left, right| {
        natord::compare(
            &left.path.to_string_lossy(),
            &right.path.to_string_lossy(),
        )
    });
    Ok(images)
}

/// Resolve a user-selected file relative to the workspace unless it is
/// already absolute.
///
/// # Errors
///
/// Returns an error when the selection is missing, not a regular file, has an
/// unsupported extension, or is not accepted by the recipe.
pub fn selected_image(
    workspace: &Path,
    selected: &Path,
    accepted: &[ImageFormat],
) -> rootcause::Result<Image> {
    let path = if selected.is_absolute() {
        selected.to_path_buf()
    } else {
        workspace.join(selected)
    };
    let path = path.canonicalize().context_with(|| {
        format!("resolve selected image {}", path.display())
    })?;
    if !path.is_file() {
        bail!("selection is not a file: {}", path.display());
    }
    let format = ImageFormat::from_path(&path).context_with(|| {
        format!("unsupported image extension: {}", path.display())
    })?;
    if !accepted.contains(&format) {
        bail!(
            "format {:?} of {} is not accepted by this recipe",
            format,
            path.display()
        );
    }
    Ok(Image { path, format })
}

#[must_use]
pub fn destination_path(source: &Path, output: ImageFormat) -> PathBuf {
    let extension = output.primary_extension().unwrap_or("output");
    source.with_extension(extension)
}

/// Preserve the workspace-relative tree under `.backup`. Absolute selections
/// outside the workspace are namespaced by their rootless absolute path.
#[must_use]
pub fn backup_path(workspace: &Path, source: &Path) -> PathBuf {
    let relative = source.strip_prefix(workspace).unwrap_or_else(|_| {
        source.strip_prefix(Path::new("/")).unwrap_or(source)
    });
    workspace.join(BACKUP_DIR_NAME).join(relative)
}
