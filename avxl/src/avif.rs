use anyhow::Context;
use anyhow::Result as AnyResult;
use std::process::ExitStatus;
use tap::Pipe;

use crate::PictureFormat;
use crate::Task;

/// Path to the "avifenc" executable.
const AVIFENC_PATH: Option<&str> = std::option_env!("CFG_AVIFENC_PATH");

#[derive(Debug, Clone, clap::Args)]
#[group(id = "AvifTranscoder")]
pub struct Avif {
    /// Apply a preset when transcoding. Has no effect on "--no-cq"
    /// is supplied.
    #[arg(long, short = 'p')]
    #[arg(default_value_t=Avif::default().quality_preset)]
    pub quality_preset: QualityPreset,

    /// Opt-out of constant quality mode. May result in worse visual quality.
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
            cq_level: 24,
            quality_preset: QualityPreset::Medium,
        }
    }
}

#[derive(Debug, Clone, clap::ValueEnum)]
pub enum QualityPreset {
    Low,
    Medium,
    High,
}

impl std::fmt::Display for QualityPreset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let r = match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        };
        f.write_str(r)
    }
}

impl crate::Transcoder for Avif {
    #[inline]
    fn id(&self) -> &'static str {
        "avif"
    }

    #[inline]
    fn input(&self) -> &'static [crate::PictureFormat] {
        &[PictureFormat::JPG, PictureFormat::PNG]
    }

    #[inline]
    fn output(&self) -> PictureFormat {
        PictureFormat::AVIF
    }

    #[tracing::instrument(name = "avif_transcode")]
    fn transcode(&self, task: Task) -> AnyResult<ExitStatus> {
        let mut avifenc = AVIFENC_PATH
            .unwrap_or("avifenc")
            .pipe(std::process::Command::new);

        let avifenc = {
            let quality = match self.quality_preset {
                QualityPreset::Low => "27",
                QualityPreset::Medium => "47",
                QualityPreset::High => "77",
            };
            avifenc.args(["--qcolor", quality, "--qalpha", quality])
        };

        let avifenc = avifenc
            // All following arguments are tuned for AOM encoder
            .args(["--codec", "aom"])
            // Let it use all cores.
            .args(["--jobs", "all"])
            // Effects the size of output.
            // However, speed < 3 increases the encoding time
            // considerably and has no almost no gain.
            .args(["--speed", "4"])
            // AVIF can save extra, and normally a lot, spaces
            // at higher bit depth.
            .args(["--depth", "12"])
            .arg("--premultiply")
            .arg("--autotiling")
            // Better RGB-YUV processing
            .arg("--sharpyuv")
            .args(["--yuv", "420"])
            .args(["--cicp", "1/13/1"])
            .arg("--ignore-icc")
            .arg("--ignore-exif")
            // Advanced options.
            // This poke into the heart of AOM encoder,
            // which effects the output every so slightly.
            .args(["-a", "color:sharpness=2"])
            .args(["-a", "color:deltaq-mode=3"])
            .args(["-a", "color:enable-chroma-deltaq=1"])
            .args(["-a", "end-usage=q"])
            .args(["-a", "enable-qm=1"])
            .args(["-a", "color:qm-min=0"])
            .args(["-a", "color:enable-dnl-denoising=0"])
            .args(["-a", "color:denoise-noise-level=10"])
            .args(["-a", "tune=ssim"]);

        // If "no_cq" is *not* set, then cq is needed.
        if !self.no_cq {
            let cq_level = format!("cq-level={}", self.cq_level);
            avifenc.args(["-a", &cq_level]);
        }

        let status = avifenc
            .arg("--")
            .args([task.src, task.dst])
            .spawn()
            .context("Failed to spawn avifenc")?
            .wait()?;

        Ok(status)
    }
}
