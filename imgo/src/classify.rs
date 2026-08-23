use std::fmt;

use anyhow::Context as _;

use crate::img::Image;

const MAX_LOCAL_SAMPLES: usize = 2_000_000;
const TILE_COUNT: usize = 8;
const TILE_TOTAL: usize = TILE_COUNT * TILE_COUNT;

const COLOR_CHANNEL_DELTA: u8 = 8;
const IMAGE_COLOR_PERCENT: f64 = 1.0;
const NEAR_BLACK_MAX: u8 = 8;
const NEAR_WHITE_MIN: u8 = 247;
const DETAIL_LAPLACIAN_THRESHOLD: u32 = 24;
const SOFT_NOISE_LAPLACIAN_MIN: u32 = 4;
const SOFT_NOISE_LAPLACIAN_MAX: u32 = 20;
const SOFT_NOISE_GRADIENT_MAX: u16 = 24;
const COLOR_TEXTURE_GLOBAL_PERCENT: f64 = 20.0;
const COLOR_TEXTURE_TILE_PERCENT: f64 = 35.0;
const GRAY_TEXTURE_GLOBAL_PERCENT: f64 = 8.0;
const GRAY_TEXTURE_TILE_PERCENT: f64 = 20.0;
const SMALL_EDGE_LIMIT: u32 = 1800;
const MEDIUM_EDGE_LIMIT: u32 = 3000;

#[derive(Debug, Clone, Copy)]
pub struct ImageFeatures {
    pub width: u32,
    pub height: u32,
    pub color_percent: f64,
    pub exact_bilevel: bool,
    pub gray_entropy: f64,
    pub gray_levels: u16,
    pub near_bw_percent: f64,
    pub detail_percent: f64,
    pub soft_noise_percent: f64,
    pub max_tile_soft_noise_percent: f64,
}

impl ImageFeatures {
    #[must_use]
    pub const fn longest_edge(self) -> u32 {
        if self.width > self.height {
            self.width
        } else {
            self.height
        }
    }
}

