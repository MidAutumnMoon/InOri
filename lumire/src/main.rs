mod plan;
mod template;
mod executor;

use crate::plan::Plan;

use anyhow::Result as AnyResult;
use anyhow::Context;
use tap::Pipe;
use tap::Tap;
use tracing::debug;
use tracing::trace;

use ino_result::ResultExt;
use ino_tap::TapExt;
use ino_color::InoColor;
use ino_color::fg::Blue;
use ino_color::fg::Yellow;

use std::path::PathBuf;

/// Maintaining symlinks.
#[ derive( clap::Parser, Debug ) ]
struct CliOpts {
    /// Plan for symlinks to be created.
    #[ arg( long, short, value_name="PATH" ) ]
    new_plan: Option<PathBuf>,
    /// Plans for old symlinks to be removed.
    #[ arg( long, short, value_name="PATH" ) ]
    old_plan: Option<PathBuf>,
}

impl CliOpts {
    fn parse() -> Self {
        <Self as clap::Parser>::parse()
    }
}

struct App { }

impl App {
    #[ tracing::instrument( name = "App::new", skip_all ) ]
    fn run_with( cliopts: CliOpts ) -> AnyResult<()> {
        eprintln!( "{}", "Prepareing plan".fg::<Blue>() );

        let new_plan = cliopts.new_plan
            .map( |it| Plan::from_file( &it ) )
            .transpose()
            .context( "Failed to load the new plan" )?
            .tap_trace();

        let old_plan = cliopts.old_plan
            .map( |it| Plan::from_file( &it ) )
            .transpose()
            .context( "Failed to load the old plan" )?
            .tap_trace();

        eprintln!( "{}", "Run the app".fg::<Blue>() );

        if new_plan.is_none() && old_plan.is_none() {
            eprintln!( "{}",
                "No new nor old plan, nothing to do".fg::<Yellow>() );
            return Ok(());
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
        App::run_with( cliopt ).context( "Error ocurred when running app" )?;
        Ok(())
    }

    ino_tracing::init_tracing_subscriber();

    eprintln!( "{}", "Strech hands".fg::<Blue>() );

    main_but_result().print_error_exit_process();
}
