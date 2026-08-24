use std::collections::BTreeSet;
use std::num::NonZeroU64;
use std::path::Path;
use std::thread::available_parallelism;

use anyhow::Context as _;
use anyhow::ensure;
use serde::Deserialize;
use serde::Serialize;

use crate::img::ImageFormat;
use crate::transcoder::Meta;
use crate::transcoder::Operation;
use crate::transcoder::Tool;
use crate::transcoder::run_command;

const DEFAULT_CLEAN_SCAN_THRESHOLD: u8 = 55;
const SAND_TONE_SIGMA_DIVISOR: f64 = 1400.0;
const SAND_TONE_MIN_BASE_SIGMA: f64 = 0.8;
const SAND_TONE_MAX_BASE_SIGMA: f64 = 2.8;
const SAND_TONE_LINE_THRESHOLD: &str = "20%";

/// `ImageMagick` preprocessing modes. Each destructive transformation is an
/// explicit recipe step; classifier policy chooses whether that recipe is a
/// default or a review-only candidate.
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
#[group(skip)]
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
    fn validate(&self) -> anyhow::Result<()> {
        ensure!(
            self.strength
                .as_deref()
                .is_none_or(|strength| !strength.trim().is_empty()),
            "denoise strength cannot be empty"
        );
        ensure!(
            self.mode != Mode::Despeckle || self.strength.is_none(),
            "despeckle does not accept --strength"
        );
        Ok(())
    }

    fn run(&self, input: &Path, output: &Path) -> anyhow::Result<()> {
        self.validate()?;
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
        run_command(self.id(), input, &mut command)
    }

    fn required_tools(&self, tools: &mut BTreeSet<Tool>) {
        tools.insert(Tool::Magick);
    }
}

/// How aggressively disposable manga sand tone is flattened.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SandToneStrength {
    Light,
    #[default]
    Medium,
    Heavy,
}

/// Low-pass and quantize sand tone, then restore only solid dark line work.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[derive(Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FlattenSandTone {
    pub strength: SandToneStrength,
}

struct SandTonePreset {
    sigma: f64,
    gray_levels: &'static str,
    line_opening: &'static str,
}

impl FlattenSandTone {
    fn preset(self, longest_edge: u32) -> SandTonePreset {
        let base_sigma = (f64::from(longest_edge)
            / SAND_TONE_SIGMA_DIVISOR)
            .clamp(SAND_TONE_MIN_BASE_SIGMA, SAND_TONE_MAX_BASE_SIGMA);
        let (multiplier, gray_levels) = match self.strength {
            SandToneStrength::Light => (0.6, "12"),
            SandToneStrength::Medium => (0.9, "8"),
            SandToneStrength::Heavy => (1.1, "6"),
        };
        let line_opening = match longest_edge {
            0..1800 => "Disk:1",
            1800..3500 => "Disk:1.5",
            _ => "Disk:2",
        };
        SandTonePreset {
            sigma: base_sigma * multiplier,
            gray_levels,
            line_opening,
        }
    }
}

impl Meta for FlattenSandTone {
    fn id(&self) -> &'static str {
        "magick flatten-sand-tone"
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

impl Operation for FlattenSandTone {
    fn validate(&self) -> anyhow::Result<()> {
        Ok(())
    }

    fn run(&self, input: &Path, output: &Path) -> anyhow::Result<()> {
        let (width, height) = image::image_dimensions(input)
            .with_context(|| {
                format!("read dimensions of {}", input.display())
            })?;
        let preset = self.preset(width.max(height));
        let blur = format!("0x{:.2}", preset.sigma);

        let mut command = Tool::Magick.command();
        command.arg(input);
        command.args([
            "-background",
            "white",
            "-alpha",
            "remove",
            "-alpha",
            "off",
            "-colorspace",
            "Gray",
            "-write",
            "mpr:source",
            "+delete",
        ]);
        command.args([
            "mpr:source",
            "-blur",
            &blur,
            "+dither",
            "-posterize",
            preset.gray_levels,
            "-write",
            "mpr:base",
            "+delete",
        ]);
        command.args([
            "mpr:source",
            "-threshold",
            SAND_TONE_LINE_THRESHOLD,
            "-negate",
            "-morphology",
            "Open",
            preset.line_opening,
            "-negate",
            "-write",
            "mpr:lines",
            "+delete",
        ]);
        command.args([
            "mpr:base",
            "mpr:lines",
            "-compose",
            "Darken",
            "-composite",
            "-strip",
            "-define",
            "png:compression-level=1",
        ]);
        command.arg(output);
        run_command(self.id(), input, &mut command)
    }

    fn required_tools(&self, tools: &mut BTreeSet<Tool>) {
        tools.insert(Tool::Magick);
    }
}

/// Convert degraded near-bilevel pages to crisp one-bit grayscale.
#[derive(Debug, Clone, PartialEq, Eq)]
#[derive(clap::Args, Serialize, Deserialize)]
#[group(skip)]
#[serde(default, deny_unknown_fields)]
pub struct CleanScan {
    /// Fixed global threshold percentage. Cannot be supplied with `--otsu`.
    #[arg(
        long,
        default_value_t = Self::default().threshold,
        conflicts_with = "otsu"
    )]
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
            threshold: DEFAULT_CLEAN_SCAN_THRESHOLD,
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
    fn validate(&self) -> anyhow::Result<()> {
        ensure!(
            self.threshold <= 100,
            "clean-scan threshold must be in 0..=100"
        );
        ensure!(
            !self.otsu || self.threshold == DEFAULT_CLEAN_SCAN_THRESHOLD,
            "--threshold cannot be customized together with --otsu"
        );
        Ok(())
    }

    fn run(&self, input: &Path, output: &Path) -> anyhow::Result<()> {
        self.validate()?;

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
        run_command(self.id(), input, &mut command)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sand_tone_preset_scales_with_resolution_and_strength() {
        let small = FlattenSandTone {
            strength: SandToneStrength::Heavy,
        }
        .preset(749);
        assert!((small.sigma - 0.88).abs() < f64::EPSILON);
        assert_eq!(small.gray_levels, "6");
        assert_eq!(small.line_opening, "Disk:1");

        let standard = FlattenSandTone::default().preset(2266);
        assert!((standard.sigma - 1.456_714_285_714_285_6).abs() < 1e-12);
        assert_eq!(standard.gray_levels, "8");
        assert_eq!(standard.line_opening, "Disk:1.5");

        let high = FlattenSandTone {
            strength: SandToneStrength::Light,
        }
        .preset(4503);
        assert!((high.sigma - 1.68).abs() < f64::EPSILON);
        assert_eq!(high.gray_levels, "12");
        assert_eq!(high.line_opening, "Disk:2");
    }
}
