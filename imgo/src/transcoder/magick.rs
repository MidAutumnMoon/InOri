use std::collections::BTreeSet;
use std::num::NonZeroU64;
use std::path::Path;
use std::thread::available_parallelism;

use anyhow::ensure;
use serde::Deserialize;
use serde::Serialize;

use crate::img::ImageFormat;
use crate::transcoder::Meta;
use crate::transcoder::Operation;
use crate::transcoder::Tool;
use crate::transcoder::run_command;

/// `ImageMagick` preprocessing modes. Destructive modes remain explicit
/// recipe steps: image statistics cannot reliably distinguish intentional
/// texture from disposable noise.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[derive(clap::ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    /// Edge-preserving bilateral filtering. The default geometry is `3x3`.
    #[default]
    Artifact,
    /// Stronger legacy adaptive blur. The default geometry is `2x0.8`.
    AdaptiveBlur,
    /// Median 3x3 followed by contrast stretch. The default stretch is `5%x0%`.
    FakePencil,
    /// `ImageMagick`'s hull-based speckle reduction.
    Despeckle,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[derive(clap::Args, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Denoise {
    #[arg(long, short, value_enum, default_value_t = Self::default().mode)]
    pub mode: Mode,

    /// `ImageMagick` geometry or percentage used by the chosen mode.
    #[arg(long, short)]
    pub strength: Option<String>,
}

impl Meta for Denoise {
    fn id(&self) -> &'static str {
        "magick denoise"
    }

    fn default_jobs(&self) -> NonZeroU64 {
        #[expect(
            clippy::unwrap_used,
            reason = "literal 2 is non-zero by construction"
        )]
        NonZeroU64::new(2).unwrap()
    }

    fn input_formats(&self) -> &'static [ImageFormat] {
        &[ImageFormat::PNG, ImageFormat::JPG, ImageFormat::WEBP]
    }

    fn output_format(&self) -> ImageFormat {
        ImageFormat::PNG
    }
}

impl Operation for Denoise {
    fn run(
        &self,
        input: &Path,
        output: &Path,
    ) -> anyhow::Result<Vec<String>> {
        let mut command = Tool::Magick.command();
        command.arg(input);
        match self.mode {
            Mode::Artifact => {
                command.args([
                    "-bilateral-blur",
                    self.strength.as_deref().unwrap_or("3x3"),
                ]);
            }
            Mode::AdaptiveBlur => {
                command.args([
                    "-adaptive-blur",
                    self.strength.as_deref().unwrap_or("2x0.8"),
                ]);
            }
            Mode::FakePencil => {
                command.args(["-statistic", "median", "3x3"]);
                command.args([
                    "-contrast-stretch",
                    self.strength.as_deref().unwrap_or("5%x0%"),
                ]);
            }
            Mode::Despeckle => {
                command.arg("-despeckle");
            }
        }
        command.args(["-define", "png:compression-level=1"]);
        command.arg(output);
        run_command(self.id(), input, &mut command)?;
        Ok(Vec::new())
    }

    fn required_tools(&self, tools: &mut BTreeSet<Tool>) {
        tools.insert(Tool::Magick);
    }
}

/// Convert degraded near-bilevel pages to crisp one-bit grayscale.
#[derive(Debug, Clone, PartialEq, Eq)]
#[derive(clap::Args, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CleanScan {
    /// Fixed global threshold percentage. Ignored when `--otsu` is set.
    #[arg(long, default_value_t = Self::default().threshold)]
    pub threshold: u8,

    /// Select a global threshold from the image histogram.
    #[arg(long, default_value_t = Self::default().otsu)]
    pub otsu: bool,

    /// Disable the mild pre-threshold unsharp mask.
    #[arg(long = "no-sharpen", action = clap::ArgAction::SetFalse)]
    pub sharpen: bool,
}

impl Default for CleanScan {
    fn default() -> Self {
        Self {
            threshold: 55,
            otsu: false,
            sharpen: true,
        }
    }
}

impl Meta for CleanScan {
    fn id(&self) -> &'static str {
        "magick clean-scan"
    }

    fn default_jobs(&self) -> NonZeroU64 {
        available_cores()
    }

    fn input_formats(&self) -> &'static [ImageFormat] {
        &[ImageFormat::PNG, ImageFormat::JPG]
    }

    fn output_format(&self) -> ImageFormat {
        ImageFormat::PNG
    }
}

impl Operation for CleanScan {
    fn run(
        &self,
        input: &Path,
        output: &Path,
    ) -> anyhow::Result<Vec<String>> {
        ensure!(
            self.threshold <= 100,
            "clean-scan threshold must be in 0..=100"
        );

        let mut command = Tool::Magick.command();
        command.arg(input);
        command.args(["-background", "white", "-alpha", "remove"]);
        command.arg("-alpha").arg("off");
        command.args(["-colorspace", "Gray"]);
        command.arg("-strip");
        if self.sharpen {
            command.args(["-unsharp", "0x2+1+0.4"]);
        }
        if self.otsu {
            command.args(["-auto-threshold", "OTSU"]);
        } else {
            command.args(["-threshold", &format!("{}%", self.threshold)]);
        }
        command.args(["-depth", "1", "-colors", "2"]);
        command.args(["-define", "png:compression-level=1"]);
        command.arg(output);
        run_command(self.id(), input, &mut command)?;
        Ok(Vec::new())
    }

    fn required_tools(&self, tools: &mut BTreeSet<Tool>) {
        tools.insert(Tool::Magick);
    }
}

fn available_cores() -> NonZeroU64 {
    let cores = available_parallelism().map_or(1, usize::from);
    let selected = u64::try_from(cores).unwrap_or(1);
    #[expect(
        clippy::unwrap_used,
        reason = "available parallelism is always at least one"
    )]
    NonZeroU64::new(selected).unwrap()
}
