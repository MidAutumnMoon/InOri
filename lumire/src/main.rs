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

/// Creating and destroying symlinks.
#[ derive( clap::Parser, Debug ) ]
struct CliOpts {
    // TODO: reword help
    /// The new plan to activate.
    #[ arg( long, short ) ]
    new: Option<PathBuf>,
    /// Old plans waiting to be cleaned.
    #[ arg( long, short ) ]
    olds: Option< Vec<PathBuf> >,
}

impl CliOpts {
    fn parse() -> Self {
        <Self as clap::Parser>::parse()
    }
}

struct App {
    new: Option<Plan>,
    olds: Option<Vec<Plan>>,
}

impl App {
    #[ tracing::instrument( name = "App::new", skip_all ) ]
    fn new( cliopts: CliOpts ) -> AnyResult<Self> {
        debug!( "Construct app" );
        eprintln!( "Prepareing the plan" );

        let new = cliopts.new
            .map( |it| Plan::from_file( &it ) )
            .transpose()
            .context( "Failed to load the new plan" )?
            .tap( |it| trace!( ?it ) );

        let olds = cliopts.olds
            .map( |it| {
                it.into_iter()
                    .map( |it| Plan::from_file( &it ) )
                    .collect::< Result<Vec<_>, _> >()
                    .context( "Failed to load (one of) old plan files" )
            } )
            .transpose()?
            .tap( |it| trace!( ?it ) );

        eprintln!( "Finished processing plan" );

        Ok( Self { new, olds } )
    }

    #[ tracing::instrument( name = "App::run", skip_all ) ]
    fn run( self ) -> AnyResult<()> {
        debug!( "Run the app" );
        eprintln!( "Run the app" );

        if self.new.is_none() && self.olds.is_none() {
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
