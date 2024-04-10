use tracing::{
    debug,
    info,
    info_span,
};

use anyhow::{
    bail,
    Error as AnyErr,
};

use clap::Parser;


/// Generate QR Code. Hint: window can also be closed
/// by pressing Q or ESC.
#[derive( Parser, Debug, Clone, Copy )]
struct CmdOpts {
    /// use the content of clipboard as QR Code
    #[ arg( short, exclusive = true ) ]
    clipboard: bool,

    /// read stdin as Qr Code
    #[ arg( short, exclusive = true ) ]
    stdin: bool,
}


mod gui_opts {
    use sdl2::pixels::Color;

    pub const MIN_WINSIZE: usize = 8 * QR_CELL_SIZE;

    pub const QR_CELL_SIZE: usize = 8;

    pub const QR_COLOR_ON: Color = Color::BLACK;
    pub const QR_COLOR_OFF: Color = Color::WHITE;

    /// #353535 is a nice grey for background
    pub const COLOR_BG: Color = Color::RGB( 35, 35, 35 );
}


// The vast APIs of sdl2 crate use [`String`] as Err,
// making the error handling a big mess :(
fn main() -> anyhow::Result<()> {

    // Enable tracing

    ino_tracing::init_tracing_subscriber();


    // Deal with inputs

    let opts = CmdOpts::parse();

    debug!( ?opts, "command options" );


    // SDL initialization

    info!( "initialize SDL" );

    let sdl_context = sdl2::init()
        .map_err( AnyErr::msg )?;

    let video_subsystem = sdl_context.video()
        .map_err( AnyErr::msg )?;


    // Generate Qrcode

    info!( "generate Qr Code" );

    let qrcode = {
        let _span = info_span!( "qr_gen" ).entered();

        use CmdOpts as O;

        let data = match opts {
            O { clipboard: true, stdin: true } =>
                // bail!( "--clipboard and --stdin can't be specified \
                //        at the same time" ),
                // exclusive when defining clap options
                unreachable!(),

            O { clipboard: false, stdin: false } =>
                bail!( "Not enough options. Run with --help \
                       to see usage." ),

            O { clipboard: true, stdin: false } => {
                info!( "data source is clipboard" );
                info!( "use SDL to read clipboard" );
                let cb = &video_subsystem.clipboard();
                cb.clipboard_text().map_err( AnyErr::msg )?
            }

            O { clipboard: false, stdin: true } => {
                use std::io::{ self, Read };
                info!( "data source is stdin" );
                let mut buffer = String::new();
                io::stdin().lock()
                    .read_to_string( &mut buffer )?;
                buffer
            }
        };

        debug!( ?data );

        use qrcode_generator::{
            to_matrix,
            QrCodeEcc,
        };

        to_matrix( data.as_bytes(), QrCodeEcc::Medium )?
    };


    // Create a window

    let sdl_window = {
        let _span = info_span!( "make_win" ).entered();

        info!( "create window" );

        // A QR Code is guaranteed to be an square,
        // meaning the size can estimated by using only the length
        // of the outer vec, which is the total columns.
        let winsize = std::cmp::max(
            gui_opts::MIN_WINSIZE,
            qrcode.len() * gui_opts::QR_CELL_SIZE
        ).try_into()?;

        debug!( ?winsize );

        let title = format!( "QR Code {winsize}x{winsize}" );

        video_subsystem.window( &title, winsize, winsize )
            .position_centered()
            .opengl()
            .build()?
    };


    // Drawing on canvas

    let _span_draw = info_span!( "draw" );
    let _span_draw_guard = _span_draw.enter();

    info!( "prepare canvas" );

    let mut canvas = sdl_window.into_canvas().build()?;

    canvas.set_draw_color( gui_opts::COLOR_BG );
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

    info!( "draw QR Code" );

    for ( row_index, column ) in qrcode.iter().enumerate() {
        for ( col_index, cell_state ) in column.iter().enumerate() {
            canvas.set_draw_color( match *cell_state {
                true => gui_opts::QR_COLOR_ON,
                false => gui_opts::QR_COLOR_OFF,
            } );

            let cell_size = gui_opts::QR_CELL_SIZE;
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

    info!( "done drawing" );

    canvas.present();

    drop( _span_draw_guard );


    // Wait for quit

    info!( "waiting for quit event" );

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

    info!( "quit" );

    Ok(())

}
