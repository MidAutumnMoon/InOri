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
        debug!( "Construct app" );
        eprintln!( "Prepareing the plan" );

        let new_plan = cliopts.new_plan
            .map( |it| Plan::from_file( &it ) )
            .transpose()
            .context( "Failed to load the new plan" )?
            .tap( |it| trace!( ?it ) );

        let old_plans = cliopts.old_plans
            .map( |it| {
                it.into_iter()
                    .map( |it| Plan::from_file( &it ) )
                    .collect::< Result<Vec<_>, _> >()
                    .context( "Failed to load (one of) old plan files" )
            } )
            .transpose()?
            .tap( |it| trace!( ?it ) );

        eprintln!( "Finished processing plan" );

        Ok( Self { new_plan, old_plans } )
    }

    #[ tracing::instrument( name = "App::run", skip_all ) ]
    fn run( self ) -> AnyResult<()> {
        debug!( "Run the app" );
        eprintln!( "Run the app" );

        if self.new_plan.is_none() && self.old_plans.is_none() {
            eprintln!( "No new or old plans provided, nothing to do" );
            return Ok(());
        }

        todo!()
    }
}

fn main_but_result() -> AnyResult<()> {

    ino_tracing::init_tracing_subscriber();

    eprintln!( "Warming up" );

    let cliopt = {
        debug!( "Parse cliopts" );
        CliOpts::parse().tap( |it| trace!( ?it ) )
    };

    App::new( cliopt )
        .context( "Failed to construct app" )?
        .run()
        .context( "Error ocurred when running app" )?
    ;


    Ok(())

}

fn main() {
    main_but_result().print_error_exit_process();
}
