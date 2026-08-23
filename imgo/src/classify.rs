use std::fmt;

use anyhow::Context as _;

use crate::img::Image;

const MAX_SAMPLED_PIXELS: usize = 2_000_000;
const TILE_COUNT: usize = 8;
const TILE_TOTAL: usize = TILE_COUNT * TILE_COUNT;

#[derive(Debug, Clone, Copy)]
pub struct ImageFeatures {
    pub width: u32,
    pub height: u32,
    pub color_percent: f64,
    pub gray_entropy: f64,
    pub gray_levels: u16,
    pub near_bw_percent: f64,
    pub detail_percent: f64,
    pub soft_noise_percent: f64,
    pub max_tile_soft_noise_percent: f64,
    pub mean_gradient: f64,
    pub mean_laplacian: f64,
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
        let palette = if features.gray_levels <= 2
            && features.near_bw_percent >= 99.9
        {
            PaletteClass::Bilevel
        } else if features.color_percent >= 1.0 {
            PaletteClass::Color
        } else {
            PaletteClass::Gray
        };
        let texture = match palette {
            PaletteClass::Color => {
                if features.soft_noise_percent >= 20.0
                    || features.max_tile_soft_noise_percent >= 35.0
                {
                    TextureClass::Textured
                } else {
                    TextureClass::Quiet
                }
            }
            PaletteClass::Gray => {
                if features.soft_noise_percent >= 8.0
                    || features.max_tile_soft_noise_percent >= 20.0
                {
                    TextureClass::Textured
                } else {
                    TextureClass::Quiet
                }
            }
            PaletteClass::Bilevel => TextureClass::Quiet,
        };
        let scale = match features.longest_edge() {
            0..1800 => ScaleClass::Small,
            1800..3000 => ScaleClass::Medium,
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
    reason = "all raw-buffer indexes derive from checked image dimensions and interior pixel coordinates"
)]
fn analyze_rgba(pixels: &image::RgbaImage) -> ImageFeatures {
    let width = pixels.width() as usize;
    let height = pixels.height() as usize;
    let pixel_count = width.saturating_mul(height);
    let sampling_ratio = pixel_count.div_ceil(MAX_SAMPLED_PIXELS).max(1);
    let floor_stride = sampling_ratio.isqrt();
    let stride = if floor_stride * floor_stride < sampling_ratio {
        floor_stride + 1
    } else {
        floor_stride
    };
    let raw = pixels.as_raw();

    let mut histogram = [0_u32; 256];
    let mut sampled = 0_u32;
    let mut colorful = 0_u32;
    let mut near_bw = 0_u32;
    let mut gradient_samples = 0_u32;
    let mut detail = 0_u32;
    let mut soft_noise = 0_u32;
    let mut gradient_sum = 0_u32;
    let mut laplacian_sum = 0_u32;
    let mut tile_soft = [0_u32; TILE_TOTAL];
    let mut tile_samples = [0_u32; TILE_TOTAL];

    for y in (0..height).step_by(stride) {
        for x in (0..width).step_by(stride) {
            let (red, green, blue) = rgb_at(raw, width, x, y);
            let maximum = red.max(green).max(blue);
            let minimum = red.min(green).min(blue);
            if maximum - minimum > 8 {
                colorful += 1;
            }
            let center = luma(red, green, blue);
            histogram[usize::from(center)] += 1;
            if center <= 8 || center >= 247 {
                near_bw += 1;
            }
            sampled += 1;

            if x == 0 || y == 0 || x + 1 >= width || y + 1 >= height {
                continue;
            }
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
            gradient_sum += u32::from(gradient);
            laplacian_sum += laplacian;
            if laplacian > 24 {
                detail += 1;
            }
            let is_soft_noise =
                (4..=20).contains(&laplacian) && gradient <= 24;
            if is_soft_noise {
                soft_noise += 1;
            }
            gradient_samples += 1;

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

    let sampled_f64 = f64::from(sampled.max(1));
    let gradient_f64 = f64::from(gradient_samples.max(1));
    let mut entropy = 0.0;
    let mut levels = 0_u16;
    for count in histogram {
        if count == 0 {
            continue;
        }
        levels += 1;
        let probability = f64::from(count) / sampled_f64;
        entropy -= probability * probability.log2();
    }
    let max_tile_soft_noise_percent = tile_soft
        .iter()
        .zip(tile_samples)
        .filter(|(_, count)| *count > 0)
        .map(|(noise, count)| f64::from(*noise) * 100.0 / f64::from(count))
        .fold(0.0_f64, f64::max);

    ImageFeatures {
        width: pixels.width(),
        height: pixels.height(),
        color_percent: f64::from(colorful) * 100.0 / sampled_f64,
        gray_entropy: entropy,
        gray_levels: levels,
        near_bw_percent: f64::from(near_bw) * 100.0 / sampled_f64,
        detail_percent: f64::from(detail) * 100.0 / gradient_f64,
        soft_noise_percent: f64::from(soft_noise) * 100.0 / gradient_f64,
        max_tile_soft_noise_percent,
        mean_gradient: f64::from(gradient_sum) / gradient_f64,
        mean_laplacian: f64::from(laplacian_sum) / gradient_f64,
    }
}

#[inline]
#[expect(
    clippy::indexing_slicing,
    reason = "caller supplies coordinates inside the image buffer"
)]
fn rgb_at(raw: &[u8], width: usize, x: usize, y: usize) -> (u8, u8, u8) {
    let index = (y * width + x) * 4;
    let alpha = raw[index + 3];
    if alpha == 255 {
        return (raw[index], raw[index + 1], raw[index + 2]);
    }
    let composite = |channel: u8| {
        let foreground = u16::from(channel) * u16::from(alpha);
        let background = 255_u16 * u16::from(255 - alpha);
        let value = (foreground + background + 127)
            .checked_div(255)
            .unwrap_or_default();
        u8::try_from(value).unwrap_or(255)
    };
    (
        composite(raw[index]),
        composite(raw[index + 1]),
        composite(raw[index + 2]),
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
}
