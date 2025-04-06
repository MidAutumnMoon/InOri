use tracing::debug;

use anyhow::bail;
use anyhow::Context;

/// Generate QR Code from stdin or clipboard.
#[ derive( clap::Parser, Debug ) ]
struct CliOpts {
    /// use the content of clipboard as QR Code
    #[ arg( short, exclusive = true ) ]
    clipboard: bool,

    /// read stdin as Qr Code
    #[ arg( short, exclusive = true ) ]
    stdin: bool,
}

fn run( cliopts: CliOpts ) -> anyhow::Result<()> {

    // Generate Qrcode

    debug!( "read data for Qr Code" );

    let data: String = match cliopts {
        CliOpts { clipboard: true, stdin: true } => {
            // Prevented by setting exclusive on arguments
            #[ allow( clippy::unreachable ) ]
            { unreachable!() }
        },

        CliOpts { clipboard: false, stdin: false } => {
            bail!( "Wrong command line options. \
                    Run with --help to see usage." )
        },

        CliOpts { clipboard: true, stdin: false } => {
            debug!( "data source is clipboard" );
            let mut cb = arboard::Clipboard::new()
                .context( "Unable to handle clipboard" )?;
            cb.get_text()
                .context( "Unable to read from clipboard" )?
        },

        CliOpts { clipboard: false, stdin: true } => {
            debug!( "data source is stdin" );
            use std::io::{ read_to_string, stdin };
            read_to_string( stdin().lock() )
                .context( "Unable to read from stdin" )?
        },
    };

    debug!( ?data );

    debug!( "generate Qr Code image" );

    let qrcode = {
        use qrcode::QrCode;
        use qrcode::render::svg;
        let code = QrCode::new( &data )?;
        code.render()
            .min_dimensions( 128, 128 )
            .dark_color( svg::Color( "#000000" ) )
            .light_color( svg::Color( "#ffffff" ) )
            .build()
    };


    // Display Qr Code

    debug!( "saving Qr Code to tempfile" );

    // Lack the motivation to deal with
    // file collinsion or clean up at all.
    let svg_path = {
        // Using UUIDv7 to make files nicely sorted
        let filename =
            format! { "quraa:{}.svg", uuid::Uuid::now_v7() };
        let path = std::env::temp_dir().join( filename );
        std::fs::write( &path, &qrcode )?;
        path
    };

    debug!( ?svg_path );

    debug!( "showing generated Qr Code" );

    std::process::Command::new( "open" )
        .arg( &svg_path )
        .output()
        .context( "Unable to execute command \"open\"" )?
    ;

    Ok(())

}

fn main() {

    ino_tracing::init_tracing_subscriber();

    let cliopts = <CliOpts as clap::Parser>::parse();

    debug!( ?cliopts );

    let _ = run( cliopts )
        .inspect_err( |err| {
            eprintln!( "{err:?}" );
            std::process::exit( 1 );
        } )
    ;

}
