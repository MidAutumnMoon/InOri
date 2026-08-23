use std::collections::BTreeSet;
use std::num::NonZeroU64;
use std::path::Path;

use anyhow::ensure;
use serde::Deserialize;
use serde::Serialize;

use crate::img::ImageFormat;
use crate::transcoder::Meta;
use crate::transcoder::Operation;
use crate::transcoder::Tool;
use crate::transcoder::run_command;

/// Chroma sampling requested from `avifenc`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[derive(clap::ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Chroma {
    /// Let libavif retain grayscale and JPEG sampling, and use 4:4:4 for
    /// other PNGs. This is the safe choice for colored line art.
    #[default]
    Auto,
    #[value(name = "444")]
    #[serde(rename = "444")]
    Yuv444,
    #[value(name = "422")]
    #[serde(rename = "422")]
    Yuv422,
    #[value(name = "420")]
    #[serde(rename = "420")]
    Yuv420,
    #[value(name = "400")]
    #[serde(rename = "400")]
    Yuv400,
}

impl Chroma {
    const fn as_arg(self) -> Option<&'static str> {
        match self {
            Self::Auto => None,
            Self::Yuv444 => Some("444"),
            Self::Yuv422 => Some("422"),
            Self::Yuv420 => Some("420"),
            Self::Yuv400 => Some("400"),
        }
    }
}

/// Stable libavif controls used by manual commands and automation recipes.
///
/// Advanced libaom controls are intentionally avoided: the old mix of
/// `qcolor`, quantizer bounds, and `cq-level` exposed two competing quality
/// controls and made content-dependent grain synthesis unconditional.
#[derive(Debug, Clone, PartialEq, Eq)]
#[derive(clap::Args, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Avif {
    /// libavif color quality in 0..=100. At 100 quantization is lossless;
    /// chroma conversion can still be lossy.
    #[arg(long, short = 'q', default_value_t = Self::default().quality)]
    pub quality: u8,

    /// Encoded channel depth: 8, 10, or 12.
    #[arg(long, default_value_t = Self::default().depth)]
    pub depth: u8,

    /// Chroma sampling. `auto` keeps grayscale monochrome and color PNGs 4:4:4.
    #[arg(long, value_enum, default_value_t = Self::default().chroma)]
    pub chroma: Chroma,

    /// Enable libaom's all-intra noise estimation, denoising, and grain
    /// synthesis. Useful for genuinely grainy color art; harmful to crisp
    /// screentone, so it is opt-in.
    #[arg(long, default_value_t = Self::default().grain)]
    pub grain: bool,

    /// Encoder speed in 0..=10. Lower is slower.
    #[arg(long, default_value_t = Self::default().speed)]
    pub speed: u8,
}

impl Default for Avif {
    fn default() -> Self {
        Self {
            quality: 65,
            depth: 10,
            chroma: Chroma::Auto,
            grain: false,
            speed: 5,
        }
    }
}

impl Meta for Avif {
    fn id(&self) -> &'static str {
        "avifenc"
    }

    fn default_jobs(&self) -> NonZeroU64 {
        NonZeroU64::MIN
    }

    fn input_formats(&self) -> &'static [ImageFormat] {
        &[ImageFormat::PNG, ImageFormat::JPG]
    }

    fn output_format(&self) -> ImageFormat {
        ImageFormat::AVIF
    }
}

impl Operation for Avif {
    fn validate(&self) -> anyhow::Result<()> {
        ensure!(self.quality <= 100, "AVIF quality must be in 0..=100");
        ensure!(
            matches!(self.depth, 8 | 10 | 12),
            "AVIF depth must be 8, 10, or 12"
        );
        ensure!(self.speed <= 10, "AVIF speed must be in 0..=10");
        Ok(())
    }

    fn run(&self, input: &Path, output: &Path) -> anyhow::Result<()> {
        self.validate()?;

        let mut command = Tool::AvifEnc.command();
        command.args(["--qcolor", &self.quality.to_string()]);
        command.args(["--qalpha", "100"]);
        command.args(["--codec", "aom"]);
        command.args(["--jobs", "all"]);
        command.args(["--speed", &self.speed.to_string()]);
        command.args(["--depth", &self.depth.to_string()]);
        if let Some(chroma) = self.chroma.as_arg() {
            command.args(["--yuv", chroma]);
            if self.chroma == Chroma::Yuv420 {
                command.arg("--sharpyuv");
            }
        }
        // Strip camera/application metadata but retain the color profile. An
        // ICC profile cannot be discarded safely without first converting it.
        command.arg("--ignore-exif");
        command.arg("--ignore-xmp");
        if self.grain {
            // In libaom all-intra mode, a positive value acts as the switch for
            // automatic source-noise estimation; the numeric magnitude is not
            // honored as a stable user-selected grain level.
            command.args(["-a", "color:denoise-noise-level=1"]);
        }
        command.arg("--").args([input, output]);

        run_command(self.id(), input, &mut command)
    }

    fn required_tools(&self, tools: &mut BTreeSet<Tool>) {
        tools.insert(Tool::AvifEnc);
    }
}
