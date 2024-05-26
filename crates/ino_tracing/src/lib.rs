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

    use std::io::IsTerminal;


    let output = std::io::stderr;

    let fmt_layer = fmt::layer()
        .with_writer( output )
        .with_ansi( output().is_terminal() )
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
