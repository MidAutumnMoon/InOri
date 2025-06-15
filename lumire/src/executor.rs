use std::collections::HashSet;
use std::path::PathBuf;

use crate::plan::Plan;
use crate::plan::Symlink;
use crate::template::RenderedPath;

use anyhow::ensure;
use anyhow::Context;
use anyhow::Result as AnyResult;
use ino_tap::TapExt;
use tap::Pipe;
use tracing::debug;
use tracing::trace;

#[ derive( Debug ) ]
pub struct Executor {
    works: Vec<Action>
}

impl Executor {

    #[ tracing::instrument( name="executor_run_with", skip_all ) ]
    pub fn run_with( new_plan: Option<Plan>, old_plan: Option<Plan> )
        -> AnyResult<()>
    {
        trace!( ?new_plan );
        trace!( ?old_plan );

        let new_plan = new_plan.unwrap_or_else( || {
            debug!( "No new plan provided, using default" );
            Plan::default()
        } );

        let old_plan = old_plan.unwrap_or_else( || {
            debug!( "No old plan provided, using default" );
            Plan::default()
        } );

        let works = Self::generate_works( new_plan, old_plan )
            .context( "Error happend when generating works" )?
            .tap_trace();

        let me = Self { works };

        todo!()
    }

    #[ tracing::instrument( skip_all ) ]
    fn generate_works( new_plan: Plan, old_plan: Plan )
        -> AnyResult< Vec<Action> >
    {
        macro_rules! into_vec_opt {
            ( $input:expr ) => { {
                $input.into_iter()
                    .map( |it| Some( it ) )
                    .collect::<Vec<_>>()
            } };
        }

        debug!( "Calculate works" );

        let new = into_vec_opt!( new_plan.symlinks );
        let mut old = into_vec_opt!( old_plan.symlinks );

        let mut works = Vec::with_capacity(
            new.len().max( old.len() )
        );

        // This is inefficient, but also imcomplex and works well
        // for few thoudsands or even few tens of thoudsands paths.
        // Considering home-manager uses the f bash to implement the same
        // thing and yet no one has complained about the performance,
        // we can assume the scale we are dealing are pretty damn small.
        // No need to switch algorithm in the near future.

        // intersection + difference (new only)
        for mut new_slk in new {
            let Some( new_slk ) = new_slk.take() else { continue; };
            let mut found = None;

            for old_slk in &mut old {
                let Some( x ) = old_slk.as_ref() else { continue; };
                if x.dst().path() == new_slk.dst().path() {
                    found = old_slk.take();
                }
            }

            if let Some( syml ) = found {
                Action::Replace {
                    new_src: new_slk.src,
                    old_src: syml.dst,
                    dst: new_slk.dst,
                }
                    .tap_trace()
                    .pipe( |it| { works.push( it ); } );
            } else {
                Action::Create { src: new_slk.src, dst: new_slk.dst, }
                    .tap_trace()
                    .pipe( |it| { works.push( it ); } );
            }
        }

        // difference (old only)
        for s in old {
            let Some( x ) = s else { continue; };
            Action::Remove { old_src: x.src, dst: x.dst }
                .tap_trace()
                .pipe( |it| { works.push( it ); } );
        }

        works.tap_trace().pipe( Ok )
    }

    // N.B. not resistent to TOCTOU bugs.
    #[ tracing::instrument( skip_all ) ]
    fn check_collision<'f>( symlinks: impl Iterator<Item = &'f Symlink> )
        -> AnyResult<()>
    {
        debug!( "precheck collisions in plan" );
        for link in symlinks {
            trace!( ?link );
            // TODO: use thiserror to replace string error
            ensure! { !link.dst().path().exists(), }
        }
        Ok(())
    }

}

impl Iterator for Executor {
    type Item = AnyResult<()>;

    fn next( &mut self ) -> Option<Self::Item> {
        todo!()
    }
}

/// The action to be taken.
/// N.B. Best effort [TOC/TOU](https://w.wiki/GQE) prevention.
#[ derive( Debug ) ]
enum Action {
    Create {
        src: RenderedPath,
        dst: RenderedPath,
    },
    Remove {
        /// Record the src to prevent TOCTOU.
        old_src: RenderedPath,
        dst: RenderedPath,
    },
    Replace {
        new_src: RenderedPath,
        old_src: RenderedPath,
        dst: RenderedPath,
    },
    Nothing,
}

impl Action {

    pub fn execute( &self ) -> AnyResult<()> {
        todo!()
    }

    #[ tracing::instrument ]
    pub fn check( &self ) -> AnyResult<()> {
        todo!()
    }

}

#[ cfg( test ) ]
mod test {

    use super::*;

    use assert_fs::prelude::*;
    use assert_fs::TempDir;

    #[ test ]
    fn collision_precheck() {
        let top = TempDir::new()
            .expect( "Failed to create tempdir" );

        let dst = top.child( "dsttttttt" );
    }

    #[ test ]
    fn old_plan_unique() {
    }

    #[ test ]
    fn action_toctou() {}

}
