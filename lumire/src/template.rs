use std::path::Path;
use std::path::PathBuf;
use std::sync::LazyLock;

use anyhow::ensure;
use anyhow::Context;
use anyhow::Result as AnyResult;
use minijinja::Environment;
use serde::Deserialize;
use tap::Pipe;

use tap::Tap;
use tracing::debug;
use tracing::trace;

#[ allow( clippy::unwrap_used ) ]
static ENGINE: LazyLock<Engine> = LazyLock::new( || {
    use ino_result::ResultExt;
    use minijinja::UndefinedBehavior;

    debug!( "Initialize global template engine" );

    let context = ContextForTemplate::new()
        .context( "Failed to initialize context for template" )
        .print_error()
        .unwrap();

    let mut environ = Environment::empty();
    environ.set_undefined_behavior( UndefinedBehavior::Strict );
    environ.set_recursion_limit( 0 );

    Engine { environ, context }
        .tap( |it| trace!( ?it ) )
} );

#[ derive( Debug ) ]
pub struct Engine {
    environ: Environment<'static>,
    context: ContextForTemplate,
}

#[ derive( serde::Serialize, Debug ) ]
pub struct ContextForTemplate {
    home: PathBuf,
    config: PathBuf,
    data: PathBuf,
    cache: PathBuf,
    state: PathBuf,
    runtime: PathBuf,
}

impl Engine {
    #[ tracing::instrument( skip( self ) ) ]
    pub fn render( &self, tmpl: &str ) -> AnyResult<String> {
        debug!( "Render template" );
        self.environ.render_str( tmpl, &self.context )
            .with_context(
                || format! { r#"Failed to render template "{tmpl}""# }
            )?
            .tap( |it| trace!( ?it ) )
            .pipe( Ok )
    }
}

impl ContextForTemplate {
    #[ tracing::instrument( name="ContextForTemplate::new" ) ]
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
            .tap( |it| trace!( ?it ) )
            .pipe( Ok )
    }
}

#[ derive( Debug ) ]
pub struct RenderedPath {
    inner: PathBuf
}

impl<'de> Deserialize<'de> for RenderedPath {
    #[ allow( clippy::unwrap_in_result ) ]
    #[ tracing::instrument( skip_all ) ]
    fn deserialize<D>( der: D ) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>
    {
        #[ inline ]
        fn ren( tmpl: &str ) -> AnyResult<PathBuf> {
            let rendered = ENGINE.render( tmpl )?;
            let path = PathBuf::from( rendered );
            ensure!( path.is_absolute(),
                r#"Path must be absolute. Raw: "{}" Rendered: "{}""#,
                tmpl, path.display(),
            );
            Ok( path )
        }
        debug!( "Deserialize into RenderedPath" );
        Ok( Self {
            inner: String::deserialize( der )?
                .pipe_deref( ren )
                .map_err( serde::de::Error::custom )?
                .tap( |it| trace!( ?it ) )
        } )
    }
}

impl RenderedPath {
    pub fn path( &self ) -> &Path {
        &self.inner
    }
}

#[ cfg( test ) ]
mod test {
    use super::*;
    use serde::de::value::StrDeserializer;
    use serde::de::value::Error as DeError;
    use serde::de::IntoDeserializer;

    #[ test ]
    #[ allow( clippy::unwrap_used ) ]
    fn rendered_path() {
        ino_tracing::init_tracing_subscriber();

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
            // non absolute
            "wow",
            // invalid template
            "{{ home",
            "{{ what-no-kidding }}",
        ];

        for t in tmpls_to_ok {
            let der: StrDeserializer<DeError> = t.into_deserializer();
            let rp = RenderedPath::deserialize( der );
            trace!( ?rp );
            assert!( rp.is_ok() );
        }
        for t in tmpls_to_err {
            let der: StrDeserializer<DeError> = t.into_deserializer();
            let rp = RenderedPath::deserialize( der );
            trace!( ?rp );
            assert!( rp.is_err() );
        }
    }
}