#[derive(Debug, Clone)]
pub struct AnalyzedImage {
    pub image: Image,
    pub features: ImageFeatures,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PaletteClass {
    Bilevel,
    Gray,
    Color,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TextureClass {
    Quiet,
    Textured,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ScaleClass {
    Small,
    Medium,
    Large,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct GroupKey {
    pub palette: PaletteClass,
    pub texture: TextureClass,
    pub scale: ScaleClass,
}

impl GroupKey {
    #[must_use]
    pub fn from_features(features: ImageFeatures) -> Self {
        let palette = if features.color_percent >= IMAGE_COLOR_PERCENT {
            PaletteClass::Color
        } else if features.exact_bilevel {
            PaletteClass::Bilevel
        } else {
            PaletteClass::Gray
        };
        let texture = match palette {
            PaletteClass::Color => {
                if features.soft_noise_percent
                    >= COLOR_TEXTURE_GLOBAL_PERCENT
                    || features.max_tile_soft_noise_percent
                        >= COLOR_TEXTURE_TILE_PERCENT
                {
                    TextureClass::Textured
                } else {
                    TextureClass::Quiet
                }
            }
            PaletteClass::Gray => {
                if features.soft_noise_percent
                    >= GRAY_TEXTURE_GLOBAL_PERCENT
                    || features.max_tile_soft_noise_percent
                        >= GRAY_TEXTURE_TILE_PERCENT
                {
                    TextureClass::Textured
                } else {
                    TextureClass::Quiet
                }
            }
            PaletteClass::Bilevel => TextureClass::Quiet,
        };
        let scale = match features.longest_edge() {
            0..SMALL_EDGE_LIMIT => ScaleClass::Small,
            SMALL_EDGE_LIMIT..MEDIUM_EDGE_LIMIT => ScaleClass::Medium,
            _ => ScaleClass::Large,
        };
        Self {
            palette,
            texture,
            scale,
        }
    }

    #[must_use]
    pub fn id(self) -> String {
        self.to_string()
    }
}

impl fmt::Display for GroupKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let palette = match self.palette {
            PaletteClass::Bilevel => "bilevel",
            PaletteClass::Gray => "gray",
            PaletteClass::Color => "color",
        };
        let texture = match self.texture {
            TextureClass::Quiet => "quiet",
            TextureClass::Textured => "textured",
        };
        let scale = match self.scale {
            ScaleClass::Small => "small",
            ScaleClass::Medium => "medium",
            ScaleClass::Large => "large",
        };
        write!(f, "{palette}-{texture}-{scale}")
    }
}

/// Decode one source image and measure classification features.
///
/// # Errors
///
/// Returns an error when the image cannot be opened or decoded.
pub fn analyze(image: Image) -> anyhow::Result<AnalyzedImage> {
    let decoded = image::open(&image.path)
        .with_context(|| format!("decode {}", image.path.display()))?;
    let pixels = decoded.into_rgba8();
    let features = analyze_rgba(&pixels);
    Ok(AnalyzedImage { image, features })
}

#[expect(
    clippy::indexing_slicing,
    reason = "RGBA chunks and coordinates are derived from checked image dimensions"
)]
fn analyze_rgba(pixels: &image::RgbaImage) -> ImageFeatures {
    let width = pixels.width() as usize;
    let height = pixels.height() as usize;
    let raw = pixels.as_raw();

    // Palette and color are exact image-wide facts. Sampling these allowed a
    // rare chromatic or midtone pixel to turn a non-bilevel page into a
    // supposedly exact bilevel page.
    let mut histogram = [0_u64; 256];
    let mut total_pixels = 0_u64;
    let mut colorful_pixels = 0_u64;
    let mut near_bw_pixels = 0_u64;
    let (rgba_pixels, remainder) = raw.as_chunks::<4>();
    debug_assert!(
        remainder.is_empty(),
        "RgbaImage storage must contain complete four-byte pixels"
    );
    for &[red, green, blue, alpha] in rgba_pixels {
        let (red, green, blue) = composite_rgb(red, green, blue, alpha);
        let maximum = red.max(green).max(blue);
        let minimum = red.min(green).min(blue);
        if maximum - minimum > COLOR_CHANNEL_DELTA {
            colorful_pixels += 1;
        }
        let luminance = luma(red, green, blue);
        histogram[usize::from(luminance)] += 1;
        if luminance <= NEAR_BLACK_MAX || luminance >= NEAR_WHITE_MIN {
            near_bw_pixels += 1;
        }
        total_pixels += 1;
    }

    // Local edge statistics are sampled because they are approximate routing
    // signals. The stride calculation enforces the accumulator bound.
    let stride = local_sample_stride(width, height);
    let mut local_samples = 0_u32;
    let mut detail = 0_u32;
    let mut soft_noise = 0_u32;
    let mut tile_soft = [0_u32; TILE_TOTAL];
    let mut tile_samples = [0_u32; TILE_TOTAL];
    for y in (0..height).step_by(stride) {
        for x in (0..width).step_by(stride) {
            if x == 0 || y == 0 || x + 1 >= width || y + 1 >= height {
                continue;
            }
            let center = luma_at(raw, width, x, y);
            let left = luma_at(raw, width, x - 1, y);
            let right = luma_at(raw, width, x + 1, y);
            let up = luma_at(raw, width, x, y - 1);
            let down = luma_at(raw, width, x, y + 1);
            let gradient = u16::midpoint(
                u16::from(right.abs_diff(left)),
                u16::from(down.abs_diff(up)),
            );
            let laplacian = (i32::from(center) * 4
                - i32::from(left)
                - i32::from(right)
                - i32::from(up)
                - i32::from(down))
            .unsigned_abs();
            if laplacian > DETAIL_LAPLACIAN_THRESHOLD {
                detail += 1;
            }
            let is_soft_noise = (SOFT_NOISE_LAPLACIAN_MIN
                ..=SOFT_NOISE_LAPLACIAN_MAX)
                .contains(&laplacian)
                && gradient <= SOFT_NOISE_GRADIENT_MAX;
            if is_soft_noise {
                soft_noise += 1;
            }
            local_samples += 1;

            let tile_x = x
                .saturating_mul(TILE_COUNT)
                .checked_div(width)
                .unwrap_or_default()
                .min(TILE_COUNT - 1);
            let tile_y = y
                .saturating_mul(TILE_COUNT)
                .checked_div(height)
                .unwrap_or_default()
                .min(TILE_COUNT - 1);
            let tile = tile_y * TILE_COUNT + tile_x;
            tile_samples[tile] += 1;
            if is_soft_noise {
                tile_soft[tile] += 1;
            }
        }
    }

    let total_f64 = count_as_f64(total_pixels.max(1));
    let local_f64 = f64::from(local_samples.max(1));
    let mut entropy = 0.0;
    let mut levels = 0_u16;
    for count in histogram {
        if count == 0 {
            continue;
        }
        levels += 1;
        let probability = count_as_f64(count) / total_f64;
        entropy -= probability * probability.log2();
    }
    let exact_bilevel = total_pixels > 0
        && colorful_pixels == 0
        && histogram
            .iter()
            .enumerate()
            .all(|(level, count)| *count == 0 || matches!(level, 0 | 255));
    let max_tile_soft_noise_percent = tile_soft
        .iter()
        .zip(tile_samples)
        .filter(|(_, count)| *count > 0)
        .map(|(noise, count)| f64::from(*noise) * 100.0 / f64::from(count))
        .fold(0.0_f64, f64::max);

    ImageFeatures {
        width: pixels.width(),
        height: pixels.height(),
        color_percent: count_as_f64(colorful_pixels) * 100.0 / total_f64,
        exact_bilevel,
        gray_entropy: entropy,
        gray_levels: levels,
        near_bw_percent: count_as_f64(near_bw_pixels) * 100.0 / total_f64,
        detail_percent: f64::from(detail) * 100.0 / local_f64,
        soft_noise_percent: f64::from(soft_noise) * 100.0 / local_f64,
        max_tile_soft_noise_percent,
    }
}

