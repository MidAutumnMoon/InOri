//! Extension methods for [`Result`]

pub trait ResultExt<OK, ERR> {
    /// Peek at the Err and print it to stderr.
    fn print_error( self ) -> Self;

    /// Print Err and exit the program, or unwrap the value if Ok.
    fn unwrap_print_error( self ) -> OK;
}

impl<OK, ERR> ResultExt<OK, ERR> for Result<OK, ERR>
where
    ERR: std::fmt::Debug
{
    #[ inline( always ) ]
    fn print_error( self ) -> Self {
        self.inspect_err( |err| eprintln!( "{err:?}" ) )
    }

    #[ inline( always ) ]
    fn unwrap_print_error( self ) -> OK {
        self
            .print_error()
            .map_err( |_| std::process::exit( 1 ) )
            .unwrap()
    }
}
