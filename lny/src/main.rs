mod blueprint;
mod step;
mod template;

use crate::blueprint::Blueprint;
use crate::step::StepQueue;

use bpaf::OptionParser;
use bpaf::Parser as _;
use bpaf::construct;
use bpaf::long;
use rootcause::Result;
use rootcause::prelude::ResultExt as _;
use tap::Tap as _;
use tracing::debug;
use tracing::info;
use tracing::trace;
use tracing::warn;

use std::path::PathBuf;

// TODO: use thiserror to replace ad-hoc string errors

/// Maintaining symlinks.
#[derive(Debug)]
struct CliOpts {
    /// Blueprint for symlinks to be created.
    new_blueprint: Option<PathBuf>,
    /// Previous generation of blueprint, symlinks in it
    /// will be removed.
    old_blueprint: Option<PathBuf>,
}

#[must_use]
fn cli() -> OptionParser<CliOpts> {
    let new_blueprint = long("new-blueprint")
        .short('n')
        .argument::<PathBuf>("PATH")
        .help("Blueprint for symlinks to be created")
        .optional();
    let old_blueprint = long("old-blueprint")
        .short('o')
        .argument::<PathBuf>("PATH")
        .help(
            "Previous generation of blueprint, symlinks in it \
             will be removed",
        )
        .optional();
    construct!(CliOpts {
        new_blueprint,
        old_blueprint
    })
    .to_options()
    .descr("Maintaining symlinks")
    .version(env!("CARGO_PKG_VERSION"))
}

fn run(cliopts: CliOpts) -> Result<()> {
    info!("Preparing blueprints");

    let new_blueprint = cliopts
        .new_blueprint
        .map(|path| Blueprint::from_file(&path))
        .transpose()
        .context("Failed to load the new blueprint")?
        .tap(|blueprint| trace!(?blueprint));

    let old_blueprint = cliopts
        .old_blueprint
        .map(|path| Blueprint::from_file(&path))
        .transpose()
        .context("Failed to load the old blueprint")?
        .tap(|blueprint| trace!(?blueprint));

    if new_blueprint.is_none() && old_blueprint.is_none() {
        warn!("No new nor old blueprint given, nothing to do");
        return Ok(());
    }

    let (new_symlinks, old_symlinks) = [new_blueprint, old_blueprint]
        .map(|blueprint| {
            blueprint.map_or_else(Vec::new, Blueprint::into_symlinks)
        })
        .into();

    let step_queue = StepQueue::new(new_symlinks, old_symlinks)
        .context("Error happened while executing the blueprint")?;

    info!("Check feasibility");

    step_queue.check_feasibility()?;

    info!("Execute blueprint");

    for step in step_queue {
        step.execute()?;
    }

    Ok(())
}

fn main() -> Result<()> {
    let _log_guard = ino_tracing::init_tracing_subscriber();

    info!("Stretch hands");

    let cliopt = {
        debug!("Parse cliopts");
        cli().run().tap(|cliopts| trace!(?cliopts))
    };

    run(cliopt)
}
