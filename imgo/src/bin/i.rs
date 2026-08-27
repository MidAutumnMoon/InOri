use rlimit::Resource;

use imgo::automation::PlanOpts;
use imgo::automation::RunOpts;
use imgo::automation::create_plan;
use imgo::automation::run_plan;
use imgo::pipeline::SharedOpts;
use imgo::pipeline::run_pipeline_pixel;
use imgo::pipeline::run_pipeline_recipe;
use imgo::recipe::Recipe;
use imgo::recipe::Step;
use imgo::transcoder::avif::Avif;
use imgo::transcoder::jxl::Jxl;
use imgo::transcoder::magick::CleanScan;
use imgo::transcoder::magick::Denoise;
use imgo::transcoder::tomato::Tomato;
use tracing::debug;
use tracing::warn;

/// Content-aware image planning, review, and batch conversion.
#[derive(Debug, clap::Parser)]
#[command(disable_help_subcommand = true)]
// clap creates an implicit ArgGroup for both struct variants and `Args`
// structs. Every options struct used here disables that group; otherwise
// same-named pairs such as `CleanScan` collide. The schema test guards this.
enum CliOpts {
    /// Phase 1: classify, encode review candidates, then stop for selection.
    Plan(PlanOpts),

    /// Phase 2: apply selected recipes, reusing reviewed representative bytes.
    Run(RunOpts),

    /// Directly encode discovered pictures to AVIF.
    #[command(visible_alias = "a")]
    Avif {
        #[command(flatten)]
        transcoder: Avif,
        #[command(flatten)]
        shared: SharedOpts,
    },

    /// Directly encode discovered pictures to mathematically lossless JXL.
    #[command(visible_alias = "j")]
    Jxl {
        #[command(flatten)]
        transcoder: Jxl,
        #[command(flatten)]
        shared: SharedOpts,
    },

    /// Apply one `ImageMagick` denoise operation and emit PNG.
    #[command(visible_alias = "d")]
    Denoise {
        #[command(flatten)]
        transcoder: Denoise,
        #[command(flatten)]
        shared: SharedOpts,
    },

    /// Convert degraded near-bilevel manga scans to one-bit PNG.
    #[command(visible_alias = "c")]
    CleanScan {
        #[command(flatten)]
        transcoder: CleanScan,
        #[command(flatten)]
        shared: SharedOpts,
    },

    /// 番茄图: scramble/descramble via a Gilbert-curve pixel permutation.
    #[command(visible_alias = "t")]
    Tomato {
        #[command(flatten)]
        tomato: Tomato,
        #[command(flatten)]
        shared: SharedOpts,
    },

    /// Generate shell completion.
    GenComplete {
        #[arg(short, long)]
        shell: clap_complete::Shell,
    },
}

fn main() -> anyhow::Result<()> {
    let _log_guard = ino_tracing::init_tracing_subscriber();
    let options = <CliOpts as clap::Parser>::parse();

    match Resource::NOFILE.get() {
        Ok((_, hard)) => {
            if let Err(error) = Resource::NOFILE.set(hard, hard) {
                warn!(%error, "Failed to raise the open-file limit");
            }
        }
        Err(error) => warn!(%error, "Failed to query the open-file limit"),
    }

    match &options {
        CliOpts::Plan(options) => {
            create_plan(options)?;
            Ok(())
        }
        CliOpts::Run(options) => run_plan(options),
        CliOpts::Avif { transcoder, shared } => {
            run_pipeline_recipe(
                shared,
                Recipe::single(Step::Avif(transcoder.clone())),
            )?;
            Ok(())
        }
        CliOpts::Jxl { transcoder, shared } => {
            run_pipeline_recipe(
                shared,
                Recipe::single(Step::Jxl(transcoder.clone())),
            )?;
            Ok(())
        }
        CliOpts::Denoise { transcoder, shared } => {
            run_pipeline_recipe(
                shared,
                Recipe::single(Step::Denoise(transcoder.clone())),
            )?;
            Ok(())
        }
        CliOpts::CleanScan { transcoder, shared } => {
            run_pipeline_recipe(
                shared,
                Recipe::single(Step::CleanScan(transcoder.clone())),
            )?;
            Ok(())
        }
        CliOpts::Tomato { tomato, shared } => {
            run_pipeline_pixel(shared, tomato)?;
            Ok(())
        }
        CliOpts::GenComplete { shell } => {
            debug!("Generate shell completion");
            clap_complete::generate(
                *shell,
                &mut <CliOpts as clap::CommandFactory>::command(),
                "i",
                &mut std::io::stdout(),
            );
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory as _;

    use super::*;

    #[test]
    fn command_schema_has_no_argument_collisions() {
        CliOpts::command().debug_assert();
    }
}
