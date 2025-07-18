use std::process::ExitStatus;
use std::path::Path;
use itertools::Itertools;
use tap::Pipe;
use tap::Tap;
use anyhow::Context;
use anyhow::Result as AnyResult;

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

impl crate::Transcoder for Despeckle {

    #[ inline ]
    fn supported_extension( &self, src: &str ) -> bool {
        matches!( src, "png" | "jpg" | "jpeg" )
    }

    #[ inline ]
    fn output_extension( &self ) -> &'static str {
        "png"
    }

    #[ tracing::instrument ]
    fn transcode( &self, input: &Path ) -> AnyResult<ExitStatus> {

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
