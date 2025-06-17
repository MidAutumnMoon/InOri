use std::hash::Hasher;
use std::path::Path;
use std::str::FromStr;

use anyhow::ensure;
use anyhow::Context;
use anyhow::Result as AnyResult;
use ino_tap::TapExt;
use itertools::Itertools;
use serdev::Deserialize;
use tap::Pipe;
use tracing::debug;

use crate::template::RenderedPath;

const CURRENT_BLUEPRINT_VERSION: usize = 1;

#[ derive( Deserialize, Debug ) ]
#[ serde( deny_unknown_fields ) ]
#[ serde( validate="Self::validate" ) ]
pub struct Blueprint {
    version: usize,
    // TODO avoid direct field access
    pub symlinks: Vec<Symlink>,
}

// TODO: implement Deserialize manually for better checking,
// but that's so miserable... fuck serde
impl Blueprint {
    #[ tracing::instrument ]
    pub fn from_file( path: &Path ) -> AnyResult<Self> {
        debug!( "read the blueprint file" );
        ensure! { path.is_file(),
            r#"The given path "{}" is not file"#, path.display()
        };
        std::fs::read_to_string( path )
            .context( "Failed to read blueprint file" )?
            .pipe_deref( Self::from_str )
            .context( "Failed to parse the blueprint's content" )
    }

    #[ tracing::instrument( skip_all ) ]
    fn validate( &self ) -> AnyResult<()> {
        debug!( "validate the blueprint" );
        ensure! { self.version == CURRENT_BLUEPRINT_VERSION,
            r#"Blueprint version mismatch, expect "{}", got "{}""#,
            CURRENT_BLUEPRINT_VERSION,
            self.version
        };
        // TODO report which ones are conflicting
        ensure! {
            self.symlinks.iter()
                .map( |it| &it.dst )
                .all_unique(),
            "Some symlinks in the blueprint have conflicting destination path"
        };
        Ok(())
    }
}

impl FromStr for Blueprint {
    type Err = anyhow::Error;

    #[ tracing::instrument( skip_all ) ]
    fn from_str( raw: &str ) -> Result<Self, Self::Err> {
        debug!( "try parse the input as json" );
        serde_json::from_str::<Self>( raw )
            .context( "Blueprint contains invalid JSON" )?
            .tap_trace()
            .pipe( Ok )
    }
}

impl Default for Blueprint {
    fn default() -> Self {
        Self { version: CURRENT_BLUEPRINT_VERSION, symlinks: vec![] }
    }
}

#[ derive( Deserialize, Debug ) ]
#[ serde( deny_unknown_fields ) ]
pub struct Symlink {
    src: RenderedPath,
    /// Only the `dst` matters as it's not our job to validate src.
    dst: RenderedPath,
}

impl Symlink {
    pub fn dst( &self ) -> &RenderedPath { &self.dst }
    pub fn src( &self ) -> &RenderedPath { &self.src }

    pub fn into_inner( self ) -> ( RenderedPath, RenderedPath ) {
        ( self.src, self.dst )
    }

    pub fn same_dst( &self, other: &Self ) -> bool {
        self.dst() == other.dst()
    }

    pub fn same_src( &self, other: &Self ) -> bool {
        self.src() == other.src()
    }
}

#[ cfg( test ) ]
mod test {

    use super::*;
    use serde::de::IntoDeserializer;
    use tap::Tap;

    #[ test ]
    #[ allow( clippy::unwrap_used ) ]
    fn symlinks_are_unique() {
        let json = serde_json::json!{ {
            "version": CURRENT_BLUEPRINT_VERSION,
            "symlinks": [
                { "src": "/a", "dst": "/tar" },
                { "src": "/b", "dst": "/tar" },
            ]
        } };
        let der = json.into_deserializer();
        let res = Blueprint::deserialize( der );
        assert!( res.is_err() );
        assert!(
            res.err().unwrap()
                .tap( |it| eprintln!( "{it:?}" ) )
                .to_string()
                .contains( "conflicting" )
        );
    }

    #[ test ]
    fn be_strict_when_parsing() {
        let json = serde_json::json!( {
            "version": CURRENT_BLUEPRINT_VERSION,
            "yolo": "once",
            "symlinks": [ { "src": "/", "dst": "/", "aa": "bb" } ]
        } );
        let der = json.into_deserializer();
        let res = Blueprint::deserialize( der );
        assert!( res.is_err() );
    }

}
