use std::collections::BTreeSet;
use std::num::NonZeroU64;
use std::path::Path;

use anyhow::Context as _;
use anyhow::ensure;
use serde::Deserialize;
use serde::Serialize;
use tempfile::Builder;
use tempfile::NamedTempFile;

use crate::img::ImageFormat;
use crate::transcoder::Meta as _;
use crate::transcoder::Operation as _;
use crate::transcoder::Tool;
use crate::transcoder::avif::Avif;
use crate::transcoder::jxl::Jxl;
use crate::transcoder::magick::CleanScan;
use crate::transcoder::magick::Denoise;

/// A serializable operation in a recipe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "options",
    rename_all = "kebab-case",
    deny_unknown_fields
)]
pub enum Step {
    Avif(Avif),
    Jxl(Jxl),
    Denoise(Denoise),
    CleanScan(CleanScan),
}

impl Step {
    #[must_use]
    pub fn id(&self) -> &'static str {
        match self {
            Self::Avif(step) => step.id(),
            Self::Jxl(step) => step.id(),
            Self::Denoise(step) => step.id(),
            Self::CleanScan(step) => step.id(),
        }
    }

    #[must_use]
    pub fn input_formats(&self) -> &'static [ImageFormat] {
        match self {
            Self::Avif(step) => step.input_formats(),
            Self::Jxl(step) => step.input_formats(),
            Self::Denoise(step) => step.input_formats(),
            Self::CleanScan(step) => step.input_formats(),
        }
    }

    #[must_use]
    pub fn output_format(&self) -> ImageFormat {
        match self {
            Self::Avif(step) => step.output_format(),
            Self::Jxl(step) => step.output_format(),
            Self::Denoise(step) => step.output_format(),
            Self::CleanScan(step) => step.output_format(),
        }
    }

    #[must_use]
    pub fn default_jobs(&self) -> NonZeroU64 {
        match self {
            Self::Avif(step) => step.default_jobs(),
            Self::Jxl(step) => step.default_jobs(),
            Self::Denoise(step) => step.default_jobs(),
            Self::CleanScan(step) => step.default_jobs(),
        }
    }

    fn validate(&self) -> anyhow::Result<()> {
        match self {
            Self::Avif(options) => {
                ensure!(
                    options.quality <= 100,
                    "AVIF quality must be in 0..=100"
                );
                ensure!(
                    matches!(options.depth, 8 | 10 | 12),
                    "AVIF depth must be 8, 10, or 12"
                );
                ensure!(
                    options.speed <= 10,
                    "AVIF speed must be in 0..=10"
                );
            }
            Self::CleanScan(options) => ensure!(
                options.threshold <= 100,
                "clean-scan threshold must be in 0..=100"
            ),
            Self::Jxl(_) | Self::Denoise(_) => {}
        }
        Ok(())
    }

    fn run(
        &self,
        input: &Path,
        output: &Path,
    ) -> anyhow::Result<Vec<String>> {
        match self {
            Self::Avif(step) => step.run(input, output),
            Self::Jxl(step) => step.run(input, output),
            Self::Denoise(step) => step.run(input, output),
            Self::CleanScan(step) => step.run(input, output),
        }
    }

    fn required_tools(&self, tools: &mut BTreeSet<Tool>) {
        match self {
            Self::Avif(step) => step.required_tools(tools),
            Self::Jxl(step) => step.required_tools(tools),
            Self::Denoise(step) => step.required_tools(tools),
            Self::CleanScan(step) => step.required_tools(tools),
        }
    }
}

/// An ordered, format-checked sequence of preprocessing and encoding steps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    pub steps: Vec<Step>,
}

impl Recipe {
    #[must_use]
    pub fn single(step: Step) -> Self {
        Self { steps: vec![step] }
    }

