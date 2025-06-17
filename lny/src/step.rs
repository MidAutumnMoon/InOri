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

// TODO: move dst conflict check here?

#[ derive( Debug, Clone ) ]
pub struct StepQueue {
    steps: Vec<Step>
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

impl Iterator for StepQueue {
    type Item = Step;
    fn next( &mut self ) -> Option<Self::Item> {
        self.steps.pop()
    }
}

/// The step to be taken.
/// N.B. Best effort [TOC/TOU](https://w.wiki/GQE) prevention.
#[ derive( Debug, Clone, PartialEq, Eq ) ]
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

impl Display for Step {
    fn fmt( &self, f: &mut std::fmt::Formatter<'_> ) -> std::fmt::Result {
        todo!()
    }
}

#[ allow( clippy::unwrap_used ) ]
#[ cfg( test ) ]
mod test {

    use super::*;

    use assert_fs::prelude::*;
    use assert_fs::TempDir;

    #[ macro_export ]
    macro_rules! make_tempdir {
        () => { {
            TempDir::new().expect( "Failed to setup tempdir" )
        } };
    }

    // TODO: move to blueprint.rs and make it public?
    #[ macro_export ]
    macro_rules! make_symlink {
        () => { {
            make_symlink!( "/ssrc", "/ddst" )
        } };
        ( $src:literal ) => {
            make_symlink!( $src, "/ddst" )
        };
        ( $src:literal, $dst:literal ) => { {
            let src = RenderedPath::from_unrendered( $src )
                .expect( "Failed to make src RenderedPath" );
            let dst = RenderedPath::from_unrendered( $dst )
                .expect( "Failed to make dst RenderedPath" );
            Symlink::new_test( src, dst )
        } };
    }

    #[ test ]
    fn generate_steps() {
        // no step
        {
            let new_bp = Blueprint::empty();
            let old_bp = Blueprint::empty();
            let q = StepQueue::new( new_bp, old_bp );
            assert!( q.is_ok_and( |it| it.steps.is_empty() ) );
        }
        // create
        {
            let sym = make_symlink!();
            let new_bp = Blueprint::empty()
                .tap_mut( |it| it.symlinks = vec![ sym.clone() ] );
            let old_bp = Blueprint::empty();
            let q = StepQueue::new( new_bp, old_bp );
            assert! {
                q.is_ok_and( |mut it| {
                    it.steps.len() == 1
                    && it.steps.pop().unwrap()
                        == Step::Create { new_symlink: sym }
                } )
            };
        }
        // remove
        {
            let sym = make_symlink!();
            let new_bp = Blueprint::empty();
            let old_bp = Blueprint::empty()
                .tap_mut( |it| it.symlinks = vec![ sym.clone() ] );
            let q = StepQueue::new( new_bp, old_bp );
            assert! {
                q.is_ok_and( |mut it| {
                    it.steps.len() == 1
                    && it.steps.pop().unwrap()
                        == Step::Remove { old_symlink: sym }
                } )
            };
        }
        // Replace
        {
            let new_symlink = make_symlink!( "/src_new", "/dst" );
            let old_symlink = make_symlink!( "/src_old", "/dst" );

            let new_bp = Blueprint::empty()
                .tap_mut( |it| it.symlinks = vec![ new_symlink.clone() ] );
            let old_bp = Blueprint::empty()
                .tap_mut( |it| it.symlinks = vec![ old_symlink.clone() ] );
            let q = StepQueue::new( new_bp, old_bp );
            assert! {
                q.is_ok_and( |mut it| {
                    it.steps.len() == 1
                    && it.steps.pop().unwrap()
                        == Step::Replace { new_symlink, old_symlink }
                } )
            };
        }
        // Nothing
        {
            let new_symlink = make_symlink!( "/src_x", "/dst" );
            let old_symlink = make_symlink!( "/src_x", "/dst" );

            let new_bp = Blueprint::empty()
                .tap_mut( |it| it.symlinks = vec![ new_symlink.clone() ] );
            let old_bp = Blueprint::empty()
                .tap_mut( |it| it.symlinks = vec![ old_symlink.clone() ] );
            let q = StepQueue::new( new_bp, old_bp );
            assert! {
                q.is_ok_and( |mut it| {
                    it.steps.len() == 1
                    && it.steps.pop().unwrap() == Step::Nothing
                } )
            };
        }
        // Mixed
        {
            let unc_symlink = make_symlink!( "/uncha", "/unch_dst" );
            let new_symlink = make_symlink!( "/src_new_1", "/dst_1" );
            let del_symlink = make_symlink!( "/src_old", "/dst_dd" );
            let rep_symlink_old = make_symlink!( "/src_ooo", "/dst_replace" );
            let rep_symlink_new = make_symlink!( "/src_yee", "/dst_replace" );

            let new_bp = Blueprint::empty()
                .tap_mut( |it| {
                    it.symlinks = vec![
                        unc_symlink.clone(),
                        new_symlink.clone(),
                        rep_symlink_new.clone(),
                    ];
                } );

            let old_bp = Blueprint::empty()
                .tap_mut( |it| {
                    it.symlinks = vec![
                        unc_symlink.clone(),
                        del_symlink.clone(),
                        rep_symlink_old.clone(),
                    ];
                } );

            let q = StepQueue::new( new_bp, old_bp );
            assert!( q.is_ok() );
            let q = q.unwrap();
            assert!( q.steps.len() == 4 );
            assert! {
                q.steps.into_iter()
                    .all( |it|
                        it == Step::Nothing
                        || it == Step::Create { new_symlink: new_symlink.clone() }
                        || it == Step::Remove { old_symlink: del_symlink.clone() }
                        || it == Step::Replace {
                            new_symlink: rep_symlink_new.clone(),
                            old_symlink: rep_symlink_old.clone()
                        } )
            };
        }
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
