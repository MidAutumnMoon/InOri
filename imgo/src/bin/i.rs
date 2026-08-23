use rlimit::Resource;

use imgo::automation::PlanOpts;
use imgo::automation::PreviewOpts;
use imgo::automation::RunOpts;
use imgo::automation::create_plan;
use imgo::automation::preview_plan;
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
enum CliOpts {
    /// Analyze a directory, group similar images, and write a reviewable plan.
    Plan(PlanOpts),

    /// Encode representative candidates from a plan for 1:1 visual review.
    Preview(PreviewOpts),

    /// Execute every selected recipe in a reviewed plan.
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
    ino_tracing::init_tracing_subscriber();
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
        CliOpts::Plan(options) => create_plan(options),
        CliOpts::Preview(options) => preview_plan(options),
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
