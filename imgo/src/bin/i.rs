use std::path::PathBuf;

use imgo::avif::Avif;

/// Batch converting pictures between formats.
#[derive(Debug)]
#[derive(clap::Parser)]
#[command(disable_help_subcommand = true)]
enum CliOpts {
    /// (Lossy) Encode pictures into AVIF.
    Avif {
        #[command(flatten)]
        transcoder: Avif,
        #[clap(flatten)]
        shared: SharedOpts,
    },

    /// (Lossless) Encode pictures into JXL.
    Jxl {
        // #[command(flatten)]
        // transcoder: jxl::Jxl,
        #[clap(flatten)]
        shared: SharedOpts,
    },

    /// Despeckle using imagemagick `-despeckle` function.
    Despeckle {
        // #[command(flatten)]
        // transcoder: despeckle::Despeckle,
        #[clap(flatten)]
        shared: SharedOpts,
    },

    /// Enhance using imagemagick `-enhance` function.
    Enhance {},

    /// Sharpen poorly scanned manga to have crispy dots.
    CleanScan {
        #[clap(flatten)]
        shared: SharedOpts,
    },

    /// Generate shell completion.
    Complete {
        #[clap(short, long)]
        shell: clap_complete::Shell,
    },
}

#[derive(clap::Args)]
#[derive(Debug)]
struct SharedOpts {
    /// The starting point for finding images. Also the backup
    /// folder will be created here.
    /// Defaults to `PWD`.
    #[arg(long, short = 'W')]
    toplevel: Option<PathBuf>,

    /// Leaving original pictures at the place after transcoding
    /// skipping backup.
    #[arg(long, short = 'N')]
    #[arg(default_value_t = false)]
    no_backup: bool,

    /// Number of parallel transcoding to run.
    /// The default job count is transcoder dependent.
    #[arg(long, short = 'J')]
    #[arg(default_value = "1")]
    jobs: usize,

    /// Manually choose pictures to transcode.
    // #[arg(last = true)]
    manual_selection: Option<Vec<PathBuf>>,
}

fn main() {
    ino_tracing::init_tracing_subscriber();

    let cliopts = <CliOpts as clap::Parser>::parse();
    dbg!(&cliopts);
}
