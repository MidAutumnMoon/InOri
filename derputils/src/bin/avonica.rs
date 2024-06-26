//! We shall all embrace AV1.
//!
//! Evovled from a script that grows out of hand,
//! which can be found at
//! gist:MidAutumnMoon/5d1d19c0de39ebc3ce2697df7daa0d77
//! if the gist is still there.
//!
//!
//! ## Random Notes
//!
//! ### CQ (Constrained Quality)
//!
//! CQ ensures that the "amount of quantization thing" won't go
//! any higher than specified value, thus depending on
//! the value given yields less compression or in another word
//! better quality than the automatically chosen one by the encoder.
//!
//! One genius thing about AV1 is that encoder is able to
//! separate the primary object from others in a scene
//! and apply different quantization scalars to each area,
//! which makes the encoding a lot better in terms of size
//! meanwhile having no "visiable loss".
//!
//! While this tool prefers quality over file size
//! and set CQ to a lower value (thus higher quality),
//! let the encoder decide what value to use can always
//! save quite a lot spaces, so there is also a switch
//! to disable CQ in this tool.


use std::path::{
    PathBuf,
    Path
};

use clap::Parser;

use tracing::debug;


/// Number of avifenc to run at the same time.
///
/// A big chunck of encoding time avifenc spent
/// is doing something single-threaded, likely decoding the
/// input picture, so multiple avifenc instances are launched
/// to minimize the CPU waste.
///
/// However the actual encoding is CPU intensive.
const AVIFENC_INSTANCES: usize = 2;


/// Path to avifenc executable.
/// Default is simply "avifenc" to use whatever found in $PATH.
/// Primarily for nix packaging.
const AVIFENC: &str = const {
    match std::option_env!( "CFG_AVIFENC" ) {
        Some( a ) => a,
        None => "avifenc",
    }
};


/// File types that avifenc supports as input
const SUPPORTED_FILE_TYPES: [ &str; 4 ] = [
    "jpg", "jpeg", "png", "y4m"
];


/// Name of the directory to archive original pictures.
const ARCHIVE_DIR: &str = "original";


/// A tool for converting pictures to AVIF format lossly
/// while preserving reasonable quality.
#[ derive( Parser, Debug ) ]
#[ command( max_term_width = 76 ) ]
struct CmdOpts {
    /// Disable CQ (constant quality) mode.
    #[ arg( long, short, action, default_value_t=false ) ]
    no_cq: bool,

    /// Process pictures recursively *(unimplemented)*
    #[ arg( long, short, action ) ]
    recursive: bool,

    /// Path to either a single picture or a directory of pictures.
    /// For single picture the result AVIF file is placed
    /// in the same directory with it.
    /// For directory the original file is moved to
    /// a child directory named "original".
    the_thing: Option<PathBuf>,
}


#[ derive( Debug, Clone ) ]
struct Picture {
    from: PathBuf,
    dest: PathBuf,
    archive: bool,
}

impl Picture {
    fn new( from: PathBuf, archive: bool ) -> Self {
        Self {
            dest: Self::avif_extension( &from ),
            from, archive,
        }
    }

    #[ tracing::instrument ]
    fn filetype_supported( path: &Path ) -> bool {
        match path.extension() {
            Some( ext ) => {
                // Few but certain encounters that
                // the extension is in all capital.
                let ext = ext
                    .to_string_lossy()
                    .into_owned()
                    .to_lowercase();
                SUPPORTED_FILE_TYPES.contains( &ext.as_str() )
            },
            None => false,
        }
    }

    #[ tracing::instrument ]
    fn avif_extension( path: &Path ) -> PathBuf {
        let mut path = path.to_owned();
        path.set_extension( "avif" );
        path
    }

    fn do_archive( &self )
        -> anyhow::Result< std::process::ExitStatus >
    {
        let status = std::process::Command::new( "mv" )
            .arg( "-vn" ).arg( "--" )
            .args( [ &self.from, Path::new( ARCHIVE_DIR ) ] )
            .spawn()?
            .wait()?
        ;
        Ok( status )
    }
}


