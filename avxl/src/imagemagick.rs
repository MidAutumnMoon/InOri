use std::process::ExitStatus;
use std::path::Path;
use itertools::Itertools;
use tap::Pipe;
use tracing::debug;
use tap::Tap;
use anyhow::Context;

/// Path to the "avifenc" executable.
const MAGICK_PATH: Option<&str> = std::option_env!( "CFG_MAGICK_PATH" );

#[ derive( Debug ) ]
pub struct Despeckle {
    pub iteration: usize,
}

impl Default for Despeckle {
    fn default() -> Self {
        Self { iteration: 1 }
    }
}

impl crate::Encoder for Despeckle {

    #[ inline ]
    fn is_ext_supported( &self, input: &str ) -> bool {
        [ "png", "jpg", "jpeg" ].contains( &input )
    }

    /// Encode the input to AVIF using avifenc.
    ///
    /// Try to document parameters as much as possible,
    /// but the whole singal processing domain is just dumpster mess.
    ///
    /// Like, for some reason, this configuration is
    /// the right magic spell to command AV1+aom+libavif to give
    /// the best results.
    #[ tracing::instrument ]
    fn perform_encode( &self, input: &Path ) -> anyhow::Result<ExitStatus>
    {

        let number_of_depseckles =
            std::iter::repeat_n( "-despeckle", self.iteration )
            .collect_vec()
        ;

        let mut magick = MAGICK_PATH.unwrap_or( "magick" )
            .pipe( std::process::Command::new );

        // TODO: we also accepts .png, this will cause conflict
        let output = input.to_owned()
            .tap_mut( |it| { it.set_extension( "png" ); } );

        let status = magick
            .arg( "-verbose" )
            .arg( "--" )
            .arg( input )
            .args( number_of_depseckles )
            .arg( output )
            .spawn()
            .context( "Failed to run magick" )?
            .wait()?;

        Ok( status )
    }

}
