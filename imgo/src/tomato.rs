//! 番茄图 (TomatoScramble): pixel-permutation obfuscation based on a
//! 2D Gilbert curve traversal.
//!
//! Ported from the reference Java implementation in `TomatoScramble.java`.
//! Lossless — output must be written to a lossless format (PNG).
//!
//! All integer arithmetic is checked (`checked_*`, `rem_euclid`) and
//! surfaces as `Err` instead of wrapping;
//! `clippy::arithmetic_side_effects` stays enabled as a tripwire. The
//! only silent numeric conversion left is the integral `f64 → usize`
//! cast in `offset()`, expected locally with its justification.

#![warn(clippy::arithmetic_side_effects)]

use anyhow::Context as _;
use anyhow::ensure;
use image::RgbaImage;

/// One axis of the traversal grid.
#[derive(Clone, Copy)]
enum Axis {
    /// Horizontal (`x`) axis.
    Horizontal,
    /// Vertical (`y`) axis.
    Vertical,
}

/// An axis-aligned traversal side: `len` steps along `axis` in a fixed
/// direction.
///
/// The Java reference models sides as general 2D vectors, but every
/// side it produces is axis-aligned; length + direction + axis carries
/// the same information without signed coordinate arithmetic.
#[derive(Clone, Copy)]
struct Side {
    axis: Axis,
    /// Number of cells along the side; ≥ 1.
    len: usize,
    /// Whether travel along `axis` goes toward increasing coordinates.
    positive: bool,
}

impl Side {
    /// A one-cell step in this side's direction.
    fn unit(self) -> Self {
        Self { len: 1, ..self }
    }

    /// The half-length side in the same direction (rounding down).
    fn half(self) -> Self {
        Self {
            len: self.len.div_euclid(2),
            ..self
        }
    }

    /// The part remaining after `head` has been split off.
    fn remainder(self, head: Self) -> Option<Self> {
        Some(Self {
            len: self.len.checked_sub(head.len)?,
            ..self
        })
    }

    /// The same side shortened by one cell.
    fn shortened(self) -> Option<Self> {
        Some(Self {
            len: self.len.checked_sub(1)?,
            ..self
        })
    }

    /// The same side with the opposite direction.
    fn reversed(self) -> Self {
        Self {
            positive: !self.positive,
            ..self
        }
    }
}

/// A traversal position in pixel coordinates, always inside the
/// `width` × `height` grid.
#[derive(Clone, Copy)]
struct Cursor {
    x: usize,
    y: usize,
}

impl Cursor {
    /// Linear pixel index `x + y * width`.
    fn linear(self, width: usize) -> Option<usize> {
        self.y.checked_mul(width)?.checked_add(self.x)
    }

    /// Moves `side.len` cells along `side`.
    fn advance(&mut self, side: Side) -> Option<()> {
        let coordinate = match side.axis {
            Axis::Horizontal => &mut self.x,
            Axis::Vertical => &mut self.y,
        };
        *coordinate = if side.positive {
            coordinate.checked_add(side.len)?
        } else {
            coordinate.checked_sub(side.len)?
        };
        Some(())
    }
}

/// The golden-ratio-based offset used by the algorithm, given a pixel
/// count and key. Matches Java's
/// `round(((sqrt(5) - 1) / 2) * pixelCount * key)`.
///
/// `key` must be finite and non-negative; [`scramble_rgba`] validates
/// at its boundary. Absurdly large products saturate at `usize::MAX`
/// (defined behavior), which is harmless: encrypt and decrypt derive
/// the same offset either way.
#[expect(
    clippy::as_conversions,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "pixel_count is memory-bound far below 2^53 and key is validated non-negative, so both casts are exact"
)]
fn offset(pixel_count: usize, key: f64) -> usize {
    let raw = ((5.0_f64.sqrt() - 1.0) / 2.0) * pixel_count as f64 * key;
    raw.round() as usize
}

/// Converts a dimension to `usize` as the traversal's index type.
fn to_index(dimension: u32, name: &str) -> anyhow::Result<usize> {
    usize::try_from(dimension)
        .with_context(|| format!("{name} does not fit in `usize`"))
}

/// Greatest common divisor (Euclidean algorithm).
fn gcd(mut left: usize, mut right: usize) -> usize {
    while right != 0 {
        let remainder = left.rem_euclid(right);
        left = right;
        right = remainder;
    }
    left
}

