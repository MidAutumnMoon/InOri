use std::path::PathBuf;

use crate::plan::Plan;
use crate::plan::Symlink;
use crate::template::RenderedPath;

use anyhow::Result as AnyResult;

#[ derive( Debug ) ]
pub struct Executor {
    works: Vec<Action>,
}

impl Executor {

    #[ allow( clippy::new_ret_no_self ) ]
    #[ tracing::instrument( skip_all ) ]
    pub fn run_with( new: Option<Plan>, olds: Option<Vec<Plan>> )
        -> AnyResult<()>
    {
        todo!()
    }

    #[ tracing::instrument( skip_all ) ]
    fn generate_works() {}

}

/// The action to be taken.
/// N.B. Best effort [TOC/TOU](https://w.wiki/GQE) prevention.
#[ derive( Debug ) ]
enum Action {
    /// Create a symlink
    Add {
        src: RenderedPath,
        dst: RenderedPath,
    },
    /// Remove a symlink
    Remove {
        /// Record the src to prevent TOCTOU.
        old_src: RenderedPath,
        dst: RenderedPath,
    },
    /// Replace a symlink
    Replace {
        new_src: PathBuf,
        old_src: PathBuf,
        dst: PathBuf,
    },
    Nothing,
}

impl Action {

    /// Generate a change by diffing two [`Symlink`]
    #[ inline ]
    pub fn from_diff( left: &Symlink, right: &Symlink ) -> Self {
        todo!()
    }

    #[ inline ]
    pub fn execute( &self ) -> AnyResult<()> {
        todo!()
    }

}

#[ cfg( test ) ]
mod test {

    use super::*;

    #[ test ]
    fn old_plan_unique() {
    }

    #[ test ]
    fn action_toctou() {}

}
