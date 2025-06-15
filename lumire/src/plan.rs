use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;

use anyhow::ensure;
use anyhow::Context;
use anyhow::Result as AnyResult;
use ino_tap::TapExt;
use itertools::Itertools;
use serdev::Deserialize;
use tap::Pipe;
use tap::Tap;
use tracing::debug;
use tracing::trace;

use crate::template::RenderedPath;

const CURRENT_PLAN_VERSION: usize = 1;

#[ derive( Deserialize, Debug ) ]
#[ serde( deny_unknown_fields ) ]
#[ serde( validate="Self::validate" ) ]
pub struct Plan {
    version: usize,
    symlinks: Vec<Symlink>,
}

// TODO: implement Deserialize manually for better checking,
// but that's so miserable... fuck serde
impl Plan {
    #[ tracing::instrument ]
    pub fn from_file( path: &Path ) -> AnyResult<Self> {
        debug!( "read plan file" );
        ensure! { path.is_file(),
            r#"Plan of path "{}" does not point to a file"#, path.display()
        };
        std::fs::read_to_string( path )
            .context( "Failed to read plan file" )?
            .pipe_deref( Self::from_str )
            .context( "Failed to parse plan data" )
    }

    #[ tracing::instrument( skip_all ) ]
    fn validate( &self ) -> AnyResult<()> {
        debug!( "validate plan" );
        ensure! { self.version == CURRENT_PLAN_VERSION,
            r#"Plan version mismatch, expect "{}", but got "{}""#,
            CURRENT_PLAN_VERSION,
            self.version
        };
        ensure! {
            self.symlinks.iter()
                .map( |it| &it.dst )
                .all_unique(),
            "Some symlinks in the plan have the same destination"
        };
        Ok(())
    }

    fn empty() -> Self {
        Self { version: CURRENT_PLAN_VERSION, symlinks: vec![] }
    }
}

impl FromStr for Plan {
    type Err = anyhow::Error;

    #[ tracing::instrument ]
    fn from_str( raw: &str ) -> Result<Self, Self::Err> {
        debug!( "Parse plan json data" );
        serde_json::from_str::<Self>( raw )
            .context( "Plan contains invalid JSON" )?
            .tap_trace()
            .pipe( Ok )
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
    use serde::de::IntoDeserializer;

    #[ test ]
    #[ allow( clippy::unwrap_used ) ]
    fn symlinks_in_plan_are_unique() {
        let json = serde_json::json!{ {
            "version": CURRENT_PLAN_VERSION,
            "symlinks": [
                { "src": "/a", "dst": "/tar" },
                { "src": "/b", "dst": "/tar" },
            ]
        } };
        let der = json.into_deserializer();
        let res = Plan::deserialize( der );
        assert!( res.is_err() );
        assert!(
            res.err().unwrap()
                .tap( |it| eprintln!( "{it:?}" ) )
                .to_string()
                .contains( "same destination" )
        );
    }

    #[ test ]
    fn plan_need_to_be_strict() {
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
