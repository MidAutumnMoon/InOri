use rlimit::Resource;

use imgo::SharedOpts;
use imgo::Tomato;
use imgo::avif::Avif;
use imgo::jxl::Jxl;
use imgo::magick::CleanScan;
use imgo::magick::Denoise;
use imgo::run_pipeline_external;
use imgo::run_pipeline_pixel;

use tracing::debug;

/// Batch converting pictures between formats.
#[derive(Debug)]
#[derive(clap::Parser)]
#[command(disable_help_subcommand = true)]
enum CliOpts {
    /// (Lossy) Encode pictures into AVIF.
    #[command(visible_alias = "a")]
    Avif {
        #[command(flatten)]
        transcoder: Avif,
        #[clap(flatten)]
        shared: SharedOpts,
    },

    /// (Lossless) Encode pictures into JXL.
    #[command(visible_alias = "j")]
    Jxl {
        #[command(flatten)]
        transcoder: Jxl,
        #[clap(flatten)]
        shared: SharedOpts,
    },

    #[command(visible_alias = "d")]
    Denoise {
        #[command(flatten)]
        transcoder: Denoise,
        #[clap(flatten)]
        shared: SharedOpts,
    },

    /// Sharpen poorly scanned manga to have crispy dots.
    #[command(visible_alias = "c")]
    CleanScan {
        #[command(flatten)]
        transcoder: CleanScan,
        #[clap(flatten)]
        shared: SharedOpts,
    },

    /// 番茄图: scramble/descramble images via a Gilbert-curve pixel
    /// permutation. Output is always PNG (lossless).
    #[command(visible_alias = "t")]
    Tomato {
        #[command(flatten)]
        tomato: Tomato,
        #[clap(flatten)]
        shared: SharedOpts,
    },

    /// Generate shell completion.
    GenComplete {
        #[clap(short, long)]
        shell: clap_complete::Shell,
    },
}

fn main() -> anyhow::Result<()> {
    ino_tracing::init_tracing_subscriber();
    let cliopts = <CliOpts as clap::Parser>::parse();

    // Raise nofile limit to max to avoid "too many open files" errors
    if let Ok((_, hard)) = Resource::NOFILE.get() {
        let _ = Resource::NOFILE.set(hard, hard);
    }

    match &cliopts {
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

        CliOpts::Avif { transcoder, shared } => {
            run_pipeline_external(shared, transcoder)
        }
        CliOpts::Jxl { transcoder, shared } => {
            run_pipeline_external(shared, transcoder)
        }
        CliOpts::Denoise { transcoder, shared } => {
            run_pipeline_external(shared, transcoder)
        }
        CliOpts::CleanScan { transcoder, shared } => {
            run_pipeline_external(shared, transcoder)
        }

        CliOpts::Tomato { tomato, shared } => {
            run_pipeline_pixel(shared, tomato)
        }
    }
}
