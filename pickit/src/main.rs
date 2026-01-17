use std::path::PathBuf;

mod pic;
mod transcoder;

pub const BACKUP_DIR_NAME: &str = ".backup";

/// Batch converting pictures between formats.
#[derive(Debug)]
#[derive(clap::Parser)]
#[command(disable_help_subcommand = true)]
enum CliOpts {
    /// (Lossy) Encode pictures into AVIF.
    Avif {
        // #[command(flatten)]
        // transcoder: avif::Avif,
        #[clap(flatten)]
        shared: SharedCliOpts,
    },

    /// (Lossless) Encode pictures into JXL.
    Jxl {
        // #[command(flatten)]
        // transcoder: jxl::Jxl,
        #[clap(flatten)]
        shared: SharedCliOpts,
    },

    /// Despeckle using imagemagick `-despeckle` function.
    Despeckle {
        // #[command(flatten)]
        // transcoder: despeckle::Despeckle,
        #[clap(flatten)]
        shared: SharedCliOpts,
    },

    /// Enhance using imagemagick `-enhance` function.
    Enhance {},

    /// Sharpen poorly scanned manga to have crispy dots.
    SharpenScan,

    /// Generate shell completion.
    Complete {
        #[clap(long)]
        shell: clap_complete::Shell,
    },
}

#[derive(clap::Args)]
#[derive(Debug)]
struct SharedCliOpts {
    /// (to write...)
    /// Defaults to PWD.
    #[arg(long, short = 'W')]
    workspace: Option<PathBuf>,

    #[arg(long, short = 'R')]
    no_recursive: bool,

    /// Leaving original pictures at the place after transcoding
    /// skipping backup.
    #[arg(long, short = 'N')]
    #[arg(default_value_t = false)]
    no_backup: bool,

    /// Number of parallel transcoding to run.
    #[arg(long, short = 'J')]
    #[arg(default_value = "1")]
    jobs: usize,

    /// Show logs from transcoders.
    #[arg(long, short = 'L')]
    #[arg(default_value_t = false)]
    show_logs: bool,

    /// Manually choose pictures to transcode.
    #[arg(last = true)]
    manual_selection: Option<Vec<PathBuf>>,
}

// TODO: use local runtime once it's stabilized
#[tokio::main(flavor = "current_thread")]
async fn main() {
    println!("Hello, world!");
}
