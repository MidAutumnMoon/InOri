use std::process::ExitStatus;
use std::path::Path;
use tracing::debug;
use tap::Tap;


/// Path to the "avifenc" executable.
const AVIFENC_PATH: Option<&str> = std::option_env!( "CFG_AVIFENC_PATH" );


#[ derive( Debug ) ]
pub struct Avif {
    pub no_cq: bool,
    pub yuv444: bool,
    pub cq_level: u8,
}

impl Default for Avif {
    fn default() -> Self {
        Self {
            no_cq: false, yuv444: false,
            cq_level: 20,
        }
    }
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

        let mut avifenc = std::process::Command::new(
            AVIFENC_PATH.unwrap_or( "avifenc" )
        );

        let output = input.to_owned()
            .tap_mut( |s| { s.set_extension( "avif" ); } );

        let avifenc = avifenc
            // All following arguments are tuned for AOM encodoer
            .args([ "--codec", "aom" ])
            // The maxium/minium amount of quantization.
            // 0..63 permits the encoder to use any value
            // it considered to be proper.
            .args([ "--min", "0", "--max", "63" ])
            .args([ "--minalpha", "0", "--maxalpha", "63" ])
            // Let it use all cores.
            .args([ "--jobs", "all" ])
            // Effects the size of output.
            // However speed < 3 increases the encoding time
            // considerably and has no almost no gain.
            .args([ "--speed", "3" ])
            // AVIF can save extra, and normally a lot, spaces
            // at higher bit depth.
            .args([ "--depth", "12" ])
            // Better alpha handling
            .args([ "--premultiply" ])
            // Let the encodoer tile the input automatically.
            // Speedup encoding.
            .args([ "--autotiling" ])
            // Better RGB-YUV processing
            .args([ "--sharpyuv" ])
            // No need to document this.
            // The reason of giving a switch to change YUV
            // is that Yuv444 takes extra spaces but does have benefits
            // of having better details on color pictures.
            .args([ "--yuv", if self.yuv444 { "444" } else { "420" } ])
            .args([ "--cicp", "1/13/1" ])
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

        let status = avifenc.arg( "--" )
            .args([ input, &output ])
            .spawn()?
            .wait()?
        ;

        Ok( status )

    }

}
