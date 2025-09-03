//! ## TODO
//! - [ ] Remote diff

use crate::nixos::BuildOpts;

#[derive(Debug)]
#[derive(clap::Args)]
pub struct Deploy {
    #[arg(long, short)]
    #[arg(value_name = "NAME")]
    exclude_host: Option<Vec<String>>,

    #[command(flatten)]
    build_opts: BuildOpts,
}