/// Reads the 4-byte pixel at pixel `index`.
fn pixel_at(pixels: &[u8], index: usize) -> anyhow::Result<[u8; 4]> {
    let start = index
        .checked_mul(4)
        .context("pixel byte offset overflows `usize`")?;
    let range = start
        ..start
            .checked_add(4)
            .context("pixel byte offset overflows `usize`")?;
    let mut pixel = [0_u8; 4];
    pixel.copy_from_slice(
        pixels.get(range).context("pixel index out of buffer")?,
    );
    Ok(pixel)
}

/// Writes the 4-byte pixel at pixel `index`.
fn set_pixel(
    pixels: &mut [u8],
    index: usize,
    pixel: [u8; 4],
) -> anyhow::Result<()> {
    let start = index
        .checked_mul(4)
        .context("pixel byte offset overflows `usize`")?;
    let range = start
        ..start
            .checked_add(4)
            .context("pixel byte offset overflows `usize`")?;
    pixels
        .get_mut(range)
        .context("pixel index out of buffer")?
        .copy_from_slice(&pixel);
    Ok(())
}

/// Builds the Gilbert curve permutation of all pixel indices over a
/// `width` x `height` grid, returned in traversal order.
///
/// `positions[i]` is the linear index (`x + y * width`) of the i-th
/// cell visited by the curve.
///
/// # Errors
///
/// Returns an error if either dimension is zero or if the traversal's
/// index arithmetic overflows (the latter cannot happen for dimensions
/// whose pixel buffer fits in memory).
pub fn gilbert2d(width: u32, height: u32) -> anyhow::Result<Vec<usize>> {
    ensure!(width > 0 && height > 0, "dimensions must be non-zero");
    let (width, height) =
        (to_index(width, "width")?, to_index(height, "height")?);
    gilbert2d_usize(width, height)
}

/// [`gilbert2d`] over dimensions already converted to `usize`.
fn gilbert2d_usize(
    width: usize,
    height: usize,
) -> anyhow::Result<Vec<usize>> {
    let pixel_count = width
        .checked_mul(height)
        .context("pixel count overflows `usize`")?;

    let mut curve = Vec::with_capacity(pixel_count);
    let origin = Cursor { x: 0, y: 0 };
    let x_side = Side {
        axis: Axis::Horizontal,
        len: width,
        positive: true,
    };
    let y_side = Side {
        axis: Axis::Vertical,
        len: height,
        positive: true,
    };
    let traversal = if width >= height {
        generate2d(&mut curve, width, origin, x_side, y_side)
    } else {
        generate2d(&mut curve, width, origin, y_side, x_side)
    };
    traversal.context("traversal exceeded the index space")?;
    Ok(curve)
}

fn generate2d(
    curve: &mut Vec<usize>,
    width: usize,
    mut origin: Cursor,
    side_a: Side,
    side_b: Side,
) -> Option<()> {
    if side_b.len == 1 {
        for visited in 0..side_a.len {
            if visited > 0 {
                origin.advance(side_a.unit())?;
            }
            curve.push(origin.linear(width)?);
        }
        return Some(());
    }

    if side_a.len == 1 {
        for visited in 0..side_b.len {
            if visited > 0 {
                origin.advance(side_b.unit())?;
            }
            curve.push(origin.linear(width)?);
        }
        return Some(());
    }

    let mut a_half = side_a.half();
    let mut b_half = side_b.half();

    if side_a.len.checked_mul(2)? > side_b.len.checked_mul(3)? {
        if a_half.len & 1 == 1 && side_a.len > 2 {
            a_half.len = a_half.len.checked_add(1)?;
        }
        generate2d(curve, width, origin, a_half, side_b)?;
        let mut rest = origin;
        rest.advance(a_half)?;
        generate2d(curve, width, rest, side_a.remainder(a_half)?, side_b)?;
    } else {
        if b_half.len & 1 == 1 && side_b.len > 2 {
            b_half.len = b_half.len.checked_add(1)?;
        }
        generate2d(curve, width, origin, b_half, a_half)?;
        let mut mid = origin;
        mid.advance(b_half)?;
        generate2d(curve, width, mid, side_a, side_b.remainder(b_half)?)?;
        let mut corner = origin;
        corner.advance(side_a.shortened()?)?;
        corner.advance(b_half.shortened()?)?;
        generate2d(
            curve,
            width,
            corner,
            b_half.reversed(),
            side_a.remainder(a_half)?.reversed(),
        )?;
    }
    Some(())
}

