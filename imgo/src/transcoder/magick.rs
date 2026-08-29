use std::collections::BTreeSet;
use std::fmt;
use std::num::NonZeroU64;
use std::path::Path;
use std::str::FromStr;
use std::thread::available_parallelism;

use bpaf::Parser;
use bpaf::construct;
use bpaf::long;
use rootcause::bail;
use rootcause::prelude::ResultExt as _;
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
const SAND_TONE_DETAIL_SIGMA_DIVISOR: f64 = 4000.0;
const SAND_TONE_MIN_DETAIL_SIGMA: f64 = 0.8;
const SAND_TONE_MAX_DETAIL_SIGMA: f64 = 1.5;
const SAND_TONE_REGION_SIGMA_DIVISOR: f64 = 500.0;
const SAND_TONE_MIN_REGION_SIGMA: f64 = 4.0;
const SAND_TONE_MAX_REGION_SIGMA: f64 = 12.0;
const SAND_TONE_COMPONENT_AREA_DIVISOR: u64 = 2000;

/// `ImageMagick` preprocessing modes. Each destructive transformation is an
/// explicit recipe step; classifier policy chooses whether that recipe is a
/// default or a review-only candidate.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[derive(Serialize, Deserialize)]
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

impl Mode {
    /// Canonical CLI and serde name, shared by `Display` and `FromStr`.
    #[must_use]
    const fn as_str(self) -> &'static str {
        match self {
            Self::Artifact => "artifact",
            Self::AdaptiveBlur => "adaptive-blur",
            Self::FakePencil => "fake-pencil",
            Self::Despeckle => "despeckle",
        }
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Mode {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "artifact" => Ok(Self::Artifact),
            "adaptive-blur" => Ok(Self::AdaptiveBlur),
            "fake-pencil" => Ok(Self::FakePencil),
            "despeckle" => Ok(Self::Despeckle),
            other => Err(format!(
                "expected one of `artifact`, `adaptive-blur`, `fake-pencil`, \
                 `despeckle`, got `{other}`"
            )),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[derive(Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Denoise {
    pub mode: Mode,

    /// `ImageMagick` geometry or percentage used by the chosen mode.
    pub strength: Option<String>,
}

/// CLI parser for [`Denoise`].
#[must_use]
pub fn denoise_cli() -> impl Parser<Denoise> {
    let mode = long("mode")
        .short('m')
        .argument::<Mode>("MODE")
        .help(
            "Preprocessing mode: `artifact` (bilateral blur), \
             `adaptive-blur`, `fake-pencil` (median plus contrast stretch) \
             or `despeckle`",
        )
        .fallback(Denoise::default().mode)
        .display_fallback();
    let strength = long("strength")
        .short('s')
        .argument::<String>("GEOMETRY")
        .help(
            "`ImageMagick` geometry or percentage used by the chosen mode",
        )
        .optional();
    construct!(Denoise { mode, strength })
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
    fn validate(&self) -> rootcause::Result<()> {
        if self
            .strength
            .as_deref()
            .is_some_and(|strength| strength.trim().is_empty())
        {
            bail!("denoise strength cannot be empty");
        }
        if self.mode == Mode::Despeckle && self.strength.is_some() {
            bail!("despeckle does not accept --strength");
        }
        Ok(())
    }

    fn run(&self, input: &Path, output: &Path) -> rootcause::Result<()> {
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

/// Flatten detected sand-tone regions while preserving smooth content.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[derive(Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FlattenSandTone {
    pub strength: SandToneStrength,
}

struct SandTonePreset {
    flatten_sigma: f64,
    gray_levels: &'static str,
    line_opening: &'static str,
    detail_sigma: f64,
    region_sigma: f64,
    mask_opening: u32,
    mask_closing: u32,
    mask_feather_sigma: f64,
    mask_threshold: &'static str,
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
        let mask_threshold = match longest_edge {
            0..1800 => "6%",
            1800..3500 => "8%",
            _ => "10%",
        };
        let detail_sigma = (f64::from(longest_edge)
            / SAND_TONE_DETAIL_SIGMA_DIVISOR)
            .clamp(SAND_TONE_MIN_DETAIL_SIGMA, SAND_TONE_MAX_DETAIL_SIGMA);
        let region_sigma = (f64::from(longest_edge)
            / SAND_TONE_REGION_SIGMA_DIVISOR)
            .clamp(SAND_TONE_MIN_REGION_SIGMA, SAND_TONE_MAX_REGION_SIGMA);
        let mask_closing = longest_edge
            .saturating_add(625)
            .checked_div(1250)
            .unwrap_or_default()
            .clamp(2, 5);
        SandTonePreset {
            flatten_sigma: base_sigma * multiplier,
            gray_levels,
            line_opening,
            detail_sigma,
            region_sigma,
            mask_opening: mask_closing.div_ceil(2),
            mask_closing,
            mask_feather_sigma: region_sigma / 4.0,
            mask_threshold,
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
    fn validate(&self) -> rootcause::Result<()> {
        Ok(())
    }

    fn run(&self, input: &Path, output: &Path) -> rootcause::Result<()> {
        let (width, height) = image::image_dimensions(input)
            .context_with(|| {
                format!("read dimensions of {}", input.display())
            })?;
        let preset = self.preset(width.max(height));
        let flatten_blur = format!("0x{:.2}", preset.flatten_sigma);
        let detail_blur = format!("0x{:.2}", preset.detail_sigma);
        let region_blur = format!("0x{:.2}", preset.region_sigma);
        let mask_opening = format!("Disk:{}", preset.mask_opening);
        let mask_closing = format!("Disk:{}", preset.mask_closing);
        let mask_feather = format!("0x{:.2}", preset.mask_feather_sigma);
        let component_area = u64::from(width)
            .saturating_mul(u64::from(height))
            .checked_div(SAND_TONE_COMPONENT_AREA_DIVISOR)
            .unwrap_or_default()
            .max(256);
        let component_area = format!(
            "connected-components:area-threshold={component_area}"
        );

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
            &flatten_blur,
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
            "-write",
            "mpr:flat",
            "+delete",
        ]);
        command.args([
            "mpr:source",
            "-blur",
            &detail_blur,
            "-write",
            "mpr:low",
            "+delete",
        ]);
        command.args([
            "mpr:source",
            "mpr:low",
            "-compose",
            "Difference",
            "-composite",
            "-blur",
            &region_blur,
            "-threshold",
            preset.mask_threshold,
            "-morphology",
            "Open",
            &mask_opening,
            "-morphology",
            "Close",
            &mask_closing,
            "-define",
            &component_area,
            "-define",
            "connected-components:mean-color=true",
            "-connected-components",
            "8",
            "-blur",
            &mask_feather,
            "-write",
            "mpr:mask",
            "+delete",
        ]);
        command.args([
            "mpr:source",
            "mpr:flat",
            "mpr:mask",
            "-compose",
            "Over",
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
#[derive(Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CleanScan {
    /// Fixed global threshold percentage. Cannot be supplied with `--otsu`.
    pub threshold: u8,

    /// Select a global threshold from the image histogram.
    pub otsu: bool,

    /// Disable the mild pre-threshold unsharp mask.
    pub sharpen: bool,
}

/// CLI parser for [`CleanScan`].
#[must_use]
pub fn clean_scan_cli() -> impl Parser<CleanScan> {
    let threshold = long("threshold")
        .argument::<u8>("PERCENT")
        .help(
            "Fixed global threshold percentage. Cannot be supplied with \
             `--otsu`",
        )
        .fallback(CleanScan::default().threshold)
        .display_fallback();
    let otsu = long("otsu")
        .switch()
        .help("Select a global threshold from the image histogram");
    let no_sharpen = long("no-sharpen")
        .switch()
        .help("Disable the mild pre-threshold unsharp mask");
    let sharpen = no_sharpen.map(|no_sharpen| !no_sharpen);
    construct!(CleanScan {
        threshold,
        otsu,
        sharpen,
    })
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
    fn validate(&self) -> rootcause::Result<()> {
        if self.threshold > 100 {
            bail!("clean-scan threshold must be in 0..=100");
        }
        if self.otsu && self.threshold != DEFAULT_CLEAN_SCAN_THRESHOLD {
            bail!("--threshold cannot be customized together with --otsu");
        }
        Ok(())
    }

    fn run(&self, input: &Path, output: &Path) -> rootcause::Result<()> {
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
        assert!((small.flatten_sigma - 0.88).abs() < f64::EPSILON);
        assert_eq!(small.gray_levels, "6");
        assert_eq!(small.line_opening, "Disk:1");
        assert!((small.detail_sigma - 0.8).abs() < f64::EPSILON);
        assert!((small.region_sigma - 4.0).abs() < f64::EPSILON);
        assert_eq!(small.mask_opening, 1);
        assert_eq!(small.mask_closing, 2);
        assert_eq!(small.mask_threshold, "6%");

        let standard = FlattenSandTone::default().preset(2266);
        assert!(
            (standard.flatten_sigma - 1.456_714_285_714_285_6).abs()
                < 1e-12
        );
        assert_eq!(standard.gray_levels, "8");
        assert_eq!(standard.line_opening, "Disk:1.5");
        assert_eq!(standard.mask_threshold, "8%");

        let high = FlattenSandTone {
            strength: SandToneStrength::Light,
        }
        .preset(4503);
        assert!((high.flatten_sigma - 1.68).abs() < f64::EPSILON);
        assert_eq!(high.gray_levels, "12");
        assert_eq!(high.line_opening, "Disk:2");
        assert!((high.detail_sigma - 1.125_75).abs() < f64::EPSILON);
        assert!((high.region_sigma - 9.006).abs() < f64::EPSILON);
        assert_eq!(high.mask_opening, 2);
        assert_eq!(high.mask_closing, 4);
        assert_eq!(high.mask_threshold, "10%");
    }
}
