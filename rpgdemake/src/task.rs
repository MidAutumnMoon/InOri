use std::path::{
    Path,
    PathBuf,
};

use anyhow::{
    Context,
    ensure,
    bail,
};

use tracing::debug;

use crate::key::Key;


#[ derive( Debug ) ]
struct Task<S> {
    step: S,
    key: &'static Key,
}


#[ derive( Debug ) ]
struct Create {
    origin: PathBuf,
}

impl Task<Create> {
    #[ tracing::instrument( skip_all ) ]
    fn new( path: &Path, key: &'static Key ) -> Self {
        debug!( "task create" );
        Self { step: Create { origin: path.to_owned() }, key }
    }
}


#[ derive( Debug ) ]
pub struct Validate {
    origin: PathBuf,
    target: PathBuf,
}

impl TryFrom< Task<Create> > for Task<Validate> {
    type Error = anyhow::Error;

    #[ tracing::instrument( skip_all ) ]
    fn try_from( prev: Task<Create> )
        -> anyhow::Result<Self>
    {
        debug!( "task validate" );

        let Create { origin } = prev.step;

        // 1) Ensure we're working with file.
        ensure!{ origin.is_file(),
            "\"{}\" is not a file", origin.display()
        };

        // 2) The file must have sufficient amount of data,
        // and the header matches.

        Validate::validate_header( &origin )?;

        let target =
            crate::resource::Resource::fix_extension( &origin )
            // TODO: refine error message
            .ok_or_else( || anyhow::anyhow!( "Can't fix extension" ) )?
        ;

        Ok( Self {
            step: Validate { origin, target },
            key: prev.key
        } )
    }
}

impl Validate {
    #[ tracing::instrument ]
    pub fn fix_extension( origin: &Path )
        -> Option< PathBuf >
    {
        todo!()
    }

    /// Map known extensions of encrypted RPG Maker files
    /// to their normal counterparts.
    #[ tracing::instrument ]
    pub fn map_extension( input: &str )
        -> Option< &'static str >
    {
        match input {
            "rpgmvp" | "png_" => Some( "png" ),
            "rpgmvo" | "ogg_" => Some( "ogg" ),
            "rpgmvm" | "m4a_" => Some( "m4a" ),
            _ => None
        }
    }

    /// Read file and ensure it has the proper RPG Maker header.
    #[ tracing::instrument ]
    fn validate_header( file: &Path )
        -> anyhow::Result<()>
    {
        use std::io::{
            prelude::*,
            ErrorKind as IOError,
        };
        use crate::resource::{
            RPG_HEADER,
            RPG_HEADER_LEN,
            ENCRYPTION_LEN,
        };

        debug!( "open file" );
        let mut file = std::fs::File::open( file )?;

        debug!( "read file content to buffer" );
        let mut buf = [ 0; RPG_HEADER_LEN + ENCRYPTION_LEN ];

        file.read_exact( &mut buf ).map_err( |e| match e.kind() {
            IOError::UnexpectedEof =>
                anyhow::anyhow!( "Insufficient data to decode" ),
            _ => e.into(),
        } )?;

        ensure! { buf[..RPG_HEADER_LEN] == RPG_HEADER,
            "RPG Maker header mismatch"
        };

        Ok(())
    }
}

#[ derive( Debug ) ]
struct Decrypt {
    origin: PathBuf,
    target: PathBuf,
    // TODO: do it in zero copy way?
    content: Vec<u8>,
}


impl TryFrom< Task<Validate> > for Task<Decrypt> {
    type Error = anyhow::Error;

    #[ tracing::instrument( skip_all ) ]
    fn try_from( prev: Task<Validate> )
        -> anyhow::Result< Self >
    {
        debug!( "task decrypt" );

        let Validate { origin, target } = prev.step;

        let mut content = std::fs::read( &origin )?;
        let content = &mut content[crate::resource::RPG_HEADER_LEN..];

        prev.key.get()
            .iter().enumerate()
            .for_each( |( idx, b )| content[idx] ^= b )
        ;

        Ok( Self {
            step: Decrypt {
                origin, target,
                content: content.to_owned()
            },
            key: prev.key,
        } )
    }
}


#[ derive( Debug ) ]
struct Write {
    origin: PathBuf,
    target: PathBuf,
}

impl TryFrom< Task<Decrypt> > for Task<Write> {
    type Error = anyhow::Error;

    #[ tracing::instrument( skip_all ) ]
    fn try_from( prev: Task<Decrypt> )
        -> anyhow::Result< Self >
    {
        debug!( "task write" );

        let Decrypt { origin, target, content } = prev.step;

        std::fs::write( &target, &content )?;

        Ok( Self {
            step: Write { origin, target },
            key: prev.key,
        } )
    }
}


#[ derive( Debug ) ]
struct Done {
    origin: PathBuf,
    target: PathBuf,
}

impl TryFrom< Task<Write> > for Task<Done> {
    type Error = anyhow::Error;

    #[ tracing::instrument( skip_all ) ]
    fn try_from( prev: Task<Write> )
        -> anyhow::Result< Self >
    {
        let Write { origin, target } = prev.step;
        Ok( Self {
            step: Done { origin, target },
            key: prev.key
        } )
    }
}


pub struct TaskRunner;

impl TaskRunner {

    #[ tracing::instrument( skip_all ) ]
    pub fn new( paths: &[PathBuf], key: &'static Key )
        -> anyhow::Result<()>
    {
        use rayon::prelude::*;

        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads( 256 )
            .build()
            .context( "Failed to build rayon threadpool" )?
        ;

        pool.install( || {
            let total = paths.len();
            paths.into_par_iter()
                .map( |path| { Task::<Create>::new( path, key ) } )
                .map( |tk| { Task::<Validate>::try_from( tk ) } )
                .map( |tk| { tk.and_then( Task::<Decrypt>::try_from ) } )
                .map( |tk| { tk.and_then( Task::<Write>::try_from ) } )
                .map( |tk| { tk.and_then( Task::<Done>::try_from ) } )
                .enumerate()
                // TODO:
                // This losts the paths of errored tasks, which can be
                // solved by using a custom error type later on.
                .for_each( |( idx, result )| {
                    let idx = idx + 1;
                    let message = match result {
                        Ok( t ) => format!( "(ok) {:?}", t.step.target ),
                        Err( e ) => format!( "(err: {e:?})" ),
                    };
                    println!( "{idx}/{total}: {message}" )
                } )
            ;
        } );

        Ok(())
    }

}
