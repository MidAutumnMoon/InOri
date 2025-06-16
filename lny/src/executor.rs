use crate::blueprint::Blueprint;
use crate::blueprint::Symlink;
use crate::template::RenderedPath;

use anyhow::ensure;
use anyhow::Context;
use anyhow::Result as AnyResult;
use ino_tap::TapExt;
use tap::Pipe;
use tracing::debug;
use tracing::trace;

#[ derive( Debug ) ]
pub struct Executor;

impl Executor {

    #[ tracing::instrument( name="executor_run_with", skip_all ) ]
    pub fn run_with(
        new_blueprint: Option<Blueprint>,
        old_blueprint: Option<Blueprint>
    ) -> AnyResult<()>
    {
        trace!( ?new_blueprint );
        trace!( ?old_blueprint );

        let new_blueprint =
            new_blueprint.unwrap_or_else( || {
                debug!( "No new blueprint provided, using default" );
                Blueprint::default()
            } );

        let old_blueprint =
            old_blueprint.unwrap_or_else( || {
                debug!( "No old blueprint provided, using default" );
                Blueprint::default()
            } );

        let works =
            Self::put_blueprint_into_action( new_blueprint, old_blueprint )
                .context( "Error happend when generating works" )?
                .tap_trace();

        Self::precheck_works( works.iter() )?;
        Self::execute_works( works.iter() )?;

        Ok(())
    }

    #[ tracing::instrument( skip_all ) ]
    fn put_blueprint_into_action(
        new_blueprint: Blueprint,
        old_blueprint: Blueprint
    ) -> AnyResult<Vec<Action>>
    {
        macro_rules! into_vec_opt {
            ( $input:expr ) => { {
                $input.into_iter()
                    .map( |it| Some( it ) )
                    .collect::<Vec<_>>()
                    .tap_trace()
            } };
        }

        debug!( "make actions according to blueprint" );

        let symlinks_in_new_blueprint =
            into_vec_opt!( new_blueprint.symlinks );
        let mut symlinks_in_old_blueprint =
            into_vec_opt!( old_blueprint.symlinks );

        let mut works =
            symlinks_in_new_blueprint.len()
                .max( symlinks_in_old_blueprint.len() )
                .pipe( Vec::with_capacity );

        // This is inefficient, but also imcomplex and works well
        // for few thoudsands or even few tens of thoudsands paths.
        // Considering home-manager uses the f bash to implement the same
        // thing and yet no one has complained about the performance,
        // we can assume the scale we are dealing are pretty damn small.
        // No need to switch algorithm in the near future.

        // intersection + difference (new only)
        //
        // 1. If two symlinks with the same dst exist in both new and old
        //  1.1 if the srcs are the same then it'll be nothing
        //  1.2 if the srcs are different then it'll be a replace
        // 2. If the symlink only exists in new, then it'll be a create
        for mut new_slk in symlinks_in_new_blueprint {
            let Some( new_slk ) = new_slk.take() else { continue; };
            let mut found = None;

            for old_slk in &mut symlinks_in_old_blueprint {
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
        for s in symlinks_in_old_blueprint {
            let Some( x ) = s else { continue; };
            Action::Remove { old_src: x.src, dst: x.dst }
                .tap_trace()
                .pipe( |it| { works.push( it ); } );
        }

        works.tap_trace().pipe( Ok )
    }

    #[ tracing::instrument( skip_all ) ]
    fn precheck_works<'f>( actions: impl Iterator<Item = &'f Action> )
        -> AnyResult<()>
    {
        todo!()
    }

    #[ tracing::instrument( skip_all ) ]
    fn execute_works<'f>( works: impl Iterator<Item = &'f Action> )
        -> AnyResult<()>
    {
        todo!()
    }

}

/// The action to be taken.
/// N.B. Best effort [TOC/TOU](https://w.wiki/GQE) prevention.
#[ derive( Debug ) ]
pub enum Action {
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
        use std::os::unix::fs::symlink;
        use Action::Create;

        match self {
            action @ Create { src, dst } => {
                debug!( "create symlink" );
                symlink( src, dst )?;
            },
            _ => todo!(),
        }

        Ok(())
    }

    // TODO replace string error with thiserror

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
    fn action_toctou() {}

}
