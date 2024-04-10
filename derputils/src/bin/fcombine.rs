use std::{
    path::PathBuf,
    fs::File,
};

use anyhow::{
    ensure,
    Context
};

use tracing::debug;

use clap::Parser;


/// Dump contents of <input_files> into <output>
/// without a shell.
#[ derive( Parser, Debug ) ]
struct CmdOpts {
    /// file to write into, can't be an existsing one
    #[ arg() ]
    output: PathBuf,

    /// source of read contents
    #[ arg() ]
    input_files: Vec<PathBuf>
}


fn main() -> anyhow::Result<()> {

    // Init tracing

    ino_tracing::init_tracing_subscriber();


    // Acquire command line options.

    let CmdOpts { output, input_files } = CmdOpts::parse();

    debug!( ?output, ?input_files );

    // Avoid accidents

    ensure! { ! output.try_exists()?,
        "\"{}\" already exists",
        output.display()
    }


    // Do the IO works

    debug!( "Create output file" );

    let mut output = File::create( output )
        .context( "Failed creating output file" )?;

    for file in input_files {

        debug!( ?file, "Read input file" );

        let mut input = File::open( &file )
            .with_context( || format!(
                "Failed reading \"{}\"", &file.display()
            ) )?;

        std::io::copy( &mut input, &mut output )
            .with_context( || format!(
                "Error to copy \"{}\"", &file.display()
            ) )?;

    }

    Ok(())

}
