//! 番茄图 (TomatoScramble): pixel-permutation obfuscation based on a
//! 2D Gilbert curve traversal.
//!
//! Ported from the reference Java implementation in `TomatoScramble.java`.
//! Lossless — output must be written to a lossless format (PNG).
//!
//! The integer casts below mirror the Java reference's `int`-based
//! arithmetic (`width as i32`, `i32 as u32`, `usize as f64`). These are
//! inherent to the algorithm's geometry and silenced module-wide. All
//! other clippy lints (indexing, too-many-args) are kept active and
//! handled at the source.

#![expect(clippy::cast_possible_wrap)]
#![expect(clippy::cast_possible_truncation)]
#![expect(clippy::cast_sign_loss)]
#![expect(clippy::cast_precision_loss)]

use image::RgbaImage;

/// A 2D integer point/vector, used by the Gilbert curve generator.
#[derive(Clone, Copy, Debug)]
struct Pt(i32, i32);

impl Pt {
    const fn signum(self) -> Self {
        Self(self.0.signum(), self.1.signum())
    }

    const fn abs_sum(self) -> i32 {
        (self.0 + self.1).abs()
    }

    const fn div_euclid(self, n: i32) -> Self {
        Self(self.0.div_euclid(n), self.1.div_euclid(n))
    }
}

impl std::ops::Add for Pt {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self(self.0 + rhs.0, self.1 + rhs.1)
    }
}

impl std::ops::Sub for Pt {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self(self.0 - rhs.0, self.1 - rhs.1)
    }
}

impl std::ops::Neg for Pt {
    type Output = Self;
    fn neg(self) -> Self {
        Self(-self.0, -self.1)
    }
}

/// The golden-ratio-based offset used by the algorithm, given a pixel
/// count and key. Matches Java's
/// `round(((sqrt(5) - 1) / 2) * pixelCount * key)`.
///
/// Clamped to `>= 0`: a negative or NaN key yields `0` instead of
/// wrapping to `usize::MAX`. The caller should still validate the key.
#[must_use]
fn offset(pixel_count: usize, key: f64) -> usize {
    let raw = ((5.0_f64.sqrt() - 1.0) / 2.0) * pixel_count as f64 * key;
    raw.round().max(0.0) as usize
}

/// Builds the Gilbert curve permutation of all pixel indices over a
/// `width` x `height` grid, returned in traversal order.
///
/// `positions[i]` is the linear index (`x + y * width`) of the i-th
/// cell visited by the curve.
#[must_use]
pub fn gilbert2d(width: u32, height: u32) -> Vec<u32> {
    let mut curve = Vec::with_capacity(width as usize * height as usize);
    let (w, h) = (width as i32, height as i32);

    if w >= h {
        generate2d(&mut curve, width, Pt(0, 0), Pt(w, 0), Pt(0, h));
    } else {
        generate2d(&mut curve, width, Pt(0, 0), Pt(0, h), Pt(w, 0));
    }
    curve
}

#[expect(clippy::many_single_char_names)]
fn generate2d(curve: &mut Vec<u32>, width: u32, mut p: Pt, a: Pt, b: Pt) {
    let w = a.abs_sum();
    let h = b.abs_sum();
    let da = a.signum();
    let db = b.signum();

    if h == 1 {
        for _ in 0..w {
            curve.push((p.0 + p.1 * width as i32) as u32);
            p = p + da;
        }
        return;
    }

    if w == 1 {
        for _ in 0..h {
            curve.push((p.0 + p.1 * width as i32) as u32);
            p = p + db;
        }
        return;
    }

    let mut a2 = a.div_euclid(2);
    let mut b2 = b.div_euclid(2);
    let w2 = a2.abs_sum();
    let h2 = b2.abs_sum();

    if 2 * w > 3 * h {
        if (w2 & 1) == 1 && w > 2 {
            a2 = a2 + da;
        }
        generate2d(curve, width, p, a2, b);
        generate2d(curve, width, p + a2, a - a2, b);
    } else {
        if (h2 & 1) == 1 && h > 2 {
            b2 = b2 + db;
        }
        generate2d(curve, width, p, b2, a2);
        generate2d(curve, width, p + b2, a, b - b2);
        generate2d(curve, width, p + (a - da) + (b2 - db), -b2, -(a - a2));
    }
}

