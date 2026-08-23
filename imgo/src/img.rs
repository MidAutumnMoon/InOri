use std::path::Path;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[derive(strum::EnumIter, strum::VariantArray)]
pub enum ImageFormat {
    PNG,
    JPG,
    WEBP,
    AVIF,
    JXL,
    GIF,
}

impl ImageFormat {
    /// Extensions accepted for this format.
    #[inline]
    #[must_use]
    pub const fn exts(self) -> &'static [&'static str] {
        match self {
            Self::PNG => &["png"],
            Self::JPG => &["jpg", "jpeg"],
            Self::WEBP => &["webp"],
            Self::AVIF => &["avif"],
            Self::JXL => &["jxl"],
            Self::GIF => &["gif"],
        }
    }

    #[must_use]
    pub fn primary_extension(self) -> Option<&'static str> {
        self.exts().first().copied()
    }

    /// Guess the picture format from a case-insensitive extension.
    #[inline]
    #[must_use]
    pub fn from_path(path: impl AsRef<Path>) -> Option<Self> {
        use strum::IntoEnumIterator as _;

        let extension = path.as_ref().extension()?.to_str()?;
        Self::iter().find(|format| {
            format
                .exts()
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(extension))
        })
    }
}

/// A discovered source image. `path` is absolute so execution never depends
/// on a worker thread's ambient current directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub path: PathBuf,
    pub format: ImageFormat,
}
