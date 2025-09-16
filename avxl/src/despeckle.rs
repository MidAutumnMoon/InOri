use anyhow::Result as AnyResult;
use itertools::Itertools;
use pty_process::blocking::Command;
use std::num::NonZeroUsize;
use std::path::Path;
use tap::Pipe;

use crate::PictureFormat;

/// Path to the "avifenc" executable.
pub const MAGICK_PATH: Option<&str> = std::option_env!("CFG_MAGICK_PATH");

#[derive(Debug, clap::Args)]
#[group(id = "DespeckleTranscoder")]
pub struct Despeckle {
    /// How many despeckle passes to run on the picture
    #[arg(long, short)]
    #[arg(default_value = "1")]
    pub iteration: NonZeroUsize,
}

impl crate::Transcoder for Despeckle {
    #[inline]
    fn id(&self) -> &'static str {
        "despeckle"
    }

    #[inline]
    fn input_format(&self) -> &'static [PictureFormat] {
        &[PictureFormat::PNG, PictureFormat::JPG]
    }

    #[inline]
    fn output_format(&self) -> PictureFormat {
        PictureFormat::PNG
    }

    #[tracing::instrument]
    fn generate_command(&self, input: &Path, output: &Path) -> Command {
        let number_of_depseckles =
            std::iter::repeat_n("-despeckle", self.iteration.into())
                .collect_vec();
        MAGICK_PATH
            .unwrap_or("magick")
            .pipe(Command::new)
            .arg("-verbose")
            .arg("--")
            .arg(input)
            .args(number_of_depseckles)
            .arg(output)
    }
}
