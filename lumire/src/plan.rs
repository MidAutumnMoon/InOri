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

const CURRENT_PLAN_VERSION: usize = 1;

#[ derive( Deserialize, Debug ) ]
pub struct Plan {
    version: usize,
    symlinks: Vec<Symlink>,
}

#[ derive( Deserialize, Debug ) ]
pub struct Symlink {
    src: RenderedPath,
    dst: RenderedPath,
    mode: String,
}

impl Plan {
    #[ tracing::instrument ]
    pub fn from_file( path: &Path ) -> AnyResult<Self> {
        debug!( "read plan file" );
        ensure! { path.is_file(),
            r#"Plan of path "{}" does not point to a file"#, path.display()
        };
        std::fs::read_to_string( path )
            .context( "Failed to read plan file" )?
            .pipe_deref( Self::from_str )?
            .pipe( Ok )
    }

    #[ tracing::instrument( skip_all ) ]
    fn from_str( text: &str ) -> AnyResult<Self> {
        debug!( "Parse plan data" );
        let plan = serde_json::from_str::<Self>( text )
            .context( "Plan contains invalid JSON" )?
            .tap( |it| trace!( ?it ) );
        ensure! { plan.version == CURRENT_PLAN_VERSION,
            "Plan version mismatch, expect {}, but got {}",
            CURRENT_PLAN_VERSION,
            plan.version
        };
        Ok( plan )
    }

    fn empty() -> Self {
        Self { version: CURRENT_PLAN_VERSION, symlinks: vec![] }
    }
}

impl PartialEq for Plan {
    fn ne(&self, other: &Self) -> bool {
        todo!()
    }
    fn eq(&self, other: &Self) -> bool {
        todo!()
    }
}

impl Symlink {}
