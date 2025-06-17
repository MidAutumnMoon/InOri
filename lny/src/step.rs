use std::fmt::Display;

use crate::blueprint::Blueprint;
use crate::blueprint::Symlink;
use crate::template::RenderedPath;

use anyhow::bail;
use anyhow::ensure;
use anyhow::Context;
use anyhow::Result as AnyResult;
use ino_color::fg::Blue;
use ino_color::fg::Red;
use ino_color::InoColor;
use ino_tap::TapExt;
use itertools::Itertools;
use tap::Pipe;
use tap::Tap;
use tracing::debug;
use tracing::trace;

#[ derive( Debug, Clone ) ]
pub struct StepQueue {
    inner: Vec<Step>
}

impl StepQueue {

    #[ tracing::instrument( name="executor_new", skip_all ) ]
    pub fn new( new_blueprint: Blueprint, old_blueprint: Blueprint )
        -> AnyResult<Self>
    {

        debug!( "actualize blueprint into steps" );

        eprintln!( "{}", "Actualize blueprint".fg::<Blue>() );

        trace!( ?new_blueprint );
        trace!( ?old_blueprint );

        let ( mut new_blueprint_symlinks, mut old_blueprint_symlinks ) =
            [ new_blueprint.symlinks, old_blueprint.symlinks ]
                .map( |it| it.into_iter().map( Some ) )
                .map( |it| it.collect_vec().tap_trace() )
                .into();

        let mut steps = new_blueprint_symlinks.len()
            .max( old_blueprint_symlinks.len() )
            .pipe( Vec::with_capacity );

        // This is inefficient, but also imcomplex and works well
        // for few thoudsands or even few tens of thoudsands items.
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
        for new_symlink in &mut new_blueprint_symlinks {
            use tracing::trace_span;

            let Some( new_symlink ) = new_symlink.take() else { continue; };
            let mut found_old_symlink = None;

            let _s = trace_span!( "iter_new", ?new_symlink ).entered();

            for old_symlink in &mut old_blueprint_symlinks {
                let _s = trace_span!( "iter_old", ?old_symlink ).entered();
                if old_symlink.as_ref()
                    .map( |old| old.same_dst( &new_symlink ) )
                    .is_some_and( |cond| cond )
                {
                    found_old_symlink = old_symlink.take();
                    trace!( ?found_old_symlink, "found matching symlink from old" );
                }
            }

            if let Some( old_symlink ) = found_old_symlink {
                if old_symlink.same_src( &new_symlink ) {
                    trace!( "same src, do nothing" );
                    Step::Nothing
                } else {
                    trace!( "replace symlink" );
                    Step::Replace { new_symlink, old_symlink }
                }
            } else {
                trace!( "create new symlink" );
                Step::Create { new_symlink }
            }
                .tap_trace()
                .pipe( |it| steps.push( it ) );
        }

        // At this point, the remaining symlinks in the old blueprint
        // are ones need to be removed, because they didn't match
        // any in the new blueprint.
        for old_symlink in &mut old_blueprint_symlinks {
            let _s = tracing::trace_span!( "iter_one_remaning", ?old_symlink )
                .entered();
            let Some( old_symlink ) = old_symlink.take() else { continue; };
            Step::Remove { old_symlink }
                .tap_trace()
                .pipe( |it| { steps.push( it ); } );
        }

        ensure!(
            new_blueprint_symlinks.iter()
                .chain( old_blueprint_symlinks.iter() )
                .all( Option::is_none ),
            "[BUG] symlinks are not completely drained"
        );

        Ok( Self { steps } )
    }

}

/// The step to be taken.
/// N.B. Best effort [TOC/TOU](https://w.wiki/GQE) prevention.
#[ derive( Debug, Clone ) ]
pub enum Step {
    Create {
        new_symlink: Symlink,
    },
    Remove {
        old_symlink: Symlink,
    },
    Replace {
        new_symlink: Symlink,
        old_symlink: Symlink,
    },
    Nothing,
}

impl Step {

    #[ tracing::instrument ]
    pub fn execute( &self ) -> AnyResult<()> {
        use std::os::unix::fs::symlink;

        match self {
            Self::Create { new_symlink } => {
                debug!( "create symlink" );
                symlink( new_symlink.src(), new_symlink.dst() )
                    .with_context( || format!(
                        r#"Failed to create symlink on "{}""#,
                        new_symlink.dst().display()
                    ) )?;
            },

            Self::Replace { new_symlink, old_symlink } => {
                todo!()
            },

            Self::Remove { old_symlink } => {
                todo!()
            },

            Self::Nothing => {
                debug!( "do nothing" );
            },
        }

        Ok(())
    }

    #[ tracing::instrument ]
    pub fn check_collision( &self ) -> AnyResult<()> {
        // TODO replace string error with thiserror
        // and don't do eprint here
        debug!( "check for collinsion" );

        #[ allow( clippy::inline_always ) ]
        #[ inline( always ) ]
        #[ tracing::instrument ]
        fn check_ours( src: &RenderedPath, dst: &RenderedPath )
            -> AnyResult<()>
        {
            if dst.try_exists()? {
                if dst.is_symlink() {
                    if dst.read_link()? == src.as_ref() {
                        Ok(())
                    } else {
                        bail!( r#"Conflict on "{}""#, dst.display() )
                    }
                } else {
                    bail!( r#"Conflict on "{}""#, dst.display() )
                }
            } else {
                Ok(())
            }
        }

        match self {
            Self::Create { new_symlink } => {
                // check_ours( src, dst )?;
            },
            _ => todo!(),
        }
        Ok(())
    }

}

#[ cfg( test ) ]
mod test {

    use super::*;

    use assert_fs::prelude::*;
    use assert_fs::TempDir;

    macro_rules! make_tempdir {
        () => { {
            TempDir::new().expect( "Failed to setup tempdir" )
        } };
    }

    #[ test ]
    fn create_collision() {
        let top = make_tempdir!();
        let src = top.child( "src" );
        let dst = top.child( "dst" );
    }

    #[ test ]
    fn collision_precheck() {
        let top = TempDir::new()
            .expect( "Failed to create tempdir" );

        let dst = top.child( "dsttttttt" );
    }

}
