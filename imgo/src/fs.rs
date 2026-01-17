use std::convert::TryFrom;
use std::num::NonZeroU64;
use std::path::Path;
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

use crate::ImageFormat;
use crate::InputImage;

#[derive(Debug, PartialEq, Eq)]
pub struct Filename {
    base: String,
    seq: Option<NonZeroU64>,
    ext: Option<String>,
}

impl FromStr for Filename {
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

impl TryFrom<&Path> for Filename {
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

impl Filename {
    #[must_use]
    pub fn increment_seq(&self) -> Self {
        let new_seq = self.seq.map_or(Some(1), |n| Some(n.get() + 1));
        Self {
            base: self.base.clone(),
            seq: new_seq.and_then(NonZeroU64::new),
            ext: self.ext.clone(),
        }
    }

    pub fn join(&self) -> anyhow::Result<String> {
        // ensure!(
        //     self.ext.starts_with('.'),
        //     "[BUG] Extension must start with a '.'"
        // );
        todo!()
    }
}

/// Recursively collect all images under `toplevel` of `formats`.
#[instrument]
#[expect(clippy::missing_errors_doc)]
pub fn collect_images(
    toplevel: &Path,
    formats: &[ImageFormat],
) -> anyhow::Result<Vec<InputImage>> {
    ensure!(!formats.is_empty(), "Image formats can't be empty");

    let mut accu = Vec::new();

    let ignore_backup_dir = |e: &DirEntry| {
        e.path().file_name().and_then(|n| n.to_str())
            == Some(crate::BACKUP_DIR_NAME)
    };

    for entry in WalkDir::new(toplevel)
        .follow_links(false)
        .into_iter()
        .filter_entry(ignore_backup_dir)
    {
        let entry = entry.context("WalkDir error")?;
        let path = entry.path();

        if !entry.file_type().is_file() {
            continue;
        }

        if let Some(format) = ImageFormat::from_path(&path)
            && formats.contains(&format)
        {
            accu.push(InputImage {
                src: path.into(),
                format,
            });
        }
    }

    Ok(accu)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_filename() {
        let f = Filename::from_str(".hide");
        assert!(f.is_err());

        let f = Filename::from_str("raw");
        assert!(f.is_err());

        let f = Filename::from_str("example.123.jpg").unwrap();
        assert_eq!(f.base, "example");
        assert_eq!(f.seq, Some(NonZeroU64::new(123).unwrap()));
        assert_eq!(f.ext, Some(".jpg".into()));

        let f = Filename::from_str("base.2.png").unwrap();
        assert_eq!(f.base, "base");
        assert_eq!(f.seq, Some(NonZeroU64::new(2).unwrap()));
        assert_eq!(f.ext, Some(".png".into()));

        let f = Filename::from_str("long.3.doc.txt").unwrap();
        assert_eq!(f.base, "long");
        assert_eq!(f.seq, Some(NonZeroU64::new(3).unwrap()));
        assert_eq!(f.ext, Some(".doc.txt".into()));

        let f = Filename::from_str("abc.1b.2.docx").unwrap();
        assert_eq!(f.base, "abc.1b");
        assert_eq!(f.seq, Some(NonZeroU64::new(2).unwrap()));
        assert_eq!(f.ext, Some(".docx".into()));

        // Test filename with only base and seq
        let f = Filename::from_str("a.123").unwrap();
        assert_eq!(f.base, "a");
        assert_eq!(f.seq, Some(NonZeroU64::new(123).unwrap()));
        assert_eq!(f.ext, None);

        // Test basic inc seq
        let f = Filename::from_str("some.2.yo").unwrap();
        let f = f.increment_seq();
        assert_eq!(f.seq, Some(NonZeroU64::new(3).unwrap()));

        //
        // Test increment None seq
        //
        let f = Filename::from_str("a.b").unwrap();
        assert_eq!(f.seq, None);
        let f = f.increment_seq();
        assert_eq!(f.seq, Some(NonZeroU64::new(1).unwrap()));

        //
        // Test filenames which contain numbers
        //
        let f = Filename::from_str("124.png").unwrap();
        assert_eq!(f.base, "124");
        assert_eq!(f.seq, None);
        assert_eq!(f.ext, Some(".png".into()));
    }
}
