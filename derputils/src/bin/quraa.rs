use sdl2::{
    event::Event,
    keyboard::Keycode,
    pixels::Color,
    rect::Rect,
    render::WindowCanvas
};

use qrcode_generator::{
    self as qr_gen,
    QrCodeEcc,
    QRCodeError,
};


const PROGRAM_NAME: &str = "quraa";

/// Guards the minimum window size in case calculation went wrong
const MINIMUM_WINDOW_SIZE: usize = 128;


struct Colorscheme;

impl Colorscheme {
    const ON: Color = Color::BLACK;
    const OFF: Color = Color::WHITE;
    /// #353535 a nice grey for background
    const BG: Color = Color::RGB( 35, 35, 35 );
}


/// Generate QrCode
#[derive( argh::FromArgs, Debug )]
struct CmdOptions {
    /// read contents of clipboard as Qr Code payload (default)
    #[argh( switch, short = 'c' )]
    clipboard: bool,

    /// read stdin as Qr Code payload
    #[argh( switch, short = 's' )]
    stdin: bool,
}

#[derive( Debug )]
enum Source {
    Stdin,
    Clipboard,
}

impl CmdOptions {
    fn source( &self ) -> Result<Source, String> {
        match self {
            Self { clipboard: true, stdin: true, } => Err(
                "Clipboard and Stdin are mutually exclusive".into()
            ),
            Self { clipboard: true, .. } => Ok( Source::Clipboard ),
            Self { stdin: true, .. } => Ok( Source::Stdin ),
            _ => Ok( Source::Clipboard ),
        }
    }
}


#[derive( Debug )]
struct QrCode {
    matrix: Vec<Vec<bool>>
}

impl QrCode {
    /// The size in pixels each dot of a qrcode takes.
    /// (dot: the little square which is either black or white)
    const DOT_SIZE: usize = 8;

    fn from_data<A>( data: A ) -> Result<Self, QRCodeError>
        where A: AsRef<[u8]>
    {
        let matrix = qr_gen::to_matrix( data, QrCodeEcc::Low )?;
        Ok( Self { matrix } )
    }

    /// Estimate the size in pixels of this qrcode.
    /// Since QrCode is guaranteed to be a square,
    /// using the length of one vector sure is enough.
    fn estimate_pixels( &self ) -> usize {
        self.matrix.len() * Self::DOT_SIZE
    }
}


// Unfortunately vast sdl2 apis use String as Err,
// causing Result<_, String> creeping all over the place.
fn main() -> Result<(), String> {

    //
    // Deal with inputs
    //

    let cmd_opts: CmdOptions = argh::from_env();


    //
    // SDL initialization
    //

    let sdl_context = sdl2::init()?;
    let video_subsystem = sdl_context.video()?;


    //
    // Generate Qrcode
    //

    let qrcode = {
        let data = match cmd_opts.source()? {
            Source::Stdin => {
                dbg!( "Source is stdin" );
                qrutil::stdin_string().map_err( |e| e.to_string() )?
            }
            Source::Clipboard => {
                dbg!( "Source is clipboard" );
                let cb = &video_subsystem.clipboard();
                cb.clipboard_text()?
            }
        };

        QrCode::from_data( data )
            .map_err( |e| e.to_string() )?
    };


    let window_size = {
        let estimated = qrcode.estimate_pixels();
        if estimated < MINIMUM_WINDOW_SIZE {
            MINIMUM_WINDOW_SIZE
        } else {
            estimated
        }
    };

    let window = video_subsystem
        .window(
            PROGRAM_NAME,
            window_size as u32,
            window_size as u32,
        )
        .position_centered()
        .opengl()
        .build()
        .map_err( |e| e.to_string() )?;


    //
    // Drawing on canvas
    //

    let mut canvas = window
        .into_canvas()
        .build()
        .map_err( |e| e.to_string() )?;

    canvas.set_draw_color( Colorscheme::BG );
    canvas.clear();

    // |-----------------------Window---------------------|
    // |                          |                       |
    // |                        y | row_index * DOT_SIZE  |
    // |                          |                       |
    // |  col_index * DOT_SIZE    |                       |
    // |--------------------------````                    |
    // |             x            ```` DOT_SIZE           |
    // |                          ````                    |
    // |                                                  |
    // |--------------------------------------------------|
    //
    // Here ought be bugs, but ECC can do the heavy lifting.
    fn draw_rect(
        canvas: &mut WindowCanvas,
        row_index: usize,
        col_index: usize
    ) -> Result<(), String>
    {
        canvas.fill_rect( Rect::new(
            ( col_index * QrCode::DOT_SIZE ) as i32,
            ( row_index * QrCode::DOT_SIZE ) as i32,
            QrCode::DOT_SIZE as u32,
            QrCode::DOT_SIZE as u32,
        ) )?;
        Ok(())
    }

    for ( row_index, column ) in qrcode.matrix.iter().enumerate() {
        for ( col_index, dot ) in column.iter().enumerate() {
            canvas.set_draw_color(
                if *dot { Colorscheme::ON } else { Colorscheme::OFF }
            );
            draw_rect( &mut canvas, row_index, col_index )?;
        }
    }

    canvas.present();


    //
    // Wait for quit
    //

    let mut event_pump = sdl_context.event_pump()?;

    // No need for redrawing once the qrcode is renderred
    for event in event_pump.wait_iter() {
        match event {
            Event::Quit { .. } => break,
            Event::KeyDown {
                keycode: Some( Keycode::Q | Keycode::Escape ), ..
            } => break,
            _ => continue
        }
    }


    Ok(())

}


mod qrutil {

    use std::io::{
        self,
        Read,
    };

    pub fn stdin_string()
        -> io::Result<String>
    {
        let mut buffer = String::new();
        io::stdin().lock().read_to_string( &mut buffer )?;
        Ok( buffer )
    }

}