/// Scrambles or descrambles a 32-bit-per-pixel image (RGBA8) in place.
///
/// `encrypt == true` scrambles; `encrypt == false` reverses it. The
/// `key` controls the offset along the Gilbert curve; the same key is
/// required for a successful round-trip.
///
/// `pixels` must be exactly `width * height * 4` bytes long, RGBA8.
///
/// # Errors
///
/// Returns an error if `key` is negative, NaN, or infinite; if
/// `pixels.len() != width * height * 4`; or if the traversal's index
/// arithmetic overflows (the latter cannot happen for dimensions whose
/// pixel buffer fits in memory).
///
/// # Offset and Java interop
///
/// The offset is `round((√5 − 1)/2 · N · key) = round(N · key / φ)`,
/// then taken **modulo `pixel_count`**. The Java reference does *not*
/// take the modulo, so it throws `ArrayIndexOutOfBoundsException` once
/// `offset > pixel_count`, i.e. once `key > φ ≈ 1.618`. This
/// implementation is more robust and round-trips correctly for any
/// non-negative finite key, but for byte-identical interop with the
/// Java tool keep `key < φ`.
///
/// Two keys that are congruent modulo `φ` produce the same scramble
/// (e.g. `key = 2.0` ≡ `key ≈ 0.382` on the same image). In particular,
/// `key = n · φ` (for any positive integer `n`) yields `offset = N`,
/// which modulo `N` is `0` — i.e. the **identity**. A user who picks
/// `key = 1.618` gets no scrambling with no indication.
pub fn scramble_rgba(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    key: f64,
    encrypt: bool,
) -> anyhow::Result<()> {
    ensure!(
        key.is_finite() && key >= 0.0,
        "key must be finite and non-negative (got {key})"
    );

    let (width, height) =
        (to_index(width, "width")?, to_index(height, "height")?);
    let pixel_count = width
        .checked_mul(height)
        .context("pixel count overflows `usize`")?;
    let byte_count = pixel_count
        .checked_mul(4)
        .context("byte count overflows `usize`")?;
    ensure!(
        pixels.len() == byte_count,
        "expected exactly {byte_count} bytes for {width}×{height} RGBA8, got {}",
        pixels.len()
    );
    if pixel_count == 0 {
        return Ok(());
    }

    let positions = gilbert2d_usize(width, height)?;
    let off = offset(pixel_count, key).rem_euclid(pixel_count);
    if off == 0 {
        return Ok(()); // identity, no work needed
    }
    let loop_position = pixel_count
        .checked_sub(off)
        .context("offset exceeds pixel count")?;

    // The scramble is a cyclic shift by `off` along the Gilbert curve.
    // In pixel-index space, the inverse permutation σ (where
    // `new[j] = old[σ(j)]`) walks each cycle either backward (encrypt:
    // step = loop_position) or forward (decrypt: step = off) along the
    // curve. Cycles partition curve-indices by residue mod
    // `gcd(N, step)`, so each residue in `[0, gcd)` is a cycle leader —
    // no `visited` bitmap is needed.
    //
    // Applying σ in place: for each cycle, save the leader's pixel,
    // shift every other pixel one step backward along σ, then drop the
    // saved pixel into the tail. Each `next` slot is read before it is
    // written, so no value is lost.
    let step = if encrypt { loop_position } else { off };
    let num_cycles = gcd(pixel_count, step);

    for start_curve in 0..num_cycles {
        let start_px = positions
            .get(start_curve)
            .copied()
            .context("curve shorter than cycle count")?;
        let leader = pixel_at(pixels, start_px)?;

        let mut cur_curve = start_curve;
        let mut cur_px = start_px;
        loop {
            let next_curve = cur_curve
                .checked_add(step)
                .context("curve index overflows `usize`")?;
            let next_curve = if next_curve < pixel_count {
                next_curve
            } else {
                next_curve
                    .checked_sub(pixel_count)
                    .context("curve index underflows")?
            };
            if next_curve == start_curve {
                set_pixel(pixels, cur_px, leader)?;
                break;
            }
            let next_px = positions
                .get(next_curve)
                .copied()
                .context("curve shorter than pixel count")?;
            // Read next into a temp before writing cur: `cur` and `next`
            // never coincide within a cycle, but Rust can't prove it.
            let next = pixel_at(pixels, next_px)?;
            set_pixel(pixels, cur_px, next)?;
            cur_curve = next_curve;
            cur_px = next_px;
        }
    }
    Ok(())
}

/// Convenience wrapper over [`scramble_rgba`] that takes an
/// `image::RgbaImage` directly.
///
/// # Errors
///
/// Propagates errors from [`scramble_rgba`].
pub fn scramble_image(
    img: &mut RgbaImage,
    key: f64,
    encrypt: bool,
) -> anyhow::Result<()> {
    let (width, height) = img.dimensions();
    scramble_rgba(img.as_mut(), width, height, key, encrypt)
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "test assertions may panic")]
#[expect(
    clippy::as_conversions,
    clippy::integer_division_remainder_used,
    clippy::cast_possible_truncation,
    reason = "test pixel patterns are exact modular byte patterns"
)]
mod tests {
    use super::*;