#[ derive( Debug ) ]
enum Mode {
    File( PathBuf ),
    Dir( PathBuf ),
}


/// Smash things together and roll it around.
#[ derive( Debug ) ]
struct App {
    mode: Mode,
    pictures: Vec<Picture>,
    cmdopts: CmdOpts,
    avifenc_jobs: usize,
}


fn main() -> anyhow::Result<()> {

    // Setup tracing

    ino_tracing::init_tracing_subscriber();


    // Setup app

    debug!( "parse cmdopts" );

    let cmdopts = CmdOpts::parse();

    debug!( ?cmdopts );


    debug!( "determine mode of app" );

    let mode = {
        let path = cmdopts.the_thing
            .clone()
            .unwrap_or( std::env::current_dir()? );
        debug!( ?path, "path to work with" );
        if path.is_file() {
            Mode::File( path )
        } else if path.is_dir() {
            Mode::Dir( path )
        } else {
            anyhow::bail!( "The thing is neither a directory \
                nor file" )
        }
    };


    debug!( "collecting pictures" );

    let pictures = match &mode {
        Mode::File( p ) => {
            anyhow::ensure! { Picture::filetype_supported( p ),
                "\"{}\" is not a supported filetype",
                p.display()
            };
            vec![ Picture::new( p.to_owned(), false ) ]
        },
        Mode::Dir( p ) => {
            find_files( p )?.into_iter()
                .filter( |p| Picture::filetype_supported( p ) )
                .map( |p| Picture::new( p, true ) )
                .collect()
        },
    };

    tracing::trace!( ?pictures );

    let avifenc_jobs = {
        let cores = std::thread::available_parallelism()?.get();
        match &mode {
            Mode::File(_) => cores,
            Mode::Dir(_) => cores.div_ceil( AVIFENC_INSTANCES )
        }
    };


    let app = App { mode, pictures, cmdopts, avifenc_jobs };

    debug!( "app made" );

    tracing::trace!( ?app );


    // Run avifenc

    debug!( "prepare to run avifenc" );

    if let Mode::Dir( p ) = &app.mode {
        debug!( ?p, "change working directory" );
        std::env::set_current_dir( p )?;
        debug!( "create archive directory" );
        std::fs::create_dir_all( ARCHIVE_DIR )?;
    }


    std::thread::scope( |scope| {
        use std::sync::Arc;
        use itertools::Itertools;

        let app = Arc::new( app );

        'cks: for pictures in app.clone()
            .pictures
            .chunks( AVIFENC_INSTANCES )
        {
            let handles = pictures.iter()
                .map( |pic| {
                    let app = app.clone();
                    let pic = pic.to_owned();
                    eprintln!( ":: Encoding {}", &pic.from.display() );
                    scope.spawn( move || encode( &app, pic ) )
                } )
                .collect_vec();

            for hdl in handles {
                let enc_res = match hdl.join() {
                    Ok( r ) => match r {
                        Ok( re ) => re,
                        Err( e ) => {
                            eprintln!( "{e:?}" ); break 'cks
                        }
                    },
                    Err(_) => break 'cks,
                };
                let EncodeResult { status, picture } = enc_res;
                if !status.success() {
                    eprintln!( ":: Encoding failed" );
                    break 'cks
                }
                if picture.archive {
                    eprintln!( ":: Archive original picture" );
                    let achv_status = match picture.do_archive() {
                        Ok( re ) => re,
                        Err( e ) => {
                            eprintln!( "{e:?}" );
                            break 'cks
                        }
                    };
                    if !achv_status.success() {
                        eprintln!( ":: Archive original picture failed" );
                        break 'cks
                    }
                }
                eprintln!()
            }
        }
    } );


    Ok(())

}


