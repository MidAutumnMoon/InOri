use std::path::Path;

pub trait Transcoder {
    fn accepted_input(&self) -> &'static [Picture];
    fn produced_output(&self) -> Picture;
}

#[allow(clippy::upper_case_acronyms)]
#[derive(Debug)]
#[derive(Clone, Copy)]
#[derive(strum::EnumIter)]
pub enum Picture {
    PNG,
    JPG,
    WEBP,
    AVIF,
    JXL,
    GIF,
}

impl Picture {
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
