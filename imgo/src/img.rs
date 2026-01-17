use std::num::NonZeroUsize;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

pub trait Transcoder {
    /// Formats that this transcoder accepts as input.
    fn input_formats(&self) -> &'static [ImageFormat];
    /// Formats that this transcoder can output.
    fn output_formats(&self) -> &'static [ImageFormat];
    /// Default number of parallel jobs.
    fn default_jobs(&self) -> NonZeroUsize;
    /// Generate the transcoding command.
    fn transcode_command(&self, transcation: Transcation) -> Command;
}

#[allow(clippy::upper_case_acronyms)]
#[derive(Debug, Clone, Copy, PartialEq, strum::EnumIter)]
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
pub struct InputImage {
    pub src: PathBuf,
    pub format: ImageFormat,
}

/// Represents an output image.
#[derive(Debug)]
pub struct OutputImage {
    dst: PathBuf,
    format: ImageFormat,
}

/// Represents the process of transcoding.
#[derive(Debug)]
pub struct Transcation {
    input: InputImage,
    output: OutputImage,
}
