//! Extension methods for [`Result`]

pub trait ResultExt<T, E> {
    /// Peek at the Err and print it to stderr.
    #[ must_use ]
    fn print_error( self ) -> Self;

    /// Print Err and exit the program, or unwrap the value if Ok.
    fn print_error_exit_process( self ) -> T;
}

impl<T, E> ResultExt<T, E> for Result<T, E>
where
    E: std::fmt::Debug
{
    #[ inline ]
    fn print_error( self ) -> Self {
        self.inspect_err( |err| eprintln!( "{err:?}" ) )
    }

    #[ inline ]
    fn print_error_exit_process( self ) -> T {
        self
            .print_error()
            .map_err( |_| std::process::exit( 1 ) )
            .unwrap()
    }
}
