//! ## TODO
//! - [ ] Remote diff

use crate::nixos::BuildOpts;

#[derive(Debug)]
#[derive(clap::Args)]
pub struct Deploy {
    #[command(flatten)]
    build_opts: BuildOpts,
}