fn local_sample_stride(width: usize, height: usize) -> usize {
    let pixel_count = width.saturating_mul(height);
    let sampling_ratio = pixel_count.div_ceil(MAX_LOCAL_SAMPLES).max(1);
    let floor_stride = sampling_ratio.isqrt();
    let mut stride = if floor_stride * floor_stride < sampling_ratio {
        floor_stride + 1
    } else {
        floor_stride
    };
    while local_sample_count(width, height, stride) > MAX_LOCAL_SAMPLES {
        stride = stride.saturating_add(1);
    }
    stride
}

fn local_sample_count(
    width: usize,
    height: usize,
    stride: usize,
) -> usize {
    width
        .div_ceil(stride)
        .saturating_mul(height.div_ceil(stride))
}

#[expect(
    clippy::cast_precision_loss,
    reason = "image population percentages do not require integer precision above f64's exact range"
)]
fn count_as_f64(count: u64) -> f64 {
    count as f64
}

fn composite_rgb(red: u8, green: u8, blue: u8, alpha: u8) -> (u8, u8, u8) {
    if alpha == 255 {
        return (red, green, blue);
    }
    let composite = |channel: u8| {
        let foreground = u16::from(channel) * u16::from(alpha);
        let background = 255_u16 * u16::from(255 - alpha);
        let value = (foreground + background + 127)
            .checked_div(255)
            .unwrap_or_default();
        u8::try_from(value).unwrap_or(255)
    };
    (composite(red), composite(green), composite(blue))
}

