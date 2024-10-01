/// Path to the "avifenc" executable.
const CJXL_PATH: Option<&str> = std::option_env!( "CFG_CJXL_PATH" );

#[ derive( Debug ) ]
pub struct Jxl;

impl crate::Encoder for Jxl {

    #[ inline ]
    fn is_ext_supported( &self, input: &str ) -> bool {
        [ "jpg", "jpeg", "png", "gif" ].contains( &input )
    }

    fn perform_encode( &self, input: &std::path::Path )
        -> anyhow::Result< std::process::ExitStatus >
    {
        todo!()
    }
}
