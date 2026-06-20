//! 番茄图 (TomatoScramble): pixel-permutation obfuscation based on a
//! 2D Gilbert curve traversal.
//!
//! Ported from the reference Java implementation in `TomatoScramble.java`.
//! Lossless — output must be written to a lossless format (PNG).
//!
//! The integer casts and indexing below mirror the Java reference's
//! `int`-based arithmetic. The algorithm's invariants keep every index
//! and cast in range (Gilbert curve yields a permutation of
//! `0..pixel_count`; offset arithmetic stays inside `[0, pixel_count)`),
//! so the clippy cast/indexing lints are silenced module-wide.

#![expect(clippy::cast_possible_wrap)]
#![expect(clippy::cast_possible_truncation)]
#![expect(clippy::cast_sign_loss)]
#![expect(clippy::cast_precision_loss)]
#![expect(clippy::indexing_slicing)]
#![expect(clippy::too_many_arguments)]

use image::RgbaImage;

/// The golden-ratio-based offset used by the algorithm, given a pixel
/// count and key. Matches Java's
/// `round(((sqrt(5) - 1) / 2) * pixelCount * key)`.
#[must_use]
fn offset(pixel_count: usize, key: f64) -> usize {
    let raw = ((5.0_f64.sqrt() - 1.0) / 2.0) * pixel_count as f64 * key;
    raw.round() as usize
}

/// `i32::signum` re-exported for parity with Java's `Math.signum`.
///
/// Kept as a named helper so the algorithm reads close to the source.
#[inline]
#[must_use]
const fn signum(v: i32) -> i32 {
    v.signum()
}

/// Builds the Gilbert curve permutation of all pixel indices over a
/// `width` x `height` grid, returned in traversal order.
///
/// `positions[i]` is the linear index (`x + y * width`) of the i-th
/// cell visited by the curve.
#[must_use]
pub fn gilbert2d(width: u32, height: u32) -> Vec<u32> {
    let (w, h) = (width as i32, height as i32);
    let mut positions = vec![0u32; width as usize * height as usize];
    let mut pos: usize = 0;

    if w >= h {
        generate2d(&mut positions, &mut pos, width, 0, 0, w, 0, 0, h);
    } else {
        generate2d(&mut positions, &mut pos, width, 0, 0, 0, h, w, 0);
    }
    positions
}

fn generate2d(
    positions: &mut [u32],
    pos: &mut usize,
    width: u32,
    mut x: i32,
    mut y: i32,
    ax: i32,
    ay: i32,
    bx: i32,
    by: i32,
) {
    let w = (ax + ay).abs();
    let h = (bx + by).abs();
    let dax = signum(ax);
    let day = signum(ay);
    let dbx = signum(bx);
    let dby = signum(by);

    if h == 1 {
        for _ in 0..w {
            positions[*pos] = (x + y * width as i32) as u32;
            *pos += 1;
            x += dax;
            y += day;
        }
        return;
    }

    if w == 1 {
        for _ in 0..h {
            positions[*pos] = (x + y * width as i32) as u32;
            *pos += 1;
            x += dbx;
            y += dby;
        }
        return;
    }

    let mut ax2 = ax.div_euclid(2);
    let mut ay2 = ay.div_euclid(2);
    let mut bx2 = bx.div_euclid(2);
    let mut by2 = by.div_euclid(2);
    let w2 = (ax2 + ay2).abs();
    let h2 = (bx2 + by2).abs();

    if 2 * w > 3 * h {
        if (w2 & 1) == 1 && w > 2 {
            ax2 += dax;
            ay2 += day;
        }
        generate2d(positions, pos, width, x, y, ax2, ay2, bx, by);
        generate2d(
            positions,
            pos,
            width,
            x + ax2,
            y + ay2,
            ax - ax2,
            ay - ay2,
            bx,
            by,
        );
    } else {
        if (h2 & 1) == 1 && h > 2 {
            bx2 += dbx;
            by2 += dby;
        }
        generate2d(positions, pos, width, x, y, bx2, by2, ax2, ay2);
        generate2d(
            positions,
            pos,
            width,
            x + bx2,
            y + by2,
            ax,
            ay,
            bx - bx2,
            by - by2,
        );
        generate2d(
            positions,
            pos,
            width,
            x + (ax - dax) + (bx2 - dbx),
            y + (ay - day) + (by2 - dby),
            -bx2,
            -by2,
            -(ax - ax2),
            -(ay - ay2),
        );
    }
}

