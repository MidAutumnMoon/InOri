use std::collections::BTreeSet;
use std::num::NonZeroU64;
use std::path::Path;

use anyhow::Context as _;
use serde::Deserialize;
use serde::Serialize;

use crate::img::ImageFormat;
use crate::transcoder::Meta;
use crate::transcoder::Operation;
use crate::transcoder::Tool;
use crate::transcoder::run_command;

/// Mathematically lossless JPEG XL encoding.
#[derive(Debug, Clone, PartialEq, Eq)]
#[derive(clap::Args, Serialize, Deserialize)]
#[group(skip)]
#[serde(default, deny_unknown_fields)]
pub struct Jxl {
    /// Try two lossless modular strategies and retain the smaller output. This
    /// costs an extra encode but avoids content-specific expert settings
    /// making some bilevel pages larger.
    #[arg(
        long = "no-optimize",
        action = clap::ArgAction::SetFalse,
        help = "Skip the second expert lossless strategy and use effort 9 only"
    )]
    pub optimize: bool,
}

impl Default for Jxl {
    fn default() -> Self {
        Self { optimize: true }
    }
}

impl Meta for Jxl {
    fn id(&self) -> &'static str {
        "cjxl"
    }

    fn input_formats(&self) -> &'static [ImageFormat] {
        &[ImageFormat::PNG, ImageFormat::JPG, ImageFormat::GIF]
    }

    fn output_format(&self) -> ImageFormat {
        ImageFormat::JXL
    }

    fn default_jobs(&self) -> NonZeroU64 {
        NonZeroU64::MIN
    }
}

impl Operation for Jxl {
    fn run(&self, input: &Path, output: &Path) -> anyhow::Result<()> {
        let directory =
            tempfile::tempdir().context("create JXL work directory")?;
        let standard_path = directory.path().join("standard.jxl");
        encode_standard(input, &standard_path)?;

        let selected = if self.optimize {
            let expert_path = directory.path().join("expert.jxl");
            encode_expert(input, &expert_path)?;
            let standard_len = std::fs::metadata(&standard_path)
                .context("read standard JXL output metadata")?
                .len();
            let expert_len = std::fs::metadata(&expert_path)
                .context("read expert JXL output metadata")?
                .len();
            if expert_len < standard_len {
                expert_path
            } else {
                standard_path
            }
        } else {
            standard_path
        };

        std::fs::copy(&selected, output).with_context(|| {
            format!("copy selected JXL output to {}", output.display())
        })?;
        Ok(())
    }

    fn required_tools(&self, tools: &mut BTreeSet<Tool>) {
        tools.insert(Tool::Cjxl);
    }
}

fn encode_standard(input: &Path, output: &Path) -> anyhow::Result<()> {
    let mut command = Tool::Cjxl.command();
    command.args(["--distance", "0"]);
    command.args(["--effort", "9"]);
    command.args(["--num_threads", "-1"]);
    command.args([input, output]);
    run_command("cjxl lossless", input, &mut command)
}

fn encode_expert(input: &Path, output: &Path) -> anyhow::Result<()> {
    let mut command = Tool::Cjxl.command();
    command.arg("--allow_expert_options");
    command.args(["--effort", "8"]);
    command.args(["--modular", "1"]);
    command.args(["--lossless_jpeg", "1"]);
    command.args(["--distance", "0"]);
    command.args(["--iterations", "100"]);
    command.args(["--modular_nb_prev_channels", "6"]);
    command.args(["--modular_group_size", "2"]);
    command.args(["--modular_predictor", "15"]);
    command.args(["--num_threads", "-1"]);
    command.args([input, output]);
    run_command("cjxl expert lossless", input, &mut command)
}
