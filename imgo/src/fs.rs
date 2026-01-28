use std::convert::TryFrom;
use std::num::NonZeroU64;
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;

use anyhow::Context;
use anyhow::ensure;
use tap::Pipe;
use tap::Tap;
use tracing::debug;
use tracing::debug_span;
use tracing::instrument;
use walkdir::DirEntry;
use walkdir::WalkDir;

use crate::Image;
use crate::ImageFormat;

#[derive(Debug, PartialEq, Eq)]
pub struct BaseSeqExt {
    base: String,
    seq: Option<NonZeroU64>,
    ext: Option<String>,
}

impl FromStr for BaseSeqExt {
    type Err = anyhow::Error;

    #[instrument]
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        debug!("Parse Filename");

        // Reject hidden files
        ensure!(!s.starts_with('.'), "Hidden files not allowed: \"{s}\"");

        let parts: Vec<&str> = s.split('.').collect();

        // Reject files without extension
        ensure!(
            parts.len() >= 2,
            r#"Filename "{s}" is missing extension"#
        );

        // Find the seq: first numeric-only part after base
        let mut seq: Option<NonZeroU64> = None;
        let mut seq_index: Option<usize> = None;

        // Find the part that is all number.
        // `name.123.ext`
        //        ^this
        //
        // Notice, the first part can't be seq
        // even if it's all digits (`.skip(1)`).
        for (idx, part) in parts.iter().enumerate().skip(1) {
            let _g = debug_span!("find_seq", idx, part).entered();

            if part.chars().all(|c| c.is_ascii_digit()) {
                debug!("Part is all digits, use it as seq");

                let num: u64 = part
                    .parse()
                    .context("[BUG] Can't parse all digits into number")?;
                ensure!(
                    num != 0,
                    r#"Sequence must be > 0 in filename "{s}""#
                );
                seq = NonZeroU64::new(num)
                    .context("[BUG] Can't create NoneZeroU64")?
                    .tap(|num| debug!(?num))
                    .pipe(Some);
                seq_index = Some(idx);
                break;
            }
        }

        #[expect(clippy::indexing_slicing)]
        let (base, ext) = if let Some(idx) = seq_index {
            ensure!(idx <= parts.len(), "[BUG] Index out of bound");
            let base = &parts[..idx];
            let ext = &parts[(idx + 1)..];
            (base, ext)
        } else {
            // The basis here is to make extension "greedy",
            // i.e. everything after base is considered extension.
            // In practice multiple extension file are so rare,
            // this should be a safe middle ground.
            let base = &parts[..1];
            let ext = &parts[1..];
            (base, ext)
        };

        let base = base.join(".");
        let ext = if matches!(ext, []) {
            None
        } else {
            Some(format!(".{}", ext.join(".")))
        };
        Self { base, seq, ext }.pipe(Ok)
    }
}

impl TryFrom<&Path> for BaseSeqExt {
    type Error = anyhow::Error;

    fn try_from(path: &Path) -> Result<Self, Self::Error> {
        ensure!(path.is_file(), "Not a file: \"{}\"", path.display());
        path.file_name()
            .context("Path has no basename")?
            .to_str()
            .context("Filename is not valid")?
            .parse()
    }
}

impl BaseSeqExt {
    /// Convert back to filename string.
    #[must_use]
    pub fn to_filename(&self) -> String {
        let mut result = self.base.clone();
        if let Some(seq) = self.seq {
            result.push('.');
            result.push_str(&seq.to_string());
        }
        if let Some(ext) = &self.ext {
            result.push_str(ext);
        }
        result
    }

    #[must_use]
    pub fn increment_seq(&self) -> Self {
        let new_seq = self.seq.map_or(Some(1), |n| Some(n.get() + 1));
        Self {
            base: self.base.clone(),
            seq: new_seq.and_then(NonZeroU64::new),
            ext: self.ext.clone(),
        }
    }

    #[must_use]
    pub fn set_ext(&self, ext: &str) -> Self {
        Self {
            base: self.base.clone(),
            seq: self.seq,
            ext: Some(ext.to_string()),
        }
    }
}

/// Collect all images under `workspace` of `formats`.
/// If `recursive` is false, only the immediate children of `workspace` are scanned.
#[instrument]
#[expect(clippy::missing_errors_doc)]
pub fn collect_images(
    workspace: &Path,
    formats: &[ImageFormat],
    recursive: bool,
) -> anyhow::Result<Vec<Image>> {
    debug!("Collect images (recursive={})", recursive);
    ensure!(!formats.is_empty(), "Image formats can't be empty");

    let mut accu = Vec::new();

    let ignore_backup_dir = |e: &DirEntry| {
        e.path().file_name().and_then(|n| n.to_str())
            != Some(crate::BACKUP_DIR_NAME)
    };

    let walker = {
        let w = WalkDir::new(workspace).follow_links(false);
        if recursive { w } else { w.max_depth(1) }
    };

    for entry in walker.into_iter().filter_entry(ignore_backup_dir) {
        let entry = entry.context("WalkDir error")?;
        let path = entry.path();
        let _g = debug_span!("process_entry", ?path).entered();

        ensure!(
            path.is_absolute(),
            "[BUG] walkdir did not yield an absolute path"
        );

        if !entry.file_type().is_file() {
            debug!("Not a file, next");
            continue;
        }

        if let Some(format) = ImageFormat::from_path(&path)
            && formats.contains(&format)
        {
            debug!(?format);
            accu.push(Image {
                path: RelAbs::from_path(workspace, path)?,
                format,
                extra: BaseSeqExt::try_from(path)?.tap(|f| debug!(?f)),
            });
        } else {
            debug!("Unsupported or invalid image format, ignored");
        }
    }
    accu.sort_by(|a, b| {
        let a_path = a.path.original_path();
        let b_path = b.path.original_path();
        natord::compare(
            &b_path.to_string_lossy(),
            &a_path.to_string_lossy(),
        )
    });
    Ok(accu)
}

