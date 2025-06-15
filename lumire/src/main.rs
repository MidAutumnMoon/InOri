mod manifest;
mod template;
mod executor;

use crate::manifest::Manifest;

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
    /// The new manifest to activate.
    #[ arg( long, short ) ]
    new: Option<PathBuf>,
    /// Old manifests waiting to be cleaned.
    #[ arg( long, short ) ]
    olds: Option< Vec<PathBuf> >,
}

impl CliOpts {
    fn parse() -> Self {
        <Self as clap::Parser>::parse()
    }
}

struct App {
    new: Option<Manifest>,
    olds: Option<Vec<Manifest>>,
}

impl App {
    #[ tracing::instrument( name = "App::new", skip_all ) ]
    fn new( cliopts: CliOpts ) -> AnyResult<Self> {
        debug!( "Construct app" );
        eprintln!( "Prepareing the manifest" );

        let new = cliopts.new
            .map( |it| Manifest::from_file( &it ) )
            .transpose()
            .context( "Failed to load the new manifest" )?
            .tap( |it| trace!( ?it ) );

        let olds = cliopts.olds
            .map( |it| {
                it.into_iter()
                    .map( |it| Manifest::from_file( &it ) )
                    .collect::< Result<Vec<_>, _> >()
                    .context( "Failed to load (one of) old manifest file" )
            } )
            .transpose()?
            .tap( |it| trace!( ?it ) );

        eprintln!( "Finished processing manifests" );

        Ok( Self { new, olds } )
    }

    #[ tracing::instrument( name = "App::run", skip_all ) ]
    fn run( self ) -> AnyResult<()> {
        debug!( "Run the app" );
        eprintln!( "Run the app" );

        if self.new.is_none() && self.olds.is_none() {
            eprintln!( "No new or old manifests provided, nothing to do" );
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
