use tracing::debug;
use tap::Tap;

use crate::StaticStrs;

/// Path to the "cjxl" executable.
const CJXL_PATH: Option<&str> = std::option_env!( "CFG_CJXL_PATH" );

#[ derive( Debug ) ]
pub struct Jxl;

impl crate::Transcoder for Jxl {

    #[ inline ]
    fn input_extensions( &self ) -> StaticStrs {
        &[ "png", "jpg", "jpeg", "apng", "gif" ]
    }

    #[ inline ]
    fn output_extension( &self ) -> &'static str {
        "jxl"
    }

    /// Jpeg XL has a suprior lossless encoding algorithm which also
    /// doesn't need too much tweaking. These options are used for squashing
    /// out more savings on spaces.
    fn transcode( &self, input: &std::path::Path )
        -> anyhow::Result< std::process::ExitStatus >
    {
        debug!( "encode using cjxl" );

        let mut cjxl = std::process::Command::new(
            CJXL_PATH.unwrap_or( "cjxl" )
        );

        let output = input.to_owned()
            .tap_mut( |s| { s.set_extension( "jxl" ); } );

        let cjxl = cjxl
            // Allow tweaking more parameters.
            .arg( "--allow_expert_options" )
            // Increase the encoding time A LOT (30s in e9 comparing to few secs
            // in default) but also saves a lot more spaces.
            .args([ "--effort", "9" ])
            // Following 3 options force cjxl to the lossless algorithm
            // called modular, lossly speakig.
            .args([ "--modular", "1" ])
            .args([ "--lossless_jpeg", "1" ])
            .args([ "--distance", "0.0" ])
            // Brotli level
            .args([ "--brotli_effort", "11" ])
            // Premultiply alpha
            .args([ "--premultiply", "1" ])
            // Controls the generation of some internal tree thing.
            // The bigger the memory it uses, but also save more spaces.
            .args([ "--iterations", "100" ])
            // Tweak the modular algorithm to save even more spaces.
            .args([ "--modular_nb_prev_channels", "6" ])
            .args([ "--modular_group_size", "2" ])
            .args([ "--modular_predictor", "15" ])
            // Use all threads
            .args([ "--num_threads", "-1" ])
        ;

        let status = cjxl
            .args([ input, &output ])
            .spawn()?.wait()?
        ;

        Ok( status )
    }
}
