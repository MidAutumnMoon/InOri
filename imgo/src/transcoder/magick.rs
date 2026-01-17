use std::num::NonZeroU64;
use std::path::Path;
use std::process::Command;
use std::thread::available_parallelism;

use itertools::Itertools;
use tap::Pipe;

use crate::ImageFormat;
use crate::Transcoder;

pub const MAGICK_PATH: Option<&str> = std::option_env!("CFG_MAGICK_PATH");

#[derive(Debug, clap::Args)]
#[group(id = "DespeckleTranscoder")]
pub struct Despeckle {
    /// How many despeckle passes to run on the picture
    #[arg(long, short)]
    #[arg(default_value = "4")]
    pub iteration: NonZeroU64,
}

impl Transcoder for Despeckle {
    fn id(&self) -> &'static str {
        "magick despeckle"
    }

    fn default_jobs(&self) -> NonZeroU64 {
        #[expect(clippy::unwrap_used)]
        NonZeroU64::new(2).unwrap()
    }

    fn input_formats(&self) -> &'static [ImageFormat] {
        &[ImageFormat::PNG, ImageFormat::JPG]
    }

    fn output_format(&self) -> ImageFormat {
        ImageFormat::PNG
    }

    fn transcode(&self, input: &Path, output: &Path) -> Command {
        #[expect(clippy::cast_possible_truncation)]
        let iterations = std::iter::repeat_n(
            "-despeckle",
            self.iteration.get() as usize,
        )
        .collect_vec();

        let mut cmd = MAGICK_PATH.unwrap_or("magick").pipe(Command::new);
        cmd.arg("-verbose");
        cmd.arg("--");
        cmd.arg(input);
        cmd.args(iterations);
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
        eightth_of_total_cores()
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
fn eightth_of_total_cores() -> NonZeroU64 {
    let cores = available_parallelism()
        .expect("Failed to get core numbers")
        .get();
    NonZeroU64::new(cores as u64 / 8).unwrap()
}
