use std::num::NonZeroU64;

use image::RgbaImage;

use anyhow::bail;

use crate::img::ImageFormat;
use crate::tomato::scramble_image;
use crate::transcoder::Meta;
use crate::transcoder::Pixel;

/// 番茄图: scramble/descramble images via a Gilbert-curve pixel
/// permutation. Output is always PNG (lossless).
#[derive(Debug)]
#[derive(clap::Args)]
#[group(id = "TomatoTranscoder")]
pub struct Tomato {
    /// Scramble (obfuscate) the image. Exactly one of `--encrypt` /
    /// `--decrypt` must be given.
    #[arg(long, short)]
    pub encrypt: bool,

    /// Descramble (restore) the image. Exactly one of `--encrypt` /
    /// `--decrypt` must be given.
    #[arg(long, short)]
    pub decrypt: bool,

    /// Key controlling the offset along the Gilbert curve.
    /// The same key is required to reverse scrambling.
    #[arg(long, default_value_t = 1.0)]
    pub key: f64,
}

impl Tomato {
    /// Resolves the encrypt/decrypt pair, erroring if not exactly one
    /// is set. Returns `true` for encrypt, `false` for decrypt.
    ///
    /// # Errors
    ///
    /// Returns an error if neither or both of `--encrypt` / `--decrypt`
    /// are set.
    pub fn mode(&self) -> anyhow::Result<bool> {
        match (self.encrypt, self.decrypt) {
            (true, false) => Ok(true),
            (false, true) => Ok(false),
            (false, false) => {
                bail!("Exactly one of --encrypt / --decrypt is required")
            }
            (true, true) => {
                bail!("--encrypt and --decrypt are mutually exclusive")
            }
        }
    }
}

impl Meta for Tomato {
    fn id(&self) -> &'static str {
        "tomato"
    }

    fn input_formats(&self) -> &'static [ImageFormat] {
        &[
            ImageFormat::PNG,
            ImageFormat::JPG,
            ImageFormat::WEBP,
            ImageFormat::GIF,
        ]
    }

    fn output_format(&self) -> ImageFormat {
        ImageFormat::PNG
    }

    fn default_jobs(&self) -> NonZeroU64 {
        let count = std::thread::available_parallelism()
            .map_or(1_usize, usize::from);
        #[expect(
            clippy::unwrap_used,
            reason = "`count` is clamped to >= 1 before `NonZeroU64::new`"
        )]
        u64::try_from(count).ok().and_then(NonZeroU64::new).unwrap()
    }
}

impl Pixel for Tomato {
    fn transform(&self, img: &mut RgbaImage) -> anyhow::Result<()> {
        let encrypt = self.mode()?;
        scramble_image(img, self.key, encrypt)
    }
}
