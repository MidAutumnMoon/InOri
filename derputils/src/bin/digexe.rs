use std::path::PathBuf;

use anyhow::{
    bail,
    Context,
    Result,
};


/// Find executable in $PATH and dig the root
/// to reveal its true location.
#[derive( argh::FromArgs, Debug )]
struct CmdOptions {
    /// executable to be digged
    #[argh( positional )]
    name: String
}


fn main() -> Result<()> {

    let CmdOptions { name: exe_name } = argh::from_env();


    let env_path =
        std::env::var( "PATH" )
        .context( "Failed reading $PATH" )?;

    let paths = env_path.rsplit( ':' );


    for dir in paths {

        let mut path = PathBuf::from( dir );

        path.push( &exe_name );

        let full_path = match path.canonicalize() {
            Ok( p ) => p,
            Err(_) => continue,
        };

        use derputils::fs::is_executable;

        match is_executable( &full_path ) {
            Ok( true ) => {
                println!( "{}", &full_path.display() );
                return Ok(())
            },
            _ => continue
        }

    }


    bail!( "Program \"{}\" not found in $PATH", &exe_name )

}