#[ tracing::instrument ]
fn find_files( dir: &Path )
    -> anyhow::Result< Vec<PathBuf> >
{
    debug!( "collect pictures from dir" );
    let files = std::fs::read_dir( dir )?
        .collect::< Result< Vec<_>, _> >()?
        .into_iter()
        .map( |entry| entry.path() )
        .filter( |path| path.is_file() )
        .collect()
    ;
    Ok( files )
}


struct EncodeResult {
    status: std::process::ExitStatus,
    picture: Picture,
}


/// Encode the picture at *from*, save the result to *dest*.
#[ tracing::instrument ]
fn encode( app: &App, picture: Picture )
    -> anyhow::Result< EncodeResult >
{

    // Trying to document things as much as possible,
    // but the whole singal processing domain is just dumpster mess.
    //
    // For some reason this configuration is the right
    // magic spell to control AV1+aom+libavif to give
    // the best results.
    let mut avifenc = std::process::Command::new( AVIFENC );

    let avifenc = avifenc
        // min/max level of quantization,
        // which means how messed up things can be.
        // 0-63 permits the encoder to choose any level
        // it likes to yield the best result.
        .args( [ "--min", "0" ] )
        .args( [ "--max", "63" ] )
        // same thing but for alpha channel
        .args( [ "--minalpha", "0" ] )
        .args( [ "--maxalpha", "63" ] )
        // avifenc is able to utilize multithread.
        .args( [ "--jobs", &app.avifenc_jobs.to_string() ] )
        // Values higher than 3 ofthen add seconds to encoding
        // while saving few to none spaces, so 3.
        .args( [ "--speed", "3" ] )
        // bit-depth can be 8, 10 or 12
        // AV1 really shines at higher bitrate which means
        // 12bit quite often saves few extra spaces than 8bit.
        // Unfortunately Windows Explorer can't thumbnail
        // 12bit AVIF picture so we're stucked with 8bit for now :(
        .args( [ "--depth", "8" ] )
        // YUV is well documented everywhere.
        // Note: AOM denoise only works with YUV420
        .args( [ "--yuv", "420" ] )
        // How AVIF converts colors between color spaces.
        // A headache topic that the author is not quite
        // understanding.
        // The stock avifenc(aom) config "1/13/6" loses colors
        // here and there, where this setup keeps the color
        // look like exactly the same (although HDR is lost).
        .args( [ "--cicp", "1/13/1" ] )
        // Don't include original EXIF or XMP
        // in the result AVIF picture.
        // (NOTE: ICC profile is preserved)
        .args( [ "--ignore-exif", "--ignore-xmp" ] )
        // Not quite understanding.
        // Some way to handle alpha channel that gives
        // higher alpha quality.
        .args( [ "--premultiply" ] )
        // Tiling divides picture into chuncks which let the encoder
        // better take advantage of multithread.
        // Autotiling let the encoder choose the number chuncks
        // based on the input picture dimension.
        .args( [ "--autotiling" ] )
        // Better RGB to YUV convertion
        .args( [ "--sharpyuv" ] )
        // All the tunes and tweaks only applies to AOM encoder
        .args( [ "--codec", "aom" ] )
        // TODO: document these things.
        // NOTE: Seems complicate but they're actually
        // pretty straight forward, although it's hard
        // to put it into texts.
        .args( [ "-a", "color:sharpness=2" ] )
        .args( [ "-a", "color:deltaq-mode=3" ] )
        .args( [ "-a", "color:enable-chroma-deltaq=1" ] )
        .args( [ "-a", "end-usage=q" ] )
        .args( [ "-a", "enable-qm=1" ] )
        .args( [ "-a", "color:qm-min=0" ] )
        .args( [ "-a", "color:enable-dnl-denoising=0" ] )
        .args( [ "-a", "color:denoise-noise-level=10" ] )
        .args( [ "-a", "tune=ssim" ] )
    ;

    if !app.cmdopts.no_cq {
        avifenc.args( [ "-a", "cq-level=18" ] );
    }

    let status = avifenc
        .arg( "--" )
        .args( [ &picture.from, &picture.dest ] )
        .spawn()?.wait()?;

    Ok( EncodeResult { status, picture } )
}