    #[must_use]
    pub fn first_input_formats(&self) -> Option<&'static [ImageFormat]> {
        self.steps.first().map(Step::input_formats)
    }

    pub fn required_tools(&self, tools: &mut BTreeSet<Tool>) {
        for step in &self.steps {
            step.required_tools(tools);
        }
    }

    /// Validate parameters and every format transition for a concrete input.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty recipe, invalid step parameters, or an
    /// incompatible input/output format transition.
    pub fn validate_for(
        &self,
        input_format: ImageFormat,
    ) -> anyhow::Result<ImageFormat> {
        ensure!(!self.steps.is_empty(), "recipe has no steps");
        let mut format = input_format;
        for step in &self.steps {
            step.validate()
                .with_context(|| format!("invalid {} step", step.id()))?;
            ensure!(
                step.input_formats().contains(&format),
                "{} does not accept {:?} input produced before it",
                step.id(),
                format
            );
            format = step.output_format();
        }
        Ok(format)
    }

    /// Verify every external tool required by the recipe.
    ///
    /// # Errors
    ///
    /// Returns an error when a configured tool is missing or fails its version
    /// probe.
    pub fn verify_tools(&self) -> anyhow::Result<()> {
        let mut tools = BTreeSet::new();
        self.required_tools(&mut tools);
        for tool in tools {
            tool.verify()?;
        }
        Ok(())
    }

    #[must_use]
    pub fn default_jobs(&self) -> NonZeroU64 {
        self.steps
            .last()
            .map_or(NonZeroU64::MIN, Step::default_jobs)
    }

    /// Execute the complete recipe without touching the source. Intermediate
    /// products live in temporary files; only `output` receives the final
    /// encoded bytes.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid recipe, temporary-file failure, or any
    /// failed preprocessing/encoding step.
    pub fn execute(
        &self,
        input: &Path,
        input_format: ImageFormat,
        output: &Path,
    ) -> anyhow::Result<Vec<String>> {
        self.validate_for(input_format)?;

        let mut current_path = input.to_path_buf();
        let mut current_format = input_format;
        let mut intermediates: Vec<NamedTempFile> = Vec::new();
        let mut warnings = Vec::new();

        for (index, step) in self.steps.iter().enumerate() {
            ensure!(
                step.input_formats().contains(&current_format),
                "{} does not accept {:?}",
                step.id(),
                current_format
            );
            let is_final = index + 1 == self.steps.len();
            if is_final {
                warnings.extend(
                    step.run(&current_path, output)
                        .with_context(|| format!("run {}", step.id()))?,
                );
            } else {
                let extension =
                    step.output_format()
                        .primary_extension()
                        .context("step output format has no extension")?;
                let temporary = Builder::new()
                    .suffix(&format!(".{extension}"))
                    .tempfile()
                    .with_context(|| {
                        format!(
                            "create intermediate output for {}",
                            step.id()
                        )
                    })?;
                step.run(&current_path, temporary.path())
                    .with_context(|| format!("run {}", step.id()))?;
                intermediates.push(temporary);
                current_path = intermediates
                    .last()
                    .context("intermediate output was not retained")?
                    .path()
                    .to_path_buf();
            }
            current_format = step.output_format();
        }

        Ok(warnings)
    }
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "test verifies rejected recipes")]
mod tests {
    use super::*;

    #[test]
    fn validates_every_format_transition() {
        let valid = Recipe {
            steps: vec![
                Step::Denoise(Denoise::default()),
                Step::Avif(Avif::default()),
            ],
        };
        assert!(matches!(
            valid.validate_for(ImageFormat::JPG),
            Ok(ImageFormat::AVIF)
        ));

        let invalid = Recipe {
            steps: vec![
                Step::Avif(Avif::default()),
                Step::Jxl(Jxl::default()),
            ],
        };
        invalid.validate_for(ImageFormat::PNG).unwrap_err();
        (Recipe { steps: Vec::new() })
            .validate_for(ImageFormat::PNG)
            .unwrap_err();
    }
}
