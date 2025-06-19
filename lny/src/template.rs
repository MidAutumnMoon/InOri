use std::ops::Deref;
use std::path::Path;
use std::path::PathBuf;
use std::sync::LazyLock;

use anyhow::ensure;
use anyhow::Context;
use anyhow::Result as AnyResult;
use ino_tap::TapExt;
use minijinja::Environment;
use serde::Deserialize;
use tap::Pipe;

use tracing::debug;

// Constructing an [`Environment`] is expensive.
#[ allow( clippy::unwrap_used ) ]
static ENGINE: LazyLock<Engine> = LazyLock::new( || {
    use ino_result::ResultExt;
    use minijinja::UndefinedBehavior;

    debug!( "Initialize global template engine" );

    let context = ContextOfTemplate::new()
        .context( "Failed to initialize context for template" )
        .print_error()
        .unwrap();

    let mut environ = Environment::empty();
    environ.set_undefined_behavior( UndefinedBehavior::Strict );
    environ.set_recursion_limit( 0 );

    Engine { environ, context }.tap_trace()
} );

#[ derive( Debug ) ]
pub struct Engine {
    environ: Environment<'static>,
    context: ContextOfTemplate,
}

impl Engine {
    #[ tracing::instrument( skip_all ) ]
    pub fn render( &self, tmpl: &str ) -> AnyResult<String> {
        debug!( ?tmpl, "Render template" );
        self.environ.render_str( tmpl, &self.context )
            .with_context(
                || format!( r#"Failed to render template "{tmpl}""# )
            )?
            .tap_trace()
            .pipe( Ok )
    }
}

// N.B. May cause test to fail in environment if XDG variables
// are not set, e.g. nix. In this case, set the variables manually.
#[ derive( serde::Serialize, Debug ) ]
pub struct ContextOfTemplate {
    home: PathBuf,
    config: PathBuf,
    data: PathBuf,
    cache: PathBuf,
    state: PathBuf,
    runtime: PathBuf,
}

impl ContextOfTemplate {
    #[ tracing::instrument( name="context_new" ) ]
    pub fn new() -> AnyResult<Self> {
        use etcetera::choose_base_strategy;
        use etcetera::BaseStrategy;

        debug!( "Initialize context for template" );

        let xdg = choose_base_strategy()
            .context( "Failed to find XDG dirs" )?;

        let home = xdg.home_dir().to_owned();
        let config = xdg.config_dir();
        let data = xdg.data_dir();
        let cache = xdg.cache_dir();

        let Some( state ) = xdg.state_dir() else {
            debug!( "Failed to get XDG_STATE_HOME" );
            anyhow::bail!( "XDG_STATE_HOME is not set" );
        };

        let Some( runtime ) = xdg.runtime_dir() else {
            debug!( "Failed to get XDG_RUNTIME_HOME" );
            anyhow::bail!( "XDG_RUNTIME_HOME is not set" );
        };

        Self { home, config, data, cache, state, runtime, }
            .tap_trace()
            .pipe( Ok )
    }
}

/// A [`Path`] wrapper that guaranteed to not contains unrendered
/// templates and be absolute.
#[ derive( Debug, Hash, PartialEq, Eq, Clone ) ]
pub struct RenderedPath {
    inner: PathBuf
}

impl RenderedPath {
    #[ tracing::instrument( skip_all ) ]
    #[ allow( dead_code ) ]
    pub fn from_unrendered( input: &str ) -> AnyResult<Self> {
        use serde::de::IntoDeserializer;
        use serde::de::value::StrDeserializer;
        use serde::de::value::Error as DeError;
        let der: StrDeserializer<DeError> = input.into_deserializer();
        Self::deserialize( der )?.pipe( Ok )
    }
}

impl Deref for RenderedPath {
    type Target = Path;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl AsRef<Path> for RenderedPath {
    fn as_ref( &self ) -> &Path {
        self
    }
}

impl<'de> Deserialize<'de> for RenderedPath {
    #[ tracing::instrument( skip_all ) ]
    fn deserialize<D>( der: D ) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>
    {
        debug!( "Deserialize into RenderedPath" );

        #[ inline ]
        fn ren( tmpl: &str ) -> AnyResult<PathBuf> {
            let path = ENGINE.render( tmpl )?
                .pipe( PathBuf::from );
            ensure!( path.is_absolute(),
                r#"Path must be absolute. Raw: "{}" Rendered: "{}""#,
                tmpl, path.display(),
            );
            Ok( path )
        }
        Ok( Self {
            inner: String::deserialize( der )?
                .pipe_deref( ren )
                .map_err( serde::de::Error::custom )?
                .tap_trace()
        } )
    }
}

#[ cfg( test ) ]
mod test {

    use tracing::trace;

    use super::*;

    #[ test ]
    #[ allow( clippy::unwrap_used ) ]
    fn rendered_path() {
        let tmpls_to_ok = [
            // absolute path
            "/home",
            // valid template
            "{{ home }}",
            "{{ config }}",
            "{{ data }}",
            "{{ cache }}",
            "{{ state }}",
            "{{ runtime }}",
        ];

        let tmpls_to_err = [
            // not absolute
            "wow",
            // invalid template
            "{{ home",
            "{{ what-no-kidding }}",
        ];

        for t in tmpls_to_ok {
            let p = RenderedPath::from_unrendered( t );
            trace!( ?p );
            assert!( p.is_ok() );
        }
        for t in tmpls_to_err {
            let p = RenderedPath::from_unrendered( t );
            trace!( ?p );
            assert!( p.is_err() );
        }
    }

}
