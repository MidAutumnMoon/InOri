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
    /// Plans for old symlinks to be removed. Can repeat multiple times
    /// to select more old plans.
    #[ arg( long="old-plan", short, value_name="PATH" ) ]
    old_plans: Option< Vec<PathBuf> >,
}

impl CliOpts {
    fn parse() -> Self {
        <Self as clap::Parser>::parse()
    }
}

struct App {
    new_plan: Option<Plan>,
    old_plans: Option<Vec<Plan>>,
}

impl App {
    #[ tracing::instrument( name = "App::new", skip_all ) ]
    fn new( cliopts: CliOpts ) -> AnyResult<Self> {
        eprintln!( "{}", "Prepareing plans".fg::<Blue>() );

        let new_plan = cliopts.new_plan
            .map( |it| Plan::from_file( &it ) )
            .transpose()
            .context( "Failed to load the new plan" )?
            .tap_trace();

        let old_plans = cliopts.old_plans
            .map( |it| {
                it.into_iter()
                    .map( |it| Plan::from_file( &it ) )
                    .collect::< Result<Vec<_>, _> >()
                    .context( "Failed to load (one of) old plan files" )
            } )
            .transpose()?
            .tap_trace();

        eprintln!( "{}", "Finished processing plan".fg::<Blue>() );

        Ok( Self { new_plan, old_plans } )
    }

    #[ tracing::instrument( name = "App::run", skip_all ) ]
    fn run( self ) -> AnyResult<()> {
        eprintln!( "{}", "Run the app".fg::<Blue>() );

        if self.new_plan.is_none() && self.old_plans.is_none() {
            eprintln!( "{}",
                "No new nor old plans, nothing to do".fg::<Yellow>() );
            return Ok(());
        }

        todo!()
    }
}

fn main() {
    fn main_but_result() -> AnyResult<()> {
        let cliopt = {
            debug!( "Parse cliopts" );
            CliOpts::parse().tap_trace()
        };
        App::new( cliopt )
            .context( "Failed to construct app" )?
            .run()
            .context( "Error ocurred when running app" )?;
        Ok(())
    }

    ino_tracing::init_tracing_subscriber();

    eprintln!( "{}", "Strech hands".fg::<Blue>() );

    main_but_result().print_error_exit_process();
}
