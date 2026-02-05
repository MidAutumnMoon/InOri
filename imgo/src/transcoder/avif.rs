use tap::Pipe;

use crate::ImageFormat;
use crate::Transcoder;

use std::num::NonZeroU64;
use std::path::Path;
use std::process::Command;

const AVIFENC_PATH: Option<&str> = std::option_env!("CFG_AVIFENC_PATH");

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

    /// Apply a preset when transcoding. Has no effect on "--no-cq"
    /// is supplied.
    #[arg(long, short = 'p')]
    #[arg(default_value_t=Avif::default().quality_preset)]
    pub quality_preset: QualityPreset,
}

impl Default for Avif {
    fn default() -> Self {
        Self {
            no_cq: false,
            cq_level: 22,
            quality_preset: QualityPreset::Medium,
        }
    }
}

#[derive(Debug, Clone, clap::ValueEnum)]
#[derive(strum::Display)]
pub enum QualityPreset {
    #[strum(to_string = "low")]
    Low,
    #[strum(to_string = "medium")]
    Medium,
    #[strum(to_string = "high")]
    High,
}

impl Transcoder for Avif {
    fn id(&self) -> &'static str {
        "avifenc"
    }

    fn default_jobs(&self) -> NonZeroU64 {
        #[expect(clippy::unwrap_used)]
        NonZeroU64::new(1).unwrap()
    }

    fn input_formats(&self) -> &'static [ImageFormat] {
        &[ImageFormat::PNG, ImageFormat::JPG]
    }

    fn output_format(&self) -> ImageFormat {
        ImageFormat::AVIF
    }

    fn transcode(&self, input: &Path, output: &Path) -> Command {
        let mut cmd = AVIFENC_PATH.unwrap_or("avifenc").pipe(Command::new);

        let quality = match self.quality_preset {
            QualityPreset::Low => "28",
            QualityPreset::Medium => "48",
            QualityPreset::High => "78",
        };
        cmd.args(["--qcolor", quality, "--qalpha", quality]);

        // All following arguments are tuned for AOM encoder
        cmd.args(["--codec", "aom"]);
        // Let it use all cores.
        cmd.args(["--jobs", "all"]);
        // Effects the size of output.
        // However, speed < 3 increases the encoding time
        // considerably and has no almost no gain.
        cmd.args(["--speed", "5"]);
        // AVIF can save extra, and normally a lot, spaces
        // at higher bit depth.
        cmd.args(["--depth", "12"]);
        cmd.arg("--premultiply");
        cmd.arg("--autotiling");
        // Better RGB-YUV processing
        cmd.arg("--sharpyuv");
        cmd.args(["--yuv", "420"]);
        cmd.args(["--cicp", "1/13/1"]);
        cmd.arg("--ignore-icc");
        cmd.arg("--ignore-exif");
        // Advanced options.
        // This poke into the heart of AOM encoder,
        // which effects the output every so slightly.
        cmd.args(["-a", "color:deltaq-mode=3"]);
        cmd.args(["-a", "color:enable-chroma-deltaq=1"]);
        cmd.args(["-a", "end-usage=q"]);
        cmd.args(["-a", "enable-qm=1"]);
        cmd.args(["-a", "color:qm-min=0"]);
        cmd.args(["-a", "aq-mode=2"]);
        cmd.args(["-a", "color:denoise-noise-level=20"]);
        cmd.args(["-a", "tune=ssim"]);

        if !self.no_cq {
            let cq_level = format!("cq-level={}", self.cq_level);
            cmd.args(["-a", &cq_level]);
        }

        cmd.arg("--").args([input, output]);
        cmd
    }
}
