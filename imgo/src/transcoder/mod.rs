use std::collections::BTreeSet;
use std::num::NonZeroU64;
use std::path::Path;
use std::process::Command;

use anyhow::Context as _;
use anyhow::bail;
use image::RgbaImage;

use crate::img::ImageFormat;

pub mod avif;
pub mod jxl;
pub mod magick;
pub mod tomato;

/// External programs used by recipe steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tool {
    AvifEnc,
    Cjxl,
    Magick,
}

impl Tool {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::AvifEnc => "avifenc",
            Self::Cjxl => "cjxl",
            Self::Magick => "magick",
        }
    }

    #[must_use]
    pub fn command(self) -> Command {
        let program = match self {
            Self::AvifEnc => {
                std::option_env!("CFG_AVIFENC_PATH").unwrap_or("avifenc")
            }
            Self::Cjxl => {
                std::option_env!("CFG_CJXL_PATH").unwrap_or("cjxl")
            }
            Self::Magick => {
                std::option_env!("CFG_MAGICK_PATH").unwrap_or("magick")
            }
        };
        Command::new(program)
    }

    /// Proves that the configured executable can be spawned before a batch
    /// mutates any source files.
    ///
    /// # Errors
    ///
    /// Returns an error when the executable cannot be spawned or its version
    /// probe exits unsuccessfully.
    pub fn verify(self) -> anyhow::Result<()> {
        let mut command = self.command();
        command.arg(match self {
            Self::Magick => "-version",
            Self::AvifEnc | Self::Cjxl => "--version",
        });
        let output = command
            .output()
            .with_context(|| format!("spawn {}", self.name()))?;
        if !output.status.success() {
            bail!(
                "{} failed its version probe (exit {:?}):\n{}",
                self.name(),
                output.status.code(),
                String::from_utf8_lossy(&output.stderr),
            );
        }
        Ok(())
    }
}

/// Metadata shared by transcoders and in-process pixel operations.
pub trait Meta: Send + Sync {
    fn id(&self) -> &'static str;
    fn input_formats(&self) -> &'static [ImageFormat];
    fn output_format(&self) -> ImageFormat;
    fn default_jobs(&self) -> NonZeroU64;
}

/// One recipe operation. Implementations own process execution so an
/// operation may try several equivalent encodings and retain the smallest.
pub trait Operation: Meta {
    /// Execute the operation into `output`.
    ///
    /// # Errors
    ///
    /// Returns an error when parameters are invalid, process execution fails,
    /// or the output cannot be materialized.
    fn run(
        &self,
        input: &Path,
        output: &Path,
    ) -> anyhow::Result<Vec<String>>;

    fn required_tools(&self, tools: &mut BTreeSet<Tool>);
}

/// In-process pixel transcoders. The orchestrator decodes and encodes; the
/// implementation only mutates pixels.
pub trait Pixel: Meta {
    /// Mutate decoded pixels in place.
    ///
    /// # Errors
    ///
    /// Returns an error when parameters or image dimensions are invalid.
    fn transform(&self, img: &mut RgbaImage) -> anyhow::Result<()>;
}

pub(crate) fn run_command(
    operation: &str,
    input: &Path,
    command: &mut Command,
) -> anyhow::Result<()> {
    let output = command
        .output()
        .with_context(|| format!("spawn {operation}"))?;
    if !output.status.success() {
        bail!(
            "{operation} failed for {} (exit {:?}):\nstdout: {}\nstderr: {}",
            input.display(),
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }
    Ok(())
}
