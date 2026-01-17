use std::path::PathBuf;

#[derive(Debug)]
#[derive(clap::Parser)]
enum CliOpts {}

#[derive(clap::Args)]
#[derive(Debug)]
struct SharedOpts {
    /// (to write...)
    /// Defaults to PWD.
    #[arg(long, short = 'W')]
    workspace: Option<PathBuf>,

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

fn main() {
    ino_tracing::init_tracing_subscriber();

    println!("Hello, imgo!");
}
