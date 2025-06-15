use std::path::PathBuf;

use crate::manifest::Manifest;
use crate::manifest::Symlink;

use anyhow::Result as AnyResult;

#[ derive( Debug ) ]
pub enum Action {
    Add {
        src: PathBuf,
        dst: PathBuf,
        mode: String,
    },
    Remove {
        dst: PathBuf
    },
    Replace {
        src: PathBuf,
        old_src: PathBuf,
        dst: PathBuf,
        mode: u32,
    }
}

impl Action {

    /// Generate a change by diffing two [`Symlink`]
    #[ inline ]
    pub fn diff( left: &Symlink, right: &Symlink ) -> Self {
        todo!()
    }

    #[ inline ]
    pub fn execute( &self ) -> AnyResult<()> {
        todo!()
    }

}

#[ derive( Debug ) ]
pub struct Executor {
    works: Works,
}

impl Executor {

    #[ tracing::instrument( skip_all ) ]
    pub fn new( new: Option<Manifest>, olds: Option<Vec<Manifest>> )
        -> AnyResult<()>
    {
        todo!()
    }

    #[ tracing::instrument( skip( self ) ) ]
    pub fn run( self ) {}

}

#[ derive( Debug ) ]
pub struct Works {
    changeset: Vec<Action>,
}

impl Iterator for Works {
    type Item = ();

    fn next( &mut self ) -> Option<Self::Item> {
        todo!()
    }
}
