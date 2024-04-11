use axum::{
    Router,
    response::IntoResponse,
};

use tracing::{
    debug,
};

use crate::mime;


/// asset! {
///     ( A, "mime", "file", [] )
///     ...
/// }
///
/// ===>
///
/// pub static A = &AssetRoute { ... };
/// ...
/// pub static ALL_ASSETS = AllAssets { &[A ...] };
macro_rules! assets {
    (
        $( ( $id:ident, $mime:expr, $file:expr, $routes:expr ) )*
    ) => {
        $(
            pub static $id: &AssetRoute = &AssetRoute {
                name: std::stringify!( $id ),
                mime: $mime,
                data: include_bytes!( $file ),
                routes: &$routes
            };
        )*
        pub static ALL_ASSETS: AllAssets = AllAssets {
            inner: &[ $( $id ),* ]
        };
    }
}

assets! {
    ( FAVICON,    mime::FAVICON, "favicon.ico", [ "/favicon.ico" ] )
    ( NOT_FOUND,  mime::HTML,    "404.html",    [] )
    ( TEAPOT_CAT, mime::JPEG,    "418.jpg",     [ "/" ] )
}



/// A single static asset with one or more routes.
#[ derive( Debug ) ]
pub struct AssetRoute {
    name: &'static str,
    mime: &'static str,
    data: &'static [u8],
    routes: &'static [&'static str]
}

impl AssetRoute {
    /// Create a new [`Router`] for current asset,
    /// with name as the path.
    #[ tracing::instrument(
        skip_all,
        fields( ?self.name, ?self.mime, ?self.routes )
    ) ]
    pub fn as_router( &'static self ) -> Router {
        use axum::{
            Router,
            routing::get,
        };
        debug!( "create route for static asset" );
        let mut router = Router::new();
        for route in self.routes {
            debug!( ?route, "assign route" );
            router = router.route(
                route, get( || async { self.as_response() } )
            )
        }
        router
    }

    /// Create a response containing `data` and having
    /// proper headers set.
    #[ tracing::instrument(
        skip(self),
        fields( ?self.name )
    ) ]
    pub fn as_response( &'static self ) -> impl IntoResponse {
        use axum::http::{
            HeaderMap,
            HeaderValue as V,
            header,
        };
        let mut headers = HeaderMap::with_capacity( 4 );
        headers.insert(
            header::CONTENT_TYPE,
            V::from_static( self.mime )
        );
        headers.insert(
            header::CACHE_CONTROL,
            V::from_static( "public, max-age 31536000, immutable" )
        );
        debug!( ?self.name, "respond with static file" );
        ( headers, self.data )
    }
}


/// All static assets.
#[ derive( Debug ) ]
pub struct AllAssets {
    inner: &'static [&'static AssetRoute]
}

impl AllAssets {
    #[ tracing::instrument( skip(self) )]
    pub fn as_router( &'static self ) -> Router {
        let mut router = Router::new();
        for ar in self.inner {
            router = router.merge( ar.as_router() )
        }
        router
    }
}
