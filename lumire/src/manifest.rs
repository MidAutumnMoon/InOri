use std::path::Path;
use std::path::PathBuf;

use anyhow::ensure;
use anyhow::Context;
use anyhow::Result as AnyResult;
use serde::Deserialize;
use tap::Pipe;
use tap::Tap;
use tracing::debug;
use tracing::trace;

use crate::template::RenderedPath;

const CURRENT_MANIFEST_VERSION: usize = 1;

#[ derive( Deserialize, Debug ) ]
pub struct Manifest {
    version: usize,
    symlinks: Vec<Symlink>,
}

#[ derive( Deserialize, Debug ) ]
pub struct Symlink {
    src: RenderedPath,
    dst: RenderedPath,
    mode: String,
}

impl Manifest {
    #[ tracing::instrument ]
    pub fn from_file( path: &Path ) -> AnyResult<Self> {
        debug!( "read manifest from file" );
        ensure! { path.is_file(),
            r#"Manifest path "{}" does not point to a file"#, path.display()
        };
        std::fs::read_to_string( path )
            .context( "Failed to read manifest file" )?
            .pipe_deref( Self::from_str )?
            .pipe( Ok )
    }

    #[ tracing::instrument( skip_all ) ]
    fn from_str( text: &str ) -> AnyResult<Self> {
        debug!( "Parse manifest" );
        let manifest = serde_json::from_str::<Self>( text )
            .context( "Invalid JSON manifest" )?
            .tap( |it| trace!( ?it ) );
        ensure! { manifest.version == CURRENT_MANIFEST_VERSION,
            "Manifest version mismatch, expect {}, got {}",
            CURRENT_MANIFEST_VERSION,
            manifest.version
        };
        Ok( manifest )
    }

    fn empty() -> Self {
        Self { version: CURRENT_MANIFEST_VERSION, symlinks: vec![] }
    }
}

impl PartialEq for Manifest {
    fn ne(&self, other: &Self) -> bool {
        todo!()
    }
    fn eq(&self, other: &Self) -> bool {
        todo!()
    }
}

impl Symlink {}