/// Represents whether a path is relative to `workspace` or absolute.
#[derive(Debug)]
pub enum RelAbs {
    Relative {
        workspace: PathBuf,
        rel_path: PathBuf,
    },
    Absolute {
        path: PathBuf,
    },
}

impl RelAbs {
    #[expect(clippy::missing_errors_doc)]
    #[instrument]
    pub fn from_path(
        workspace: &Path,
        orig_path: &Path,
    ) -> anyhow::Result<Self> {
        debug!("Guess whether path is relative or absolute");
        if orig_path.is_absolute() {
            debug!("Input path is absolute");
            #[expect(clippy::option_if_let_else)]
            if let Ok(rel_path) = orig_path.strip_prefix(workspace) {
                // workspace=/home path=/home/uv
                // path stripped => uv
                debug!("Input path is relative to workspace");
                Ok(Self::Relative {
                    workspace: workspace.to_path_buf(),
                    rel_path: rel_path.to_path_buf(),
                })
            } else {
                debug!("Input path is not relative to workspace");
                Ok(Self::Absolute {
                    path: orig_path.to_path_buf(),
                })
            }
        } else {
            debug!("Input path is relative, use it as-is");
            // Already relative path
            Ok(Self::Relative {
                workspace: workspace.to_path_buf(),
                rel_path: orig_path.to_path_buf(),
            })
        }
    }

    #[must_use]
    pub fn original_path(&self) -> PathBuf {
        match self {
            Self::Relative {
                workspace,
                rel_path,
            } => workspace.join(rel_path),
            Self::Absolute { path } => path.clone(),
        }
    }

    /// Returns the backup path for this image.
    /// For relative paths `a/b.png` -> `backup_dir/a/b.png`
    /// For absolute paths `/mnt/media/a.png` -> `backup_dir/mnt/media/a.png`
    #[must_use]
    pub fn backup_path_structure(&self, backup_dir: &Path) -> PathBuf {
        let rel_path = match self {
            Self::Relative { rel_path, .. } => rel_path.as_path(),
            Self::Absolute { path } => {
                path.strip_prefix("/").unwrap_or(path)
            }
        };
        backup_dir.join(rel_path)
    }

    /// Returns the parent directory of the original path.
    #[must_use]
    pub fn parent_dir(&self) -> Option<PathBuf> {
        self.original_path().parent().map(Path::to_path_buf)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_filename() {
        let f = BaseSeqExt::from_str(".hide");
        assert!(f.is_err());

        let f = BaseSeqExt::from_str("raw");
        assert!(f.is_err());

        let f = BaseSeqExt::from_str("example.123.jpg").unwrap();
        assert_eq!(f.base, "example");
        assert_eq!(f.seq, Some(NonZeroU64::new(123).unwrap()));
        assert_eq!(f.ext, Some(".jpg".into()));

        let f = BaseSeqExt::from_str("base.2.png").unwrap();
        assert_eq!(f.base, "base");
        assert_eq!(f.seq, Some(NonZeroU64::new(2).unwrap()));
        assert_eq!(f.ext, Some(".png".into()));

        let f = BaseSeqExt::from_str("long.3.doc.txt").unwrap();
        assert_eq!(f.base, "long");
        assert_eq!(f.seq, Some(NonZeroU64::new(3).unwrap()));
        assert_eq!(f.ext, Some(".doc.txt".into()));

        let f = BaseSeqExt::from_str("abc.1b.2.docx").unwrap();
        assert_eq!(f.base, "abc.1b");
        assert_eq!(f.seq, Some(NonZeroU64::new(2).unwrap()));
        assert_eq!(f.ext, Some(".docx".into()));

        // Test filename with only base and seq
        let f = BaseSeqExt::from_str("a.123").unwrap();
        assert_eq!(f.base, "a");
        assert_eq!(f.seq, Some(NonZeroU64::new(123).unwrap()));
        assert_eq!(f.ext, None);

        // Test basic inc seq
        let f = BaseSeqExt::from_str("some.2.yo").unwrap();
        let f = f.increment_seq();
        assert_eq!(f.seq, Some(NonZeroU64::new(3).unwrap()));

        //
        // Test increment None seq
        //
        let f = BaseSeqExt::from_str("a.b").unwrap();
        assert_eq!(f.seq, None);
        let f = f.increment_seq();
        assert_eq!(f.seq, Some(NonZeroU64::new(1).unwrap()));

        //
        // Test filenames which contain numbers
        //
        let f = BaseSeqExt::from_str("124.png").unwrap();
        assert_eq!(f.base, "124");
        assert_eq!(f.seq, None);
        assert_eq!(f.ext, Some(".png".into()));
    }
}
