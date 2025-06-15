use std::path::Path;
use std::path::PathBuf;

use anyhow::ensure;
use anyhow::Context;
use anyhow::Result as AnyResult;
use ino_tap::TapExt;
use serde::Deserialize;
use tap::Pipe;
use tap::Tap;
use tracing::debug;
use tracing::trace;

use crate::template::RenderedPath;

const CURRENT_PLAN_VERSION: usize = 1;

#[ derive( Deserialize, Debug ) ]
#[ serde( deny_unknown_fields ) ]
pub struct Plan {
    version: usize,
    symlinks: Vec<Symlink>,
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
            .tap_trace();
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

#[ derive( Deserialize, Debug ) ]
#[ serde( deny_unknown_fields ) ]
pub struct Symlink {
    src: RenderedPath,
    dst: RenderedPath,
}

#[ cfg( test ) ]
mod test {
    use super::*;

    #[ test ]
    fn plan_need_to_be_strict() {
        use serde::de::IntoDeserializer;
        // plan with arbitrary unknown fields
        let json = serde_json::json!( {
            "version": CURRENT_PLAN_VERSION,
            "yolo": "once",
            "symlinks": [ { "src": "/", "dst": "/", "aa": "bb" } ]
        } );
        let der = json.into_deserializer();
        let res = Plan::deserialize( der );
        assert!( res.is_err() );
    }

}