/// Scrambles or descrambles a 32-bit-per-pixel image (RGBA8) in place.
///
/// `encrypt == true` scrambles; `encrypt == false` reverses it. The
/// `key` controls the offset along the Gilbert curve; the same key is
/// required for a successful round-trip.
///
/// `pixels` must be `width * height * 4` bytes long, RGBA8. This is
/// checked via `debug_assert`; in release builds a mismatched length
/// panics at the first pixel copy with an opaque message.
///
/// # Key preconditions
///
/// `key` must be finite and non-negative. The caller is responsible
/// for validation; this function does not check. Negative, NaN, or
/// infinite keys produce silent no-ops or nonsensical offsets.
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
#[expect(clippy::indexing_slicing)]
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

    // The permutation is a cyclic shift by `off` along the Gilbert
    // curve: pixel at curve-index `i` moves to `wrap(i + off)`.
    //
    // `wrap(i + off)` equals `i + off` when `i < loop_position`,
    // otherwise `i - loop_position`. Both stay inside `[0, N)`, and
    // `positions` is a permutation of `0..N`, so every index is valid.
    let wrap = |i: usize| -> usize {
        if i < loop_position {
            i + off
        } else {
            i - loop_position
        }
    };

    let src: &[u8] = pixels;
    let mut dst = vec![0u8; pixel_count * 4];

    for i in 0..pixel_count {
        let a = positions[i];
        let b = positions[wrap(i)];
        let (s, d) = if encrypt { (a, b) } else { (b, a) };
        let (s, d) = (s as usize * 4, d as usize * 4);
        dst[d..d + 4].copy_from_slice(&src[s..s + 4]);
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
            // Use a HashSet to verify permutation without indexing.
            let mut seen = std::collections::HashSet::new();
            for &idx in &p {
                assert!(idx < p.len() as u32, "{w}x{h}: idx {idx} OOB");
                assert!(seen.insert(idx), "{w}x{h}: dup {idx}");
            }
            assert_eq!(seen.len(), p.len(), "{w}x{h}: missing indices");
        }
    }

    #[test]
    fn offset_matches_java_formula() {
        // sqrt(5) ≈ 2.2360679; (sqrt(5)-1)/2 ≈ 0.6180339887
        let n = 1000usize;
        assert_eq!(offset(n, 1.0), 618);
        assert_eq!(offset(n, 0.0), 0);
        // Negative and NaN clamp to 0.
        assert_eq!(offset(n, -1.0), 0);
        assert_eq!(offset(n, f64::NAN), 0);
    }

    #[test]
    fn roundtrip() {
        use image::ImageBuffer;
        use image::Rgba;

        let sizes = [
            (1u32, 1u32),
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

        for &(w, h) in &sizes {
            let n = (w as usize) * (h as usize);

            // ── Raw buffer round-trip ───────────────────────────────
            let original: Vec<u8> =
                (0..(n * 4) as u32).map(|v| v as u8).collect();
            for &key in &keys {
                let mut buf = original.clone();
                scramble_rgba(&mut buf, w, h, key, true);
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

            // ── PNG encode/decode round-trip ────────────────────────
            let orig_img: ImageBuffer<Rgba<u8>, Vec<u8>> =
                ImageBuffer::from_fn(w, h, |x, y| {
                    Rgba([
                        (x * 7) as u8,
                        (y * 11) as u8,
                        (x ^ y) as u8,
                        255,
                    ])
                });
            for &key in &keys {
                if key == 0.0 {
                    continue; // identity, skip PNG test
                }
                let mut scrambled = orig_img.clone();
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

                let mut restored = reloaded;
                scramble_image(&mut restored, key, false);
                assert_eq!(
                    restored.as_raw(),
                    orig_img.as_raw(),
                    "{w}x{h} key={key} PNG round-trip failed",
                );
            }
        }
    }
}
