use std::process::ExitStatus;
use std::path::Path;
use tap::Pipe;
use tracing::debug;
use tap::Tap;
use anyhow::Context;

/// Path to the "avifenc" executable.
const AVIFENC_PATH: Option<&str> = std::option_env!( "CFG_AVIFENC_PATH" );

#[ derive( Debug ) ]
pub struct Avif {
    pub no_cq: bool,
    pub cq_level: u8,
    pub quality_preset: QualityPreset,
}

impl Default for Avif {
    fn default() -> Self {
        Self {
            no_cq: false,
            cq_level: 20,
            quality_preset: QualityPreset::Medium,
        }
    }
}

#[ derive( Debug, Clone, clap::ValueEnum ) ]
pub enum QualityPreset {
    Low,
    Medium,
    High,
}

impl crate::Encoder for Avif {

    #[ inline ]
    fn is_ext_supported( &self, input: &str ) -> bool {
        [ "png", "jpg", "jpeg", "y4m" ].contains( &input )
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
    fn perform_encode( &self, input: &Path )
        -> anyhow::Result<ExitStatus>
    {

        debug!( "encoding using avifenc" );

        let mut avifenc = AVIFENC_PATH.unwrap_or( "avifenc" )
            .pipe( std::process::Command::new );

        let avifenc = {
            let quality = match self.quality_preset {
                QualityPreset::Low => "27",
                QualityPreset::Medium => "47",
                QualityPreset::High => "77",
            };
            avifenc.args( [
                "--qcolor", quality,
                "--qalpha", quality,
            ] )
        };

        let avifenc = avifenc
            // All following arguments are tuned for AOM encodoer
            .args([ "--codec", "aom" ])
            // Let it use all cores.
            .args([ "--jobs", "all" ])
            // Effects the size of output.
            // However speed < 3 increases the encoding time
            // considerably and has no almost no gain.
            .args([ "--speed", "4" ])
            // AVIF can save extra, and normally a lot, spaces
            // at higher bit depth.
            .args([ "--depth", "12" ])
            .arg( "--premultiply" )
            .arg( "--autotiling" )
            // Better RGB-YUV processing
            .arg( "--sharpyuv" )
            .args([ "--yuv", "420" ])
            .args([ "--cicp", "1/13/1" ])
            .arg( "--ignore-icc" )
            .arg( "--ignore-exif" )
            // Advanced options.
            // This poke into the heart of AOM encoder,
            // which effects the output every so slightly.
            .args([ "-a", "color:sharpness=2" ])
            .args([ "-a", "color:deltaq-mode=3" ])
            .args([ "-a", "color:enable-chroma-deltaq=1" ])
            .args([ "-a", "end-usage=q" ])
            .args([ "-a", "enable-qm=1" ])
            .args([ "-a", "color:qm-min=0" ])
            .args([ "-a", "color:enable-dnl-denoising=0" ])
            .args([ "-a", "color:denoise-noise-level=10" ])
            .args([ "-a", "tune=ssim" ])
        ;

        // if "no_cq" is *not* set, then cq is needed.
        if !self.no_cq {
            // This is the meat for avifenc to yield
            // high quality pictures at the cost of spaces.
            //
            // By default avifenc uses some sort of variable
            // CQ which greatly saves spaces but also losts
            // a lot of details.
            //
            // The lower this value is, the fewer quantization
            // will be applied, which means higher details.
            //
            // "18" is choosen base on authos' experience
            // and expectation.
            let cq_level = format!( "cq-level={}", self.cq_level );
            avifenc.args([ "-a", &cq_level ]);
        }

        let output = input.to_owned()
            .tap_mut( |it| { it.set_extension( "avif" ); } );

        let status = avifenc.arg( "--" )
            .args([ input, &output ])
            .spawn()
            .context( "Failed to spawn avifenc" )?
            .wait()?
        ;

        Ok( status )

    }

}
