use tracing::{
    debug,
    info,
};

use std::net::SocketAddr;

mod asset;
mod mime;


const LISTEN: SocketAddr = {
    use std::net::{IpAddr, Ipv4Addr};
    SocketAddr::new(
        IpAddr::V4( Ipv4Addr::new( 127, 0, 0, 1 ) ),
        3000
    )
};


/// The thing that powers https://418.im/
#[ derive( argh::FromArgs, Debug ) ]
struct CmdOpts {
    // listen
    // #[ argh( option, default="Some(LISTEN)" ) ]
    // listen: Option<SocketAddr>,
}


#[ tokio::main ]
async fn main() -> anyhow::Result<()> {

    // Initialize tracing

    ino_tracing::init_tracing_subscriber();


    // Command line options

    let cmd_opts = argh::from_env::<CmdOpts>();

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

    let listener =
        TcpListener::bind( "127.0.0.1:3000" ).await?;

    info!( "listen on http://127.0.0.1:3000" );

    axum::serve( listener, app ).await?;

    Ok(())

}
