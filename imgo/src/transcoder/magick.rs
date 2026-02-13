use std::num::NonZeroU64;
use std::path::Path;
use std::process::Command;
use std::thread::available_parallelism;

use tap::Pipe;

use crate::ImageFormat;
use crate::Transcoder;

pub const MAGICK_PATH: Option<&str> = std::option_env!("CFG_MAGICK_PATH");

/// Various imagemagick tricks to remove various kinds of noise.
#[derive(Debug, clap::Args)]
#[group(id = "DenoiseTranscoder")]
pub struct Denoise {
    #[arg(long, short)]
    #[arg(default_value = "artifact")]
    pub mode: Mode,

    /// The strength of the denoise. Different mode has different settings.
    /// Read the doc of imagemagick.
    #[arg(long, short)]
    strength: Option<String>,
}

#[derive(Clone, Debug)]
#[derive(Default)]
#[derive(clap::ValueEnum)]
pub enum Mode {
    /// Use `-adaptive-blue` to remove artifacts resulted from JPEG compression.
    /// The default strength is `2x0.8`.
    #[default]
    Artifact,

    ///  Use `-contrast-stretch` to remove fake pencil-style noise
    /// (black strokes consist of noise pixels).
    /// The default strength is `5%x0%`.
    FakePencil,
}

impl Transcoder for Denoise {
    fn id(&self) -> &'static str {
        "magick despeckle"
    }

    fn default_jobs(&self) -> NonZeroU64 {
        #[expect(clippy::unwrap_used)]
        NonZeroU64::new(2).unwrap()
    }

    fn input_formats(&self) -> &'static [ImageFormat] {
        &[ImageFormat::PNG, ImageFormat::JPG, ImageFormat::WEBP]
    }

    fn output_format(&self) -> ImageFormat {
        ImageFormat::PNG
    }

    fn transcode(&self, input: &Path, output: &Path) -> Command {
        let mut cmd = MAGICK_PATH.unwrap_or("magick").pipe(Command::new);

        cmd.arg("-verbose");
        cmd.arg(input);

        match self.mode {
            Mode::Artifact => {
                let strength = self.strength.as_deref().unwrap_or("2x0.8");
                cmd.args(["-adaptive-blur", strength]);
            }
            Mode::FakePencil => {
                let strength = self.strength.as_deref().unwrap_or("5%x0%");
                cmd.args(["-statistic", "median", "3x3"]);
                cmd.args(["-contrast-stretch", strength]);
            }
        }

        // Images later to be processed by avifenc
        cmd.args(["-define", "png:compression-level=1"]);
        cmd.arg(output);
        cmd
    }
}

#[derive(Debug, clap::Args)]
#[group(id = "CleanScanTranscoder")]
pub struct CleanScan {}

impl Transcoder for CleanScan {
    fn id(&self) -> &'static str {
        "magick clean-scan"
    }

    fn default_jobs(&self) -> NonZeroU64 {
        eighth_of_total_cores()
    }

    fn input_formats(&self) -> &'static [ImageFormat] {
        &[ImageFormat::PNG, ImageFormat::JPG]
    }

    fn output_format(&self) -> ImageFormat {
        ImageFormat::PNG
    }

    fn transcode(&self, input: &Path, output: &Path) -> Command {
        let mut cmd = MAGICK_PATH.unwrap_or("magick").pipe(Command::new);
        cmd.arg("-verbose");
        cmd.arg(input);
        cmd.args(["-colorspace", "Gray"]);
        cmd.arg("-strip");
        cmd.args(["-unsharp", "0x2+1+0.4"]);
        cmd.args(["-threshold", "55%"]);
        cmd.args(["-background", "black", "-alpha", "remove"]);
        cmd.args(["-depth", "1", "-colors", "2"]);
        cmd.args(["-define", "png:compression-level=1"]);
        cmd.arg(output);
        cmd
    }
}

#[inline]
#[expect(clippy::unwrap_used)]
fn eighth_of_total_cores() -> NonZeroU64 {
    let cores = available_parallelism()
        .expect("Failed to get core numbers")
        .get();
    NonZeroU64::new(((cores as u64 * 80) / 100).max(1)).unwrap()
}
