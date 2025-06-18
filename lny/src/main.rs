mod blueprint;
mod template;
mod step;

use crate::step::StepQueue;
use crate::blueprint::Blueprint;

use anyhow::Result as AnyResult;
use anyhow::Context;
use tracing::debug;

use ino_result::ResultExt;
use ino_tap::TapExt;
use ino_color::InoColor;
use ino_color::fg::Blue;
use ino_color::fg::Yellow;

use std::path::PathBuf;

// TODO: use thiserror to replace adhoc string errors

/// Maintaining symlinks.
#[ derive( clap::Parser, Debug ) ]
struct CliOpts {
    /// Blueprint for symlinks to be created.
    #[ arg( long, short, value_name="PATH" ) ]
    new_blueprint: Option<PathBuf>,
    /// Previous generation of blueprint, symlinks in it
    /// will be removed.
    #[ arg( long, short, value_name="PATH" ) ]
    old_blueprint: Option<PathBuf>,
}

impl CliOpts {
    fn parse() -> Self {
        <Self as clap::Parser>::parse()
    }
}

struct App;

impl App {

    #[ tracing::instrument( name = "app_run_with", skip_all ) ]
    fn run_with( cliopts: CliOpts ) -> AnyResult<()> {
        eprintln!( "{}", "Prepareing blueprints".fg::<Blue>() );

        let new_blueprint = cliopts.new_blueprint
            .map( |it| Blueprint::from_file( &it ) )
            .transpose()
            .context( "Failed to load the new blueprint" )?
            .tap_trace();

        let old_blueprint = cliopts.old_blueprint
            .map( |it| Blueprint::from_file( &it ) )
            .transpose()
            .context( "Failed to load the old blueprint" )?
            .tap_trace();

        if new_blueprint.is_none() && old_blueprint.is_none() {
            eprintln!( "{}",
                "No new nor old blueprint given, nothing to do".fg::<Yellow>() );
            return Ok(());
        }

        let ( new_blueprint, old_blueprint ) =
            [ new_blueprint, old_blueprint ]
                .map( Option::unwrap_or_default )
                .into();

        let step_queue = StepQueue::new( new_blueprint, old_blueprint )
            .context( "Error happened while executing the blueprint" )?;

        eprintln!( "{}",
            "Check collision".fg::<Blue>() );

        // TODO: use new type for checked steps?
        for step in step_queue.clone() {
            step.check_collision()?;
        }

        eprintln!( "{}",
            "Execute blueprint".fg::<Blue>() );

        for step in step_queue {
            step.execute()?;
        }

        Ok(())
    }

}

fn main() {
    fn main_but_result() -> AnyResult<()> {
        let cliopt = {
            debug!( "Parse cliopts" );
            CliOpts::parse().tap_trace()
        };
        App::run_with( cliopt )
            .context( "Error ocurred when running app" )?;
        Ok(())
    }

    ino_tracing::init_tracing_subscriber();

    eprintln!( "{}", "Strech hands".fg::<Blue>() );

    main_but_result().print_error_exit_process();
}
