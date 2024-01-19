use std::path::PathBuf;

use anyhow::{
    bail,
    Result,
    Context
};


/// Dump contents of all "input" into "output" without
/// piping through a shell.
#[derive( argh::FromArgs, Debug )]
struct CmdOptions {
    /// a place to dump contents into
    #[argh( positional )]
    output: PathBuf,

    /// whose contents will be combined
    #[argh( positional )]
    files: Vec<PathBuf>
}


fn main() -> Result<()> {

    // Acquire command line options.

    let CmdOptions { output, files } = argh::from_env();


    // Avoid accidents

    if output.try_exists()? {
        bail!(
            "\"{}\" already exists",
            output.display()
        )
    }


    // Do the IO works

    use std::fs::File;

    let mut output = {
        let message = || format! {
            "Failed writing \"{}\"",
            &output.display()
        };
        File::create( &output ).with_context( message )?
    };


    for file in files {

        let mut input = {
            let message = || format! {
                "Failed writing \"{}\"",
                &file.display()
            };
            File::open( &file ).with_context( message )?
        };

        std::io::copy( &mut input, &mut output )
            .with_context( || format!{ "Error copying \"{}\"", &file.display() } )?;

    }


    Ok( () )

}
