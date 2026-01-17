use std::path::PathBuf;

pub trait Transcoder {}

#[allow(clippy::upper_case_acronyms)]
#[derive(Debug)]
#[derive(Clone, Copy)]
#[derive(strum::EnumIter)]
pub enum ImageFormat {
    PNG,
    JPG,
    WEBP,
    AVIF,
    JXL,
    GIF,
}

/// Represents an input image.
#[derive(Debug)]
pub struct InputImage {
    src: PathBuf,
    format: ImageFormat,
}

/// Represents an output image.
#[derive(Debug)]
pub struct OutputImage {
    dst: PathBuf,
    format: ImageFormat,
}

/// Represents the process of transcoding.
#[derive(Debug)]
pub struct TranscodeProcess {
    input: InputImage,
    output: OutputImage,
}