    #[test]
    fn gilbert_is_permutation() {
        for (width, height) in [
            (1, 1),
            (2, 2),
            (3, 5),
            (5, 3),
            (8, 8),
            (16, 9),
            (9, 16),
            (64, 32),
        ] {
            let points = gilbert2d(width, height).unwrap();
            assert_eq!(
                points.len(),
                (width as usize) * (height as usize),
                "{width}x{height}"
            );
            // Use a HashSet to verify permutation without indexing.
            let mut seen = std::collections::HashSet::new();
            for &idx in &points {
                assert!(
                    idx < points.len(),
                    "{width}x{height}: idx {idx} OOB"
                );
                assert!(seen.insert(idx), "{width}x{height}: dup {idx}");
            }
            assert_eq!(
                seen.len(),
                points.len(),
                "{width}x{height}: missing indices"
            );
        }
    }

    #[test]
    fn offset_matches_java_formula() {
        // sqrt(5) ≈ 2.2360679; (sqrt(5)-1)/2 ≈ 0.6180339887
        let n = 1000_usize;
        assert_eq!(offset(n, 1.0), 618);
        assert_eq!(offset(n, 0.0), 0);
    }

    #[test]
    fn rejects_invalid_input() {
        let mut buf = vec![0_u8; 16]; // 4x1 RGBA
        scramble_rgba(&mut buf, 4, 1, -1.0, true).unwrap_err();
        scramble_rgba(&mut buf, 4, 1, f64::NAN, true).unwrap_err();
        scramble_rgba(&mut buf, 4, 1, f64::INFINITY, true).unwrap_err();
        // Wrong buffer length.
        let (short, _rest) = buf.split_at_mut(15);
        scramble_rgba(short, 4, 1, 1.0, true).unwrap_err();
        gilbert2d(0, 4).unwrap_err();
    }

    #[test]
    fn roundtrip() {
        use image::ImageBuffer;
        use image::Rgba;

        let sizes = [
            (1_u32, 1_u32),
            (2, 2),
            (3, 5),
            (5, 3),
            (8, 8),
            (16, 9),
            (9, 16),
            (31, 7),
            (64, 32),
            (37, 23),
            (64, 1),
            (1, 64),
            (13, 29),
        ];
        let keys = [0.0_f64, 0.5, 1.0, 2.0, 3.7];

        for &(width, height) in &sizes {
            let count = (width as usize) * (height as usize);

            // ── Raw buffer round-trip ───────────────────────────────
            let original: Vec<u8> = (0..count * 4)
                .map(|byte| u8::try_from(byte % 256).unwrap())
                .collect();
            for &key in &keys {
                let mut buf = original.clone();
                scramble_rgba(&mut buf, width, height, key, true).unwrap();
                if key != 0.0 && count > 1 {
                    assert_ne!(
                        buf, original,
                        "{width}x{height} key={key} did not change"
                    );
                }
                scramble_rgba(&mut buf, width, height, key, false)
                    .unwrap();
                assert_eq!(
                    buf, original,
                    "{width}x{height} key={key} round-trip failed",
                );
            }

            // ── PNG encode/decode round-trip ────────────────────────
            let orig_img: ImageBuffer<Rgba<u8>, Vec<u8>> =
                ImageBuffer::from_fn(width, height, |x, y| {
                    Rgba([
                        (x * 7 % 256) as u8,
                        (y * 11 % 256) as u8,
                        (x ^ y) as u8,
                        255,
                    ])
                });
            for &key in &keys {
                if key == 0.0 {
                    continue; // identity, skip PNG test
                }
                let mut scrambled = orig_img.clone();
                scramble_image(&mut scrambled, key, true).unwrap();
                let mut png_bytes = std::io::Cursor::new(Vec::new());
                scrambled
                    .write_to(&mut png_bytes, image::ImageFormat::Png)
                    .unwrap();
                let reloaded = image::load_from_memory_with_format(
                    &png_bytes.into_inner(),
                    image::ImageFormat::Png,
                )
                .unwrap()
                .to_rgba8();

                let mut restored = reloaded;
                scramble_image(&mut restored, key, false).unwrap();
                assert_eq!(
                    restored.as_raw(),
                    orig_img.as_raw(),
                    "{width}x{height} key={key} PNG round-trip failed",
                );
            }
        }
    }
}
