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

use std::process::{
    Command,
    ExitStatus,
};

use tracing::debug;


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
const ARCHIVE_DIR_NAME: &str = "original";

const ARCHIVE_BATCH_SIZE: usize = 250;


/// A tool for converting pictures to AVIF format lossly
/// while preserving reasonable quality.
#[ derive( clap::Parser, Debug ) ]
#[ command( max_term_width = 76 ) ]
struct CmdOpts {
    /// Disable CQ (constant quality) mode.
    #[ arg( long, short, action, default_value_t=false ) ]
    no_cq: bool,

    /// Encode in YUV444 instead of default YUV420
    #[ arg( long, short, action, default_value_t=false ) ]
    yuv444: bool,

    /// Process pictures recursively *(unimplemented)*
    #[ arg( long, short, action ) ]
    recursive: bool,

    /// Path to either a picture or a directory.
    /// For single picture the result AVIF file is placed
    /// in the same directory beside it.
    /// For directory the original file is moved to
    /// a child directory named "original".
    /// If no path is supplied then the current directory is used.
    input: Option<PathBuf>,
}


#[ derive( Debug ) ]
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
        if let Some( ext ) = path.extension() {
            let ext = ext.to_string_lossy().into_owned().to_lowercase();
            debug!( ext );
            SUPPORTED_FILE_TYPES.contains( &ext.as_str() )
        } else {
            debug!( "path doesn't have extension" );
            false
        }
    }

    #[ tracing::instrument ]
    fn avif_extension( path: &Path ) -> PathBuf {
        let mut path = path.to_owned();
        path.set_extension( "avif" );
        path
    }

}


/// Smash things together and roll it around.
#[ derive( Debug ) ]
struct App {
    pictures: Vec<Picture>,
    cmdopts: CmdOpts,
}


#[ derive( Debug, Default ) ]
struct ArchiveQueue<'queue> {
    pictures: Vec<&'queue Picture>
}

impl<'queue> ArchiveQueue<'queue> {

    fn push( &mut self, pic: &'queue Picture ) {
        self.pictures.push( pic )
    }

    fn execute( &self ) -> anyhow::Result<()> {
        use itertools::Itertools;

        debug!( "execute archiving tasks" );

        eprintln!( ":: Archive original pictures" );

        let chunks = self.pictures
            .chunks( ARCHIVE_BATCH_SIZE )
            .map( |ck| {
                ck.iter()
                    .filter( |p| p.archive )
                    .map( |p| &p.from )
                    .collect_vec()
            } )
            .collect_vec()
        ;

        debug!( "{} chunk(s)", chunks.len() );

        for ck in chunks {
            if ck.is_empty() {
                continue
            }

            debug!( "run mv" );

            let status = Command::new( "mv" )
                .arg( "-vn" )
                .args( [ "--target-directory", ARCHIVE_DIR_NAME ] )
                .arg( "--" )
                .args( ck )
                .spawn()?.wait()?
            ;

            anyhow::ensure! {
                status.success(),
                ":: Failed to archive some pictures."
            }
        };

        Ok(())
    }

}


fn main() -> anyhow::Result<()> {

    // Setup tracing

    ino_tracing::init_tracing_subscriber();


    // Setup app

    debug!( "parse cmdopts" );

    let cmdopts = <CmdOpts as clap::Parser>::parse();

    debug!( ?cmdopts );


    debug!( "collecting pictures" );

    let pictures = {

        let input = {
            let p = cmdopts.input.clone()
                .unwrap_or( std::env::current_dir()? );
            std::path::absolute( p )?
        };

        debug!( ?input );

        if input.is_file() {

            debug!( "input is picture" );

            anyhow::ensure! {
                Picture::filetype_supported( &input ),
                "\"{input:?}\" is not a supported filetype",
            };

            vec![ Picture::new( input, false ) ]

        } else if input.is_dir() {

            debug!( "input is directory" );

            debug!( ?input, "change working directory" );
            std::env::set_current_dir( &input )?;

            // N.B. This relies on the fact that the pwd is set to
            // the input directory so that the archive dir is created
            // inside it.
            debug!( "create archive directory" );
            std::fs::create_dir_all( ARCHIVE_DIR_NAME )?;

            find_files( &input )?.into_iter()
                .filter( |p| Picture::filetype_supported( p ) )
                .map( |p| Picture::new( p, true ) )
                .collect()

        } else {
            anyhow::bail!( "Input is neither a directory nor a file" )
        }
    };


    if pictures.is_empty() {
        eprintln!( ":: No pictures to process" );
        return Ok(())
    }

    tracing::trace!( ?pictures );

    let app = App { pictures, cmdopts };

    tracing::trace!( ?app );


    // Run avifenc

    debug!( "prepare to run avifenc" );

    let mut archive_queue = ArchiveQueue::default();

    for pic in &app.pictures {

        let result = encode( &app, pic )?;

        if !result.success() {
            anyhow::bail!( ":: Encoding failed" );
        }

        archive_queue.push( pic );

    }

    debug!( "post encoding tasks" );

    archive_queue.execute()?;

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


/// Encode the picture at *from*, save the result to *dest*.
#[ tracing::instrument ]
fn encode( app: &App, picture: &Picture )
    -> anyhow::Result< ExitStatus >
{

    // Trying to document things as much as possible,
    // but the whole singal processing domain is just dumpster mess.
    //
    // For some reason this configuration is the right
    // magic spell to control AV1+aom+libavif to give
    // the best results.
    let mut avifenc = Command::new( AVIFENC );

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
        .args( [ "--jobs", "all" ] )
        // Values higher than 3 ofthen add seconds to encoding
        // while saving few to none spaces, so 3.
        .args( [ "--speed", "3" ] )
        // bit-depth can be 8, 10 or 12
        // 12bit quite often saves few extra spaces than 8bit.
        // AV1 really shines at higher bitrate which means
        // Unfortunately Windows Explorer can't thumbnail
        // 12bit AVIF picture so we're stucked with 8bit for now :(
        .args( [ "--depth", "12" ] )
        // YUV is well documented everywhere.
        .args( [ "--yuv",
            if app.cmdopts.yuv444 { "444" } else { "420" }
        ] )
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

    Ok( status )

}
