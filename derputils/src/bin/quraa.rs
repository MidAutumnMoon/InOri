use tracing::{
    debug,
    info,
};

use anyhow::bail;
use clap::Parser;


/// Generate QR Code. Hint: window can also be closed
/// by pressing Q or ESC.
#[ derive( Parser, Debug, Clone, Copy ) ]
struct CmdOpts {
    /// use the content of clipboard as QR Code
    #[ arg( short, exclusive = true ) ]
    clipboard: bool,

    /// read stdin as Qr Code
    #[ arg( short, exclusive = true ) ]
    stdin: bool,
}


fn main() -> anyhow::Result<()> {

    // Enable tracing

    ino_tracing::init_tracing_subscriber();


    // Deal with inputs

    let opts = CmdOpts::parse();

    debug!( ?opts, "command options" );


    // Generate Qrcode

    info!( "read data for Qr Code" );

    let data: String = match opts {
        CmdOpts { clipboard: true, stdin: true } => {
            // Prevented by setting exclusive
            unreachable!()
        },

        CmdOpts { clipboard: false, stdin: false } => {
            bail!( "Wrong command line options. \
                    Run with --help to see usage." )
        },

        CmdOpts { clipboard: true, stdin: false } => {
            info!( "data source is clipboard" );
            let mut cb = arboard::Clipboard::new()?;
            cb.get_text()?
        },

        CmdOpts { clipboard: false, stdin: true } => {
            info!( "data source is stdin" );
            use std::io::{ read_to_string, stdin };
            read_to_string( stdin().lock() )?
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
    let svg = {
        // Using UUIDv7 to make files nicely sorted
        let filename = format! { "quraa:{}.svg",
            uuid::Uuid::now_v7()
        };
        let path = std::env::temp_dir().join( filename );
        std::fs::write( &path, &qrcode )?;
        path
    };

    debug!( "path of qr code file {:?}", svg );

    debug!( "showing generated Qr Code" );

    std::process::Command::new( "open" )
        .arg( &svg )
        .output()?
    ;

    Ok(())

}