#[inline]
#[expect(
    clippy::indexing_slicing,
    reason = "caller supplies coordinates inside the image buffer"
)]
fn rgb_at(raw: &[u8], width: usize, x: usize, y: usize) -> (u8, u8, u8) {
    let index = (y * width + x) * 4;
    composite_rgb(
        raw[index],
        raw[index + 1],
        raw[index + 2],
        raw[index + 3],
    )
}

#[inline]
fn luma(red: u8, green: u8, blue: u8) -> u8 {
    let weighted = u16::from(red) * 54
        + u16::from(green) * 183
        + u16::from(blue) * 19
        + 128;
    u8::try_from(weighted >> 8).unwrap_or(255)
}

#[inline]
fn luma_at(raw: &[u8], width: usize, x: usize, y: usize) -> u8 {
    let (red, green, blue) = rgb_at(raw, width, x, y);
    luma(red, green, blue)
}

#[cfg(test)]
mod tests {
    use image::Rgba;
    use image::RgbaImage;

    use super::*;

    #[test]
    fn exact_bilevel_is_distinct_from_grayscale() {
        let bilevel = RgbaImage::from_fn(12, 12, |x, _| {
            if x % 2 == 0 {
                Rgba([0, 0, 0, 255])
            } else {
                Rgba([255, 255, 255, 255])
            }
        });
        let bilevel_features = analyze_rgba(&bilevel);
        assert_eq!(bilevel_features.gray_levels, 2);
        assert_eq!(
            GroupKey::from_features(bilevel_features).palette,
            PaletteClass::Bilevel
        );

        let grayscale = RgbaImage::from_fn(12, 12, |x, _| {
            let value = match x % 3 {
                0 => 0,
                1 => 127,
                _ => 255,
            };
            Rgba([value, value, value, 255])
        });
        assert_eq!(
            GroupKey::from_features(analyze_rgba(&grayscale)).palette,
            PaletteClass::Gray
        );
    }

    #[test]
    fn opaque_chroma_is_classified_as_color() {
        let color =
            RgbaImage::from_pixel(12, 12, Rgba([220, 40, 80, 255]));
        let key = GroupKey::from_features(analyze_rgba(&color));
        assert_eq!(key.palette, PaletteClass::Color);
        assert_eq!(key.texture, TextureClass::Quiet);
        assert_eq!(key.scale, ScaleClass::Small);
    }

    #[test]
    fn exact_bilevel_scans_every_decoded_pixel() {
        // The feature sampler uses a stride of two at this size. The lone
        // midtone deliberately sits between sampled coordinates.
        let mut image =
            RgbaImage::from_pixel(2001, 1001, Rgba([255, 255, 255, 255]));
        image.put_pixel(0, 0, Rgba([0, 0, 0, 255]));
        image.put_pixel(1, 1, Rgba([127, 127, 127, 255]));

        let features = analyze_rgba(&image);
        assert_eq!(features.gray_levels, 3);
        assert_eq!(
            GroupKey::from_features(features).palette,
            PaletteClass::Gray
        );
    }

    #[test]
    fn near_black_chroma_is_not_bilevel() {
        let image = RgbaImage::from_fn(12, 12, |x, _| {
            if x % 2 == 0 {
                Rgba([0, 0, 30, 255])
            } else {
                Rgba([255, 255, 255, 255])
            }
        });

        assert_eq!(
            GroupKey::from_features(analyze_rgba(&image)).palette,
            PaletteClass::Color
        );
    }

    #[test]
    fn local_sampling_bound_holds_for_extreme_aspect_ratios() {
        for (width, height) in [
            (4299, 1325),
            (1, 10_000_000),
            (10_000_000, 1),
            (100_000, 100_000),
        ] {
            let stride = local_sample_stride(width, height);
            assert!(
                local_sample_count(width, height, stride)
                    <= MAX_LOCAL_SAMPLES
            );
        }
    }
}
