/// Init custom tracing_subscriber configuration.
#[ inline( always ) ]
pub fn init_tracing_subscriber() {

    use tracing::Level;

    use tracing_subscriber::prelude::*;

    use tracing_subscriber::{
        EnvFilter,
        fmt,
        registry
    };


    let fmt_layer = fmt::layer()
        .with_writer( std::io::stderr )
        .with_ansi( true )
        ;

    let env_layer = EnvFilter::builder()
        .with_default_directive( Level::INFO.into() )
        .from_env_lossy()
        ;


    registry()
        .with( fmt_layer )
        .with( env_layer )
        .init()

}
