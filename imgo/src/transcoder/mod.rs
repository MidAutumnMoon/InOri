use std::num::NonZeroU64;
use std::path::Path;
use std::process::Command;

use image::RgbaImage;

use crate::ImageFormat;

pub mod avif;
pub mod jxl;
pub mod magick;
pub mod tomato;

pub use tomato::Tomato;

/// Metadata shared by all transcoder kinds.
pub trait Meta: Send + Sync {
    /// A string id representing this transcoder.
    fn id(&self) -> &'static str;

    /// Formats that this transcoder accepts as input.
    fn input_formats(&self) -> &'static [ImageFormat];

    /// Formats that this transcoder can output.
    fn output_format(&self) -> ImageFormat;

    /// Default number of parallel jobs.
    fn default_jobs(&self) -> NonZeroU64;
}

/// Shell-out transcoders. Sans-IO: returns a process declaration; the
/// orchestrator spawns it.
pub trait External: Meta {
    /// Generate the transcoding command.
    fn transcode(&self, input: &Path, output: &Path) -> Command;
}

/// In-process pixel transcoders. Sans-IO: the orchestrator decodes the
/// input and encodes the output; the impl only mutates pixels.
pub trait Pixel: Meta {
    /// Transform the decoded RGBA image in place.
    ///
    /// # Errors
    ///
    /// Implementations may return an error if the transform cannot be
    /// applied (e.g. invalid parameters).
    fn transform(&self, img: &mut RgbaImage) -> anyhow::Result<()>;
}
