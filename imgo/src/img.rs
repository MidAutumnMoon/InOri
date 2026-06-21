use std::path::Path;

use crate::BaseSeqExt;
use crate::RelAbs;

#[allow(clippy::upper_case_acronyms)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[derive(strum::EnumIter)]
#[derive(strum::VariantArray)]
pub enum ImageFormat {
    PNG,
    JPG,
    WEBP,
    AVIF,
    JXL,
    GIF,
}

impl ImageFormat {
    /// Extensions of each image format.
    #[inline]
    #[must_use]
    pub fn exts(&self) -> &'static [&'static str] {
        match self {
            Self::PNG => &["png"],
            Self::JPG => &["jpg", "jpeg"],
            Self::WEBP => &["webp"],
            Self::AVIF => &["avif"],
            Self::JXL => &["jxl"],
            Self::GIF => &["gif"],
        }
    }

    /// Guess the picture's format based on the extension of the path.
    #[inline]
    #[must_use]
    pub fn from_path(path: &impl AsRef<Path>) -> Option<Self> {
        use strum::IntoEnumIterator;
        if let Some(ext) = path.as_ref().extension()
            && let Some(ext) = ext.to_str()
        {
            Self::iter().find(|fmt| fmt.exts().contains(&ext))
        } else {
            None
        }
    }
}

/// Represents an input image.
#[derive(Debug)]
pub struct Image {
    pub path: RelAbs,
    pub format: ImageFormat,
    pub extra: BaseSeqExt,
}
