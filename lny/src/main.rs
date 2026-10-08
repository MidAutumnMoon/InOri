mod blueprint;
mod state;
mod step;
mod template;

use crate::blueprint::Blueprint;
use crate::state::PendingState;
use crate::state::load_old;
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
    /// State file recording the last applied blueprint.
    state: Option<PathBuf>,
}

#[must_use]
fn cli() -> OptionParser<CliOpts> {
    let new_blueprint = long("new-blueprint")
        .short('n')
        .argument::<PathBuf>("PATH")
        .help("Blueprint for symlinks to be created")
        .optional();
    let state = long("state")
        .short('s')
        .argument::<PathBuf>("PATH")
        .help(
            "State file recording the last applied blueprint, \
             superseded on success",
        )
        .optional();
    construct!(CliOpts {
        new_blueprint,
        state
    })
    .to_options()
    .descr("Maintaining symlinks")
    .version(env!("CARGO_PKG_VERSION"))
}

fn run(cliopts: CliOpts) -> Result<()> {
    let CliOpts {
        new_blueprint,
        state,
    } = cliopts;

    let Some(new_blueprint) = new_blueprint else {
        warn!("No new blueprint given, nothing to do");
        return Ok(());
    };

    info!("Preparing blueprints");

    let new_blueprint = Blueprint::from_file(&new_blueprint)
        .context("Failed to load the new blueprint")?
        .tap(|blueprint| trace!(?blueprint));

    let (old_symlinks, pending_state) = match state {
        None => (Vec::new(), None),
        Some(state_path) => {
            let old_symlinks = load_old(&state_path)?;
            let pending_state =
                PendingState::new(state_path, &new_blueprint)?;
            (old_symlinks, Some(pending_state))
        }
    };

    let new_symlinks = new_blueprint.into_symlinks();

    let step_queue = StepQueue::new(new_symlinks, old_symlinks)
        .context("Error happened while executing the blueprint")?;

    info!("Check feasibility");

    step_queue.check_feasibility()?;

    info!("Execute blueprint");

    for step in step_queue {
        step.execute()?;
    }

    if let Some(pending_state) = pending_state {
        info!("Persist state");
        pending_state.write()?;
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
