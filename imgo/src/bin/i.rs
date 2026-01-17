use std::path::PathBuf;

use anyhow::Context;
use anyhow::bail;
use anyhow::ensure;
use imgo::BACKUP_DIR_NAME;
use imgo::BaseSeqExt;
use imgo::Image;
use imgo::ImageFormat;
use imgo::RelAbs;
use imgo::Transcoder;
use imgo::avif::Avif;
use imgo::collect_images;
use ino_color::ceprintln;
use ino_color::fg::Yellow;
use tracing::debug;

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
    workspace: Option<PathBuf>,

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
    /// This also disables backup.
    // #[arg(last = true)]
    manual_selection: Option<Vec<PathBuf>>,
}

fn main() -> anyhow::Result<()> {
    ino_tracing::init_tracing_subscriber();
    let cliopts = <CliOpts as clap::Parser>::parse();

    // Get transcoder and opts from cliopts.
    let (transcoder, shared_opts): (&dyn Transcoder, &SharedOpts) =
        match &cliopts {
            CliOpts::Complete { shell } => {
                debug!("Generate shell completion");
                clap_complete::generate(
                    *shell,
                    &mut <CliOpts as clap::CommandFactory>::command(),
                    "i",
                    &mut std::io::stdout(),
                );
                std::process::exit(0);
            }
            CliOpts::Avif { transcoder, shared } => {
                (transcoder as &dyn Transcoder, shared)
            }
            _ => unimplemented!(),
        };

    ceprintln!(Yellow, "[Transcoder is {}]", transcoder.id());

    // Initialize states
    let workspace = {
        let pwd = std::env::current_dir()?;
        shared_opts.workspace.as_ref().map_or(pwd, Clone::clone)
    };

    let input_formats = transcoder.input_formats();

    // Try to collect images
    let images = if let Some(man_sel) = &shared_opts.manual_selection {
        debug!("Use manually chosen images");
        let mut accu = vec![];
        for sel in man_sel {
            let path = RelAbs::from_path(&workspace, sel)?;
            let Some(format) = ImageFormat::from_path(sel) else {
                bail!("The format of {} is not supported", sel.display());
            };
            let extra = BaseSeqExt::try_from(sel.as_ref())?;
            accu.push(Image {
                path,
                format,
                extra,
            });
        }
        accu
    } else {
        debug!(
            "No manual selection, collect images from {} of {:?}",
            workspace.display(),
            input_formats
        );
        collect_images(&workspace, input_formats)
            .context("Failed to collect images")?
    };

    dbg!(&images);

    let backup_dir = {
        let dir = workspace.join(BACKUP_DIR_NAME);
        ensure!(!dir.exists());
        dir
    };

    Ok(())
}
