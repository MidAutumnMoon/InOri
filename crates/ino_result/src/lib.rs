//! Extension methods for [`Result`]

pub trait ResultExt<T, E> {
    /// Print Err to stderr and exit the program, or unwrap if Ok.
    fn unwrap_print_error( self ) -> T;
}

impl<T, E> ResultExt<T, E> for Result<T, E>
where
    E: std::fmt::Debug
{
    #[ inline ]
    fn unwrap_print_error( self ) -> T {
        match self {
            Err( e ) => {
                eprintln!( "{e:?}" );
                std::process::exit( 1 );
            }
            Ok( o ) => o,
        }
    }
}
