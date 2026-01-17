use std::num::NonZeroU64;
use std::path::Path;
use std::process::Command;

use crate::ImageFormat;
use crate::Transcoder;

const CJXL_PATH: Option<&str> = std::option_env!("CFG_CJXL_PATH");

#[derive(Debug)]
#[derive(clap::Args)]
#[group(id = "JxlTranscoder")]
pub struct Jxl;

impl Transcoder for Jxl {
    fn id(&self) -> &'static str {
        "jxl"
    }

    #[inline]
    fn input_formats(&self) -> &'static [ImageFormat] {
        &[ImageFormat::PNG, ImageFormat::JPG, ImageFormat::GIF]
    }

    #[inline]
    fn output_format(&self) -> ImageFormat {
        ImageFormat::JXL
    }

    fn default_jobs(&self) -> std::num::NonZeroU64 {
        #[expect(clippy::unwrap_used)]
        NonZeroU64::new(1).unwrap()
    }

    /// JPEG XL has a superior lossless encoding algorithm which also
    /// doesn't need too much tweaking. These options are used for squashing
    /// out more savings on spaces.
    #[tracing::instrument(name = "jxl_transcode")]
    fn transcode(&self, input: &Path, output: &Path) -> Command {
        let mut cjxl = Command::new(CJXL_PATH.unwrap_or("cjxl"));

        // Allow tweaking more parameters.
        cjxl.arg("--allow_expert_options");
        // Increase the encoding time A LOT
        // (30s in e9 comparing to few seconds
        // in default) but also saves a lot more spaces.
        cjxl.args(["--effort", "8"]);
        // Following 3 options force cjxl to the lossless algorithm
        // called modular, loosely speaking.
        cjxl.args(["--modular", "1"]);
        // Premultiply alpha
        cjxl.args(["--premultiply", "1"]);
        // Controls the generation of some internal tree thing.
        // The bigger the memory it uses, but also save more spaces.
        cjxl.args(["--iterations", "100"]);
        // Tweak the modular algorithm to save even more spaces.
        cjxl.args(["--modular_nb_prev_channels", "6"]);
        cjxl.args(["--modular_group_size", "2"]);
        cjxl.args(["--modular_predictor", "13"]);
        // Use all threads
        cjxl.args(["--num_threads", "-1"]);

        cjxl.args([input, output]);
        cjxl
    }
}
