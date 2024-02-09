use sdl2::pixels::Color;

use tracing::debug;

use anyhow::{
    bail,
    Error as AnyErr,
};


/// Generate QR Code. Hint: window can also be closed
/// by pressing Q or ESC.
#[derive( argh::FromArgs, Debug, Clone, Copy )]
struct CmdOpts {
    /// use the content of clipboard as QR Code
    #[argh( switch, short = 'c' )]
    clipboard: bool,

    /// read stdin as Qr Code
    #[argh( switch, short = 's' )]
    stdin: bool,
}


struct GuiOpts;

impl GuiOpts {
    const QRCODE_CELL_SIZE: usize = 8;

    const MINIMUM_WINDOW_SIZE: usize =
        8 * Self::QRCODE_CELL_SIZE;

    const COLOR_QR_ON: Color = Color::BLACK;
    const COLOR_QR_OFF: Color = Color::WHITE;

    /// #353535 is a nice grey for background
    const COLOR_BG: Color = Color::RGB( 35, 35, 35 );
}


// The vast APIs of sdl2 crate use [`String`] as Err,
// making the error handling a big mess :(
fn main() -> anyhow::Result<()> {

    // Enable tracing

    ino_tracing::init_tracing_subscriber();


    // Deal with inputs

    let opts: CmdOpts = argh::from_env();

    debug!( ?opts, "Command line options" );


    // SDL initialization

    debug!( "Initialize SDL" );

    let sdl_context = sdl2::init()
        .map_err( AnyErr::msg )?;

    let video_subsystem = sdl_context.video()
        .map_err( AnyErr::msg )?;


    // Generate Qrcode

    debug!( "Convert input data into QR Code" );

    let qrcode = {
        use CmdOpts as O;

        let data = match opts {
            O { clipboard: true, stdin: true } =>
                bail!( "--clipboard and --stdin can't be specified \
                       at the same time" ),

            O { clipboard: false, stdin: false } =>
                bail!( "Not enough options. Run with --help \
                       to see usage." ),

            O { clipboard: true, stdin: false } => {
                debug!( "Source is clipboard" );
                debug!( "Use SDL to read clipboard" );
                let cb = &video_subsystem.clipboard();
                cb.clipboard_text().map_err( AnyErr::msg )?
            }

            O { clipboard: false, stdin: true } => {
                debug!( "Source is stdin" );
                use std::io::{ self, Read };
                let mut buffer = String::new();
                io::stdin().lock()
                    .read_to_string( &mut buffer )?;
                buffer
            }
        };

        debug!( ?data, "Input data" );

        use qrcode_generator::{
            to_matrix,
            QrCodeEcc,
        };

        to_matrix( data.as_bytes(), QrCodeEcc::Medium )?
    };


    // Create a window

    debug!( "Create window" );

    let sdl_window = {
        // A QR Code is guaranteed to be an square,
        // meaning the size can estimated by using only the length
        // of the outer vec, which is the total columns.
        let winsize = std::cmp::max(
            GuiOpts::MINIMUM_WINDOW_SIZE,
            qrcode.len() * GuiOpts::QRCODE_CELL_SIZE
        ).try_into()?;

        let title = format!( "QR Code {winsize}x{winsize}" );

        video_subsystem.window( &title, winsize, winsize )
            .position_centered()
            .opengl()
            .build()?
    };


    // Drawing on canvas

    debug!( "Prepare canvase to draw QR Code" );

    let mut canvas = sdl_window.into_canvas().build()?;

    canvas.set_draw_color( GuiOpts::COLOR_BG );
    canvas.clear();
    canvas.present();

    // |-----------------------Noice----------------------|
    // |                          |                       |
    // |                        y | row_index * CELL_SIZE |
    // |                          |                       |
    // |  col_index * CELL_SIZE   |                       |
    // |--------------------------````                    |
    // |             x            ```` CELL_SIZE          |
    // |                          ````                    |
    // |                                                  |
    // |--------------------------------------------------|

    debug!( "Draw QR Code" );

    for ( row_index, column ) in qrcode.iter().enumerate() {
        for ( col_index, cell_state ) in column.iter().enumerate() {
            canvas.set_draw_color( match *cell_state {
                true => GuiOpts::COLOR_QR_ON,
                false => GuiOpts::COLOR_QR_OFF,
            } );

            let cell_size = GuiOpts::QRCODE_CELL_SIZE;
            let x = col_index * cell_size;
            let y = row_index * cell_size;

            let rect = sdl2::rect::Rect::new(
                x.try_into()?,
                y.try_into()?,
                cell_size.try_into()?,
                cell_size.try_into()?
            );

            canvas.fill_rect( rect ).map_err( AnyErr::msg )?;
        }
    }

    debug!( "QR Code drawn, show window" );

    canvas.present();


    // Wait for quit

    debug!( "Waiting for quit event" );

    let mut event_pump = sdl_context.event_pump()
        .map_err( AnyErr::msg )?;

    for event in event_pump.wait_iter() {
        use sdl2::event::Event as E;
        use sdl2::keyboard::Keycode as K;
        match event {
            E::Quit { .. } => break,
            E::KeyDown { keycode: Some(K::Q | K::Escape), .. } => break,
            _ => continue
        }
    }

    Ok(())

}
