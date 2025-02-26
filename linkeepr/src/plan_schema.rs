use serde::Deserialize;
use std::path::Path;

use anyhow::Context;
use tracing::debug;

#[ derive( Deserialize, Debug ) ]
#[ serde( tag="version" ) ]
pub enum Plan {
    V1( v1::Plan )
}

impl Plan {
    /// Upgrade the current plan to the latest.
    #[ allow( dead_code ) ]
    pub fn upgrade( self ) -> Self {
        todo!()
    }

    /// Parse plan from the contents of file.
    #[ tracing::instrument ]
    pub fn from_file( path: &Path ) -> anyhow::Result<Self> {
        debug!( "Read file content and parse the plan" );
        let content = std::fs::read_to_string( path )
            .with_context(
                || format!( r#"Error reading file content "{}""#, path.display() )
            )?;
        let plan = serde_json::from_str::<Self>( &content )
            .with_context(
                || format!( r#"Failed to parse "{}""#, path.display() )
            )?;
        Ok( plan )
    }
}

pub mod v1 {
    //! Version 1 of plan schema
    use serde::Deserialize;

    #[ derive( Deserialize, Debug ) ]
    pub struct Plan {
        pub links: Vec<LinkAction>,
    }

    #[ derive( Deserialize, Debug ) ]
    pub struct LinkAction {
        pub source: String,
        pub target: String,
    }
}

#[ test ]
fn tt() {
    let c = r#"
        {
            "version": "V1",
            "links": [
                { "source": "a", "target": "b" }
            ]
        }
    "#;
    dbg!( serde_json::from_str::<Plan>( &c ) );
}
