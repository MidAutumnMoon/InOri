use anyhow::Context;
use anyhow::Result as AnyResult;
use itertools::Itertools;
use std::num::NonZeroUsize;
use std::process::ExitStatus;
use tap::Pipe;

use crate::PictureFormat;
use crate::Task;

/// Path to the "avifenc" executable.
const MAGICK_PATH: Option<&str> = std::option_env!("CFG_MAGICK_PATH");

#[derive(Debug, clap::Args)]
#[group(id = "DespeckleTranscoder")]
pub struct Despeckle {
    /// How many despeckle passes to run on the picture
    #[arg(long, short)]
    #[ arg( default_value_t=Self::default().iteration ) ]
    pub iteration: NonZeroUsize,
}

impl Default for Despeckle {
    fn default() -> Self {
        #[allow(clippy::unwrap_used)]
        Self {
            iteration: NonZeroUsize::new(1).unwrap(),
        }
    }
}

impl crate::Transcoder for Despeckle {
    fn id(&self) -> &'static str {
        "despeckle"
    }

    #[inline]
    fn input(&self) -> &'static [PictureFormat] {
        &[PictureFormat::PNG, PictureFormat::JPG]
    }

    #[inline]
    fn output(&self) -> PictureFormat {
        PictureFormat::PNG
    }

    #[tracing::instrument]
    fn transcode(&self, task: Task) -> AnyResult<ExitStatus> {
        let number_of_depseckles =
            std::iter::repeat_n("-despeckle", self.iteration.into())
                .collect_vec();

        let mut magick = MAGICK_PATH
            .unwrap_or("magick")
            .pipe(std::process::Command::new);

        let status = magick
            .arg("-verbose")
            .arg("--")
            .arg(&task.src)
            .args(number_of_depseckles)
            .arg(&task.dst)
            .spawn()
            .context("Failed to run magick")?
            .wait()?;

        Ok(status)
    }
}
