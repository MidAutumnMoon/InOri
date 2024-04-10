use axum::{
    Router,
    response::IntoResponse,
};

use tracing::{
    debug,
};

use crate::mime;


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


macro_rules! asset_routes {
    (
        $( ( $id:literal, $mime:expr, $file:expr, $routes:expr ) )*
    ) => {
        pub static ALL_ASSETS: AllAssets = AllAssets {
            inner: phf::phf_map! {
                $( $id => AssetRoute {
                    name: $id,
                    mime: $mime,
                    data: include_bytes!( $file ),
                    routes: &$routes
                } ),*
            } // end inner phf_map
        }; // end pub static ALL_ASSETS
    }
}

asset_routes! {
    ( "418 cat", mime::JPEG, "418.jpg", [ "/" ] )
    ( "favicon", mime::FAVICON, "favicon.ico", [ "/favicon.ico" ] )

    ( "404 page", mime::HTML, "404.html", [] )
}

/// All static assets.
#[ derive( Debug ) ]
pub struct AllAssets {
    inner: phf::Map<&'static str, AssetRoute>
}

impl AllAssets {
    /// Create a [`Router`]
    #[ tracing::instrument( skip(self) )]
    pub fn as_router( &'static self ) -> Router {
        let mut router = Router::new();
        for ar in self.inner.values() {
            router = router.merge( ar.as_router() )
        }
        router
    }

    pub fn get( &'static self, name: &str )
        -> Option<&AssetRoute>
    {
        self.inner.get( name )
    }
}
