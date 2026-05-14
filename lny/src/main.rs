mod blueprint;
mod step;
mod template;

use crate::blueprint::Blueprint;
use crate::step::StepQueue;

use anyhow::Context;
use anyhow::Result as AnyResult;
use tracing::debug;

use ino_color::ceprintln;
use ino_color::fg::Blue;
use ino_color::fg::Yellow;
use ino_tap::TapExt;

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

impl CliOpts {
    fn parse() -> Self {
        <Self as clap::Parser>::parse()
    }
}

struct App;

impl App {
    #[tracing::instrument(name = "app_run_with", skip_all)]
    fn run_with(cliopts: CliOpts) -> AnyResult<()> {
        ceprintln!(Blue, "Prepareing blueprints");

        let new_blueprint = cliopts
            .new_blueprint
            .map(|it| Blueprint::from_file(&it))
            .transpose()
            .context("Failed to load the new blueprint")?
            .tap_trace();

        let old_blueprint = cliopts
            .old_blueprint
            .map(|it| Blueprint::from_file(&it))
            .transpose()
            .context("Failed to load the old blueprint")?
            .tap_trace();

        if new_blueprint.is_none() && old_blueprint.is_none() {
            ceprintln!(
                Yellow,
                "No new nor old blueprint given, nothing to do"
            );
            return Ok(());
        }

        let (new_blueprint, old_blueprint) =
            [new_blueprint, old_blueprint]
                .map(Option::unwrap_or_default)
                .into();

        let step_queue = StepQueue::new(new_blueprint, old_blueprint)
            .context("Error happened while executing the blueprint")?;

        ceprintln!(Blue, "Check collision");

        // TODO: use new type for checked steps?
        // TODO: structural error for reporting
        for step in step_queue.clone() {
            step.dry_execute()?;
        }

        ceprintln!(Blue, "Execute blueprint");

        for step in step_queue {
            step.execute()?;
        }

        Ok(())
    }
}

fn main() -> AnyResult<()> {
    ino_tracing::init_tracing_subscriber();

    ceprintln!(Blue, "Strech hands");

    let cliopt = {
        debug!("Parse cliopts");
        CliOpts::parse().tap_trace()
    };

    App::run_with(cliopt).context("Error ocurred when running app")
}
