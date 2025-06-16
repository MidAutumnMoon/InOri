use std::fmt::Display;

use crate::blueprint::Blueprint;
use crate::blueprint::Symlink;
use crate::template::RenderedPath;

use anyhow::ensure;
use anyhow::Context;
use anyhow::Result as AnyResult;
use ino_color::fg::Blue;
use ino_color::InoColor;
use ino_tap::TapExt;
use tap::Pipe;
use tap::Tap;
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
        eprintln!( "{}", "Understanding blueprint".fg::<Blue>() );

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

        let actions =
            Self::put_blueprint_into_action( new_blueprint, old_blueprint )
                .context( "Error happended when generating works" )?;

        eprintln!( "{}", "Run preflight checks".fg::<Blue>() );
        // Self::precheck_works( &actions )?;

        eprintln!( "{}", "Now it's time to do the real work".fg::<Blue>() );
        Self::execute_works( &actions )
            .context( "Failed to execute the blueprint" )?;

        Ok(())
    }

    #[ tracing::instrument( skip_all ) ]
    fn put_blueprint_into_action(
        new_blueprint: Blueprint,
        old_blueprint: Blueprint
    )
        -> AnyResult<Vec<Action>>
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

        let mut symlinks_in_new_blueprint =
            into_vec_opt!( new_blueprint.symlinks );

        let mut symlinks_in_old_blueprint =
            into_vec_opt!( old_blueprint.symlinks );

        let mut planned_actions =
            symlinks_in_new_blueprint.len()
                .max( symlinks_in_old_blueprint.len() )
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
        for new_symlink in &mut symlinks_in_new_blueprint {
            let Some( new_symlink ) = new_symlink.take() else { continue; };
            let mut found_old_symlink = None;

            let _s =
                tracing::trace_span!( "iter_new", ?new_symlink )
                .entered();

            for old_symlink in &mut symlinks_in_old_blueprint {
                let _s =
                    tracing::trace_span!( "iter_old", ?old_symlink )
                    .entered();
                if old_symlink.as_ref()
                    .map( |old| old.same_dst( &new_symlink ) )
                    .is_some_and( |cond| cond )
                {
                    found_old_symlink = old_symlink.take();
                    trace!( ?found_old_symlink, "found matching symlink from old" );
                } else {
                    trace!( "not this one" );
                }
            }

            if let Some( found_old_symlink ) = found_old_symlink {
                if found_old_symlink.same_src( &new_symlink ) {
                    trace!( "same src, do nothing" );
                    Action::new_nothing()
                } else {
                    trace!( "replace symlink" );
                    Action::new_replace( new_symlink, found_old_symlink )
                }
            } else {
                trace!( "create new symlink" );
                Action::new_create( new_symlink )
            }
                .tap_trace()
                .pipe( |it| planned_actions.push( it ) );
        }

        // At this point, the remaining symlinks in the old blueprint
        // are ones need to be removed, because they didn't match
        // any in the new blueprint.
        for old_symlink in &mut symlinks_in_old_blueprint {
            let _s =
                tracing::trace_span!( "iter_one_remaning", ?old_symlink ).entered();
            let Some( old_symlink ) = old_symlink.take() else { continue; };
            Action::new_remove( old_symlink )
                .tap_trace()
                .pipe( |it| { planned_actions.push( it ); } );
        }

        ensure!(
            symlinks_in_new_blueprint.into_iter()
                .chain( symlinks_in_old_blueprint.into_iter() )
                .all( |it| it.is_none() ),
            "Bug in the code, symlinks are not completely drained"
        );

        planned_actions.tap_trace().pipe( Ok )
    }

    #[ tracing::instrument( skip_all ) ]
    fn precheck_works( actions: &Vec<Action> ) -> AnyResult<()> {
        for act in actions {
            act.check_collision()?;
        }
        Ok(())
    }

    #[ tracing::instrument( skip_all ) ]
    fn execute_works( actions: &Vec<Action> ) -> AnyResult<()> {
        for act in actions {
            act.execute()?;
        }
        Ok(())
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
        src: RenderedPath,
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

    #[ inline ]
    pub fn new_create( new: Symlink ) -> Self {
        Self::Create {
            src: new.src,
            dst: new.dst
        }
    }
    #[ inline ]
    pub fn new_remove( old: Symlink ) -> Self {
        Self::Remove {
            src: old.src,
            dst: old.dst
        }
    }
    #[ inline ]
    pub fn new_replace( new: Symlink, old: Symlink ) -> Self {
        Self::Replace {
            new_src: new.src,
            old_src: old.src,
            dst: new.dst
        }
    }
    #[ inline ]
    pub fn new_nothing() -> Self {
        Self::Nothing
    }

    #[ tracing::instrument ]
    pub fn execute( &self ) -> AnyResult<()> {
        use std::os::unix::fs::symlink;

        match self {
            Self::Create { src, dst } => {
                debug!( "create symlink" );
                symlink( src, dst )
                    .with_context( || format!(
                        r#"Failed to create symlink on "{}""#,
                        dst.path().display()
                    ) )?;
            },

            Self::Replace { new_src, old_src, dst } => {
                todo!()
            },

            Self::Remove { src, dst } => {
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
        todo!()
    }

}

impl Display for Action {
    fn fmt( &self, f: &mut std::fmt::Formatter<'_> ) -> std::fmt::Result {
        match self {
            Self::Create { src, dst } => {
                format!(
                    r#"Create symlink: src="{}" dst="{}""#,
                    src.path().display(),
                    dst.path().display(),
                ).pipe( |it| f.write_str( &it ) )?;
            },
            _ => todo!()
        }
        Ok(())
    }
}

#[ cfg( test ) ]
mod test {

    use super::*;

    use assert_fs::prelude::*;
    use assert_fs::TempDir;

    #[ test ]
    fn generate_action() {
        let mut new = Blueprint::default();
        new.symlinks = vec![];

        let old = Blueprint::default();
    }

    #[ test ]
    fn collision_precheck() {
        let top = TempDir::new()
            .expect( "Failed to create tempdir" );

        let dst = top.child( "dsttttttt" );
    }

    #[ test ]
    fn action_toctou() {}

}
