///!

#[global_allocator]
static ALLOC: jemallocator::Jemalloc = jemallocator::Jemalloc;


use tracing::{
    debug,
    info,
};

use std::net::SocketAddr;

use clap::Parser;

mod asset;
mod mime;


/// The thing that powers https://418.im/
#[ derive( Parser, Debug ) ]
#[ command( version = clap::crate_version!() ) ]
struct CmdOpts {
    /// Address that the server will listen on.
    #[ arg( long, short, default_value = "127.0.0.1:3000" ) ]
    listen: Option<SocketAddr>,
}


#[ tokio::main ]
async fn main() -> anyhow::Result<()> {

    // Initialize tracing

    ino_tracing::init_tracing_subscriber();


    // Command line options

    let cmd_opts = CmdOpts::parse();

    debug!( ?cmd_opts );


    // Create axum router

    debug!( "create axum router" );

    let app = {
        use axum::{
            Router,
            http::StatusCode
        };
        use tower_http::trace::TraceLayer;
        use asset::ALL_ASSETS;

        let handle_404 = {
            let fof_page = ALL_ASSETS.get( "404 page" );
            anyhow::ensure! {
                fof_page.is_some(),
                "Static asset \"404.html\" not found"
            }
            let fof_page = fof_page.unwrap();
            move || async {
                ( StatusCode::NOT_FOUND, fof_page.as_response() )
            }
        };

        Router::new()
            .merge( ALL_ASSETS.as_router() )
            .fallback( handle_404 )
            // TODO: add more customisation to tracing
            .layer( TraceLayer::new_for_http() )
    };

    debug!( ?app );


    // Start server

    debug!( "start axum server" );

    use tokio::net::TcpListener;

    debug!( ?cmd_opts.listen, "listen address" );

    let listener =
        // Safety: clap has default set
        TcpListener::bind( &cmd_opts.listen.unwrap() ).await?;

    info! {
        "server started on http://{}/",
        cmd_opts.listen.unwrap()
    };

    axum::serve( listener, app ).await?;

    Ok(())

}
