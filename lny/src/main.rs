mod blueprint;
mod step;
mod template;

use crate::blueprint::Blueprint;
use crate::step::StepQueue;

use anyhow::Context as _;
use anyhow::Result as AnyResult;
use tap::Tap as _;
use tracing::debug;
use tracing::info;
use tracing::trace;
use tracing::warn;

use std::path::PathBuf;

// TODO: use thiserror to replace ad-hoc string errors

/// Maintaining symlinks.
#[derive(clap::Parser, Debug)]
struct CliOpts {
    /// Blueprint for symlinks to be created.
    #[arg(long, short, value_name = "PATH")]
    new_blueprint: Option<PathBuf>,
    /// Previous generation of blueprint, symlinks in it
    /// will be removed.
    #[arg(long, short, value_name = "PATH")]
    old_blueprint: Option<PathBuf>,
}

#[tracing::instrument(name = "app_run", skip_all)]
fn run(cliopts: CliOpts) -> AnyResult<()> {
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

    let (new_blueprint, old_blueprint) = [new_blueprint, old_blueprint]
        .map(Option::unwrap_or_default)
        .into();

    let step_queue = StepQueue::new(new_blueprint, old_blueprint)
        .context("Error happened while executing the blueprint")?;

    info!("Check feasibility");

    // TODO: use new type for checked steps?
    // TODO: structural error for reporting
    for step in step_queue.clone() {
        step.check_feasibility()?;
    }

    info!("Execute blueprint");

    for step in step_queue {
        step.execute()?;
    }

    Ok(())
}

fn main() -> AnyResult<()> {
    ino_tracing::init_tracing_subscriber();

    info!("Stretch hands");

    let cliopt = {
        debug!("Parse cliopts");
        <CliOpts as clap::Parser>::parse().tap(|cliopts| trace!(?cliopts))
    };

    run(cliopt).context("Error occurred when running app")
}
