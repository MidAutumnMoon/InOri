use crate::ImageFormat;
use crate::Transcation;
use crate::Transcoder;

use std::num::NonZeroUsize;
use std::process::Command;

#[derive(Debug)]
#[derive(Clone)]
#[derive(clap::Args)]
#[group(id = "AvifTranscoderOpts")]
pub struct Avif {
    /// Opt-out of constant quality mode.
    /// Will result in worse visual quality but save extra spaces.
    #[arg(long, short)]
    #[arg(default_value_t=Avif::default().no_cq)]
    pub no_cq: bool,

    /// Custom constant quality value. Has no effect if "--no-cq"
    /// is supplied.
    #[arg(long, short)]
    #[arg(default_value_t=Avif::default().cq_level)]
    pub cq_level: u8,
}

impl Default for Avif {
    fn default() -> Self {
        Self {
            no_cq: false,
            cq_level: 22,
        }
    }
}

impl Transcoder for Avif {
    fn default_jobs(&self) -> NonZeroUsize {
        todo!()
    }

    fn input_formats(&self) -> &'static [ImageFormat] {
        todo!()
    }

    fn output_formats(&self) -> &'static [ImageFormat] {
        todo!()
    }

    fn transcode_command(&self, transcation: Transcation) -> Command {
        todo!()
    }
}