/// Scrambles or descrambles a 32-bit-per-pixel image (RGBA) in place.
///
/// `encrypt == true` scrambles; `encrypt == false` reverses it. The
/// `key` controls the offset along the Gilbert curve; the same key is
/// required for a successful round-trip.
///
/// `pixels` must be `width * height * 4` bytes long, RGBA8.
pub fn scramble_rgba(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    key: f64,
    encrypt: bool,
) {
    let pixel_count = width as usize * height as usize;
    debug_assert_eq!(pixels.len(), pixel_count * 4);
    if pixel_count == 0 {
        return;
    }

    let positions = gilbert2d(width, height);
    let off = offset(pixel_count, key) % pixel_count;
    let loop_position = pixel_count - off;

    // Treat the buffer as 4-byte pixel slots, copying each pixel from
    // its source to its destination slot via a temporary output buffer.
    let src: &[u8] = pixels;
    let mut dst = vec![0u8; pixel_count * 4];

    let copy_pixel = |src: &[u8],
                      dst: &mut [u8],
                      src_curve_idx: u32,
                      dst_curve_idx: u32| {
        let s = (src_curve_idx as usize) * 4;
        let d = (dst_curve_idx as usize) * 4;
        // Indices stay within `pixel_count * 4`: `positions` is a
        // permutation of `0..pixel_count`, and the `i ± off` /
        // `i - loop_position` arithmetic stays inside `[0, pixel_count)`
        // (see the bounds notes above the loops).
        dst[d..d + 4].copy_from_slice(&src[s..s + 4]);
    };

    // Encrypt:  dst[positions[wrap(i+off)]] = src[positions[i]]
    // Decrypt:  dst[positions[i]]             = src[positions[wrap(i+off)]]
    //
    // where wrap(i+off) = i + off            if i < loop_position (= N - off)
    //                   = i - loop_position   otherwise
    //
    // For i in [0, loop_position):
    //   i + off ranges in [off, N)          — valid.
    // For i in [loop_position, N):
    //   i - loop_position ranges in [0, off) — valid.
    for i in 0..loop_position {
        let a = positions[i]; // curve index i
        let b = positions[i + off]; // curve index wrap(i+off)
        let (s, d) = if encrypt { (a, b) } else { (b, a) };
        copy_pixel(src, &mut dst, s, d);
    }
    for i in loop_position..pixel_count {
        let a = positions[i]; // curve index i
        let b = positions[i - loop_position]; // curve index wrap(i+off)
        let (s, d) = if encrypt { (a, b) } else { (b, a) };
        copy_pixel(src, &mut dst, s, d);
    }

    pixels.copy_from_slice(&dst);
}

/// Convenience wrapper over [`scramble_rgba`] that takes an
/// `image::RgbaImage` directly.
pub fn scramble_image(img: &mut RgbaImage, key: f64, encrypt: bool) {
    let (w, h) = img.dimensions();
    scramble_rgba(img.as_mut(), w, h, key, encrypt);
}

#[cfg(test)]
#[expect(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn gilbert_is_permutation() {
        for (w, h) in [
            (1, 1),
            (2, 2),
            (3, 5),
            (5, 3),
            (8, 8),
            (16, 9),
            (9, 16),
            (64, 32),
        ] {
            let p = gilbert2d(w, h);
            assert_eq!(p.len(), (w as usize) * (h as usize), "{w}x{h}");
            let mut seen = vec![false; p.len()];
            for &idx in &p {
                assert!(idx < p.len() as u32, "{w}x{h}: idx {idx} OOB");
                assert!(!seen[idx as usize], "{w}x{h}: dup {idx}");
                seen[idx as usize] = true;
            }
            assert!(seen.iter().all(|s| *s), "{w}x{h}: missing");
        }
    }

    #[test]
    fn roundtrip_rgba() {
        for (w, h) in [
            (1, 1),
            (2, 2),
            (3, 5),
            (5, 3),
            (8, 8),
            (16, 9),
            (9, 16),
            (31, 7),
            (64, 32),
        ] {
            let n = (w as usize) * (h as usize);
            let original: Vec<u8> =
                (0..(n * 4) as u32).map(|v| v as u8).collect();
            for key in [1.0_f64, 0.5, 2.0, 0.0, 3.7] {
                let mut buf = original.clone();
                scramble_rgba(&mut buf, w, h, key, true);
                // Scrambled should differ (unless degenerate key/size).
                if key != 0.0 && n > 1 {
                    assert_ne!(
                        buf, original,
                        "{w}x{h} key={key} did not change"
                    );
                }
                scramble_rgba(&mut buf, w, h, key, false);
                assert_eq!(
                    buf, original,
                    "{w}x{h} key={key} round-trip failed",
                );
            }
        }
    }

    #[test]
    fn offset_matches_java_formula() {
        // sqrt(5) ≈ 2.2360679; (sqrt(5)-1)/2 ≈ 0.6180339887
        let n = 1000usize;
        assert_eq!(offset(n, 1.0), 618);
        assert_eq!(offset(n, 0.0), 0);
    }

    #[test]
    fn roundtrip_through_png_via_image_crate() {
        // Exercises the real decode -> RGBA8 -> scramble -> encode PNG
        // -> decode pipeline used by the CLI, for a non-power-of-two
        // size and several keys.
        use image::ImageBuffer;
        use image::Rgba;

        for (w, h) in [(37u32, 23u32), (64, 1), (1, 64), (13, 29)] {
            let orig: ImageBuffer<Rgba<u8>, Vec<u8>> =
                ImageBuffer::from_fn(w, h, |x, y| {
                    Rgba([
                        (x * 7) as u8,
                        (y * 11) as u8,
                        (x ^ y) as u8,
                        255,
                    ])
                });

            for key in [1.0_f64, 0.5, 2.0, 3.7] {
                // Scramble, serialize to PNG in memory, reload.
                let mut scrambled = orig.clone();
                scramble_image(&mut scrambled, key, true);
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

                // Descramble the reloaded buffer and compare to original.
                let mut restored = reloaded;
                scramble_image(&mut restored, key, false);
                assert_eq!(
                    restored.as_raw(),
                    orig.as_raw(),
                    "{w}x{h} key={key} PNG round-trip failed",
                );
            }
        }
    }
}
