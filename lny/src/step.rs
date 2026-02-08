use std::fs::remove_file;
use std::fs::rename;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::path::PathBuf;

use crate::blueprint::Blueprint;
use crate::blueprint::Symlink;

use anyhow::Context;
use anyhow::Result as AnyResult;
use anyhow::bail;
use anyhow::ensure;
use ino_color::InoColor;
use ino_color::fg::Blue;
use ino_path::PathExt;
use ino_tap::TapExt;
use itertools::Itertools;
use rand::RngExt;
use tap::Pipe;
use tap::Tap;
use tracing::debug;
use tracing::trace;

// TODO: move dst conflict check here?

#[derive(Debug, Clone)]
pub struct StepQueue {
    steps: Vec<Step>,
}

impl StepQueue {
    #[tracing::instrument(name = "step_queue_new", skip_all)]
    pub fn new(
        new_blueprint: Blueprint,
        old_blueprint: Blueprint,
    ) -> AnyResult<Self> {
        eprintln!("{}", "Actualize blueprint".fg::<Blue>());
        debug!("actualize blueprint into steps");
        trace!(?new_blueprint, ?old_blueprint);

        let (mut new_blueprint_symlinks, mut old_blueprint_symlinks) =
            [new_blueprint.symlinks, old_blueprint.symlinks]
                .map(|it| it.into_iter().map(Some))
                .map(|it| it.collect_vec().tap_trace())
                .into();

        let mut steps = new_blueprint_symlinks
            .len()
            .max(old_blueprint_symlinks.len())
            .pipe(Vec::with_capacity);

        // This is inefficient, but also not complex and works well
        // for few thousands or even few tens of thousands items.
        // Considering home-manager uses the f bash to implement the same
        // thing and yet no one has complained about the performance,
        // we can assume the scale we are dealing are pretty damn small.
        // No need to switch algorithm in the near future.

        // intersection + difference (new only)
        //
        // 1. If two symlinks with the same dst exist in both new and old
        //  1.1 if the src are the same, then it'll be nothing
        //  1.2 if the src are different then it'll be a replace
        // 2. If the symlink only exists in new, then it'll be a create
        for new_symlink in &mut new_blueprint_symlinks {
            use tracing::trace_span;

            let Some(new_symlink) = new_symlink.take() else {
                continue;
            };
            let mut found_old_symlink = None;

            let _s = trace_span!("iter_new", ?new_symlink).entered();

            for old_symlink in &mut old_blueprint_symlinks {
                let _s = trace_span!("iter_old", ?old_symlink).entered();
                if let Some(old) = old_symlink.as_ref()
                    && old.same_dst(&new_symlink)
                {
                    found_old_symlink = old_symlink.take();
                    trace!(?found_old_symlink, "matched symlink from old");
                }
            }

            if let Some(old_symlink) = found_old_symlink {
                if old_symlink.same_src(&new_symlink) {
                    trace!("same src, do nothing");
                    Step::Nothing
                } else {
                    trace!("replace symlink");
                    Step::Replace {
                        new_symlink,
                        old_symlink,
                    }
                }
            } else {
                trace!("create new symlink");
                Step::Create { new_symlink }
            }
            .tap_trace()
            .pipe(|it| steps.push(it));
        }

        // At this point, the remaining symlinks in the old blueprint
        // are ones need to be removed, because they didn't match
        // any in the new blueprint.
        for old_symlink in &mut old_blueprint_symlinks {
            let _s =
                tracing::trace_span!("iter_one_remaning", ?old_symlink)
                    .entered();
            let Some(old_symlink) = old_symlink.take() else {
                continue;
            };
            Step::Remove { old_symlink }.tap_trace().pipe(|it| {
                steps.push(it);
            });
        }

        ensure!(
            new_blueprint_symlinks
                .iter()
                .chain(old_blueprint_symlinks.iter())
                .all(Option::is_none),
            "[BUG] symlinks are not completely drained"
        );

        Ok(Self { steps })
    }
}

impl Iterator for StepQueue {
    type Item = Step;
    fn next(&mut self) -> Option<Self::Item> {
        self.steps.pop()
    }
}

/// The step to be taken.
/// N.B. Best effort [TOC/TOU](https://w.wiki/GQE) prevention.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    #[inline]
    pub fn dry_execute(self) -> AnyResult<()> {
        self.real_execute(true)
    }

    #[inline]
    pub fn execute(self) -> AnyResult<()> {
        self.real_execute(false)
    }

    #[tracing::instrument(name = "step_execute", skip(self))]
    fn real_execute(self, dry: bool) -> AnyResult<()> {
        trace!(?self);
        match self {
            Self::Create { new_symlink } => {
                Self::create_symlink(new_symlink, dry)?;
            }

            Self::Replace {
                new_symlink,
                old_symlink,
            } => Self::replace_symlink(new_symlink, old_symlink, dry)?,

            Self::Remove { old_symlink } => {
                Self::remove_symlink(old_symlink, dry)?;
            }

            Self::Nothing => {
                let _s = tracing::trace_span!("nothig_to_do").entered();
                debug!("do nothing");
            }
        }
        Ok(())
    }

    #[tracing::instrument]
    #[inline]
    fn create_symlink(new_symlink: Symlink, dry: bool) -> AnyResult<()> {
        let Symlink { src, dst } = new_symlink;
        let dst_fact = DstFact::check(&src, &dst)?;

        if dst_fact.is_collision() {
            debug!("dst collides");
            bail!(
                r#"Symlink target "{}" is occupied by another file"#,
                dst.display()
            );
        }

        // N.B. early return
        if dry {
            debug!("dry run");
            return Ok(());
        }

        debug!("not dry run, do symlink");

        if matches!(dst_fact, DstFact::SymlinkToSrc) {
            debug!("dst points to src already, nothing to do");
            return Ok(());
        }

        if let Some(parent) = dst.parent() {
            Self::create_parent_dirs(parent)?;
        }

        debug!("ready to create the real symlink");
        symlink(&src, &dst).with_context(|| {
            format!(r#"Failed to create symlink "{}""#, dst.display())
        })?;

        Ok(())
    }

    #[tracing::instrument]
    #[inline]
    fn replace_symlink(
        new_symlink: Symlink,
        old_symlink: Symlink,
        dry: bool,
    ) -> AnyResult<()> {
        let Symlink {
            src: new_src,
            dst: new_dst,
        } = new_symlink;
        let Symlink {
            src: old_src,
            dst: old_dst,
        } = old_symlink;

        ensure!(new_dst == old_dst, "[BUG] new_dst not equals to old_dst");

        let dst = new_dst;
        drop(old_dst);
        let dst_fact = DstFact::check(&old_src, &dst)?;

        if dst_fact.is_collision() {
            debug!("dst collides");
            bail!(
                r#"Symlink target "{}" is not controlled by us, \
                refuse to replace"#,
                dst.display(),
            );
        }

        // If dst does not exist, replace essentially becomes create
        // with extra steps
        if matches!(dst_fact, DstFact::NotExist) {
            debug!("dst not exist, ignore");
        }

        // N.B. early rerun
        if dry {
            debug!("dry run");
            return Ok(());
        }

        debug!("not dry run, replace symlink");

        if new_src == old_src {
            debug!("srcs are the same, nothing to replace");
            return Ok(());
        }

        // attempt to atomic replace
        let tmp_dst = {
            use rand::distr::Alphanumeric;
            trace!("generate temporary dst");
            let suffix = rand::rng()
                .sample_iter(&Alphanumeric)
                .take(6)
                .map(char::from)
                .collect::<String>();
            let ostr =
                dst.as_os_str().to_owned().tap_mut(|it| it.push(suffix));
            PathBuf::from(ostr).tap_trace()
        };
        symlink(new_src, &tmp_dst).with_context(|| {
            format!(
                r#"Failed to link to the temporary target "{}", \
                    the existing symlink is intact"#,
                tmp_dst.display(),
            )
        })?;
        // posix says it's atomic
        let rename_ret = rename(&tmp_dst, &dst).with_context(|| {
            format!(r#"Failed to replace symlink "{}""#, dst.display())
        });
        if rename_ret.is_err() {
            debug!("error when renaming symlink, remove tmp file");
            remove_file(&tmp_dst).context(
                "Failed to remove intermediate symlink, \
                    your filesystem might be cooked",
            )?;
        }
        Ok(())
    }

    fn remove_symlink(old_symlink: Symlink, dry: bool) -> AnyResult<()> {
        let Symlink { src, dst } = old_symlink;
        let dst_fact = DstFact::check(&src, &dst)?;

        if dst_fact.is_collision() {
            debug!("dst collides");
            bail!(
                r#"Symlink target "{}" is not controlled by us, \
                refuse to remove"#,
                dst.display(),
            );
        }

        // N.B. early return
        if dry {
            debug!("dry run");
            return Ok(());
        }

        debug!("not dry run, remove symlink");

        if matches!(dst_fact, DstFact::NotExist) {
            debug!("dst not exist, do nothing");
            return Ok(());
        }

        debug!("ready to remove the old symlink");
        remove_file(&dst).with_context(|| {
            format!(r#"Failed to remove symlink "{}""#, dst.display())
        })?;

        if let Some(parent) = dst.parent() {
            Self::remove_empty_parent_dirs(parent)?;
        }

        Ok(())
    }

    #[inline]
    #[tracing::instrument]
    fn create_parent_dirs(path: &Path) -> AnyResult<()> {
        debug!("attempt to create parent dirs");
        std::fs::create_dir_all(path).with_context(|| {
            format!(
                r#"Failed to create parent directories of "{}""#,
                path.display()
            )
        })?;
        Ok(())
    }

    #[inline]
    #[tracing::instrument]
    fn remove_empty_parent_dirs(path: &Path) -> AnyResult<()> {
        debug!("attempt to remove empty parent dirs");
        trace!(?path);
        for ances in path.ancestors() {
            trace!(?ances, "parent's ancestor");
            let metadata = ances
                .symlink_metadata()
                .context("Failed to read ancestor metadata")?;
            if metadata.is_dir() && ances.read_dir()?.next().is_none() {
                debug!("ancestor dir is empty, remove it");
                std::fs::remove_dir( ances )
                    .with_context( || format!(
                        r#"Failed to remove empty ancestor directory "{}""#,
                        ances.display()
                    ) )?;
            } else {
                debug!("not empty, skip remaining ancestors");
                return Ok(());
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub enum DstFact {
    /// It's solid collision between two totally unrelated files.
    Exist,
    /// Same a [`Self::Collide`] but in addition this signals
    /// `dst` is a symlink but it doesn't point to our `src`.
    SymlinkNotSrc,
    /// `dst` is occupied by a symlink but that symlink is pointing
    /// to our `src`.
    SymlinkToSrc,
    /// Nothing is occupying the `dst`.
    NotExist,
}

impl DstFact {
    #[inline]
    #[tracing::instrument(name = "dst_fact_check")]
    pub fn check(src: &Path, dst: &Path) -> AnyResult<Self> {
        debug!("check potential collision");
        // N.B. Don't use [`Path::exists`] because it follows symlink
        if dst.try_exists_no_traverse()? {
            debug!("dst is occupied");
            if dst.is_symlink() {
                debug!("dst is a symlink, do further checks");
                if dst.read_link()? == src {
                    debug!("dst symlink is ours");
                    Ok(Self::SymlinkToSrc)
                } else {
                    debug!("dst symlink doesn't point to our src");
                    Ok(Self::SymlinkNotSrc)
                }
            } else {
                debug!("dst is not a symlink, it can't be ours");
                Ok(Self::Exist)
            }
        } else {
            debug!("dst is clear from collision");
            Ok(Self::NotExist)
        }
    }

    pub fn is_collision(&self) -> bool {
        match self {
            Self::Exist | Self::SymlinkNotSrc => true,
            Self::SymlinkToSrc | Self::NotExist => false,
        }
    }
}

#[allow(clippy::unwrap_used)]
#[cfg(test)]
mod test {

    use super::*;
    use crate::template::RenderedPath;

    use assert_fs::TempDir;
    use assert_fs::prelude::*;

    use std::fs::remove_file;
    use std::os::unix::fs::symlink;

    #[macro_export]
    macro_rules! make_tempdir {
        () => {{ TempDir::new().expect("Failed to setup tempdir") }};
    }

    // TODO: move to blueprint.rs and make it public?
    #[macro_export]
    macro_rules! make_symlink {
        () => {{ make_symlink!("/ssrc", "/ddst") }};
        ( $src:expr ) => {
            make_symlink!($src, "/ddst")
        };
        ( $src:expr, $dst:expr ) => {{
            let src = RenderedPath::from_unrendered($src)
                .expect("Failed to make src RenderedPath");
            let dst = RenderedPath::from_unrendered($dst)
                .expect("Failed to make dst RenderedPath");
            Symlink::new_test(src, dst)
        }};
    }

    macro_rules! make_random_str {
        () => {{
            use rand::distr::Alphanumeric;
            rand::rng()
                .sample_iter(&Alphanumeric)
                .take(8)
                .map(char::from)
                .collect::<String>()
        }};
    }

    #[test]
    fn generate_steps() {
        // no step
        {
            let new_bp = Blueprint::empty();
            let old_bp = Blueprint::empty();
            let q = StepQueue::new(new_bp, old_bp);
            assert!(q.is_ok_and(|it| it.steps.is_empty()));
        }
        // create
        {
            let sym = make_symlink!();
            let new_bp = Blueprint::empty()
                .tap_mut(|it| it.symlinks = vec![sym.clone()]);
            let old_bp = Blueprint::empty();
            let q = StepQueue::new(new_bp, old_bp);
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
                .tap_mut(|it| it.symlinks = vec![sym.clone()]);
            let q = StepQueue::new(new_bp, old_bp);
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
            let new_symlink = make_symlink!("/src_new", "/dst");
            let old_symlink = make_symlink!("/src_old", "/dst");

            let new_bp = Blueprint::empty()
                .tap_mut(|it| it.symlinks = vec![new_symlink.clone()]);
            let old_bp = Blueprint::empty()
                .tap_mut(|it| it.symlinks = vec![old_symlink.clone()]);
            let q = StepQueue::new(new_bp, old_bp);
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
            let new_symlink = make_symlink!("/src_x", "/dst");
            let old_symlink = make_symlink!("/src_x", "/dst");

            let new_bp = Blueprint::empty()
                .tap_mut(|it| it.symlinks = vec![new_symlink.clone()]);
            let old_bp = Blueprint::empty()
                .tap_mut(|it| it.symlinks = vec![old_symlink.clone()]);
            let q = StepQueue::new(new_bp, old_bp);
            assert! {
                q.is_ok_and( |mut it| {
                    it.steps.len() == 1
                    && it.steps.pop().unwrap() == Step::Nothing
                } )
            };
        }
        // Mixed
        {
            let unc_symlink = make_symlink!("/uncha", "/unch_dst");
            let new_symlink = make_symlink!("/src_new_1", "/dst_1");
            let del_symlink = make_symlink!("/src_old", "/dst_dd");
            let rep_symlink_old =
                make_symlink!("/src_ooo", "/dst_replace");
            let rep_symlink_new =
                make_symlink!("/src_yee", "/dst_replace");

            let new_bp = Blueprint::empty().tap_mut(|it| {
                it.symlinks = vec![
                    unc_symlink.clone(),
                    new_symlink.clone(),
                    rep_symlink_new.clone(),
                ];
            });

            let old_bp = Blueprint::empty().tap_mut(|it| {
                it.symlinks = vec![
                    unc_symlink.clone(),
                    del_symlink.clone(),
                    rep_symlink_old.clone(),
                ];
            });

            let q = StepQueue::new(new_bp, old_bp);
            assert!(q.is_ok());
            let q = q.unwrap();
            assert!(q.steps.len() == 4);
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

    #[test]
    fn check_collision() {
        let top = make_tempdir!();
        let src = top.child("src");
        let dst = top.child("dst");

        // 1. collide
        dst.touch().unwrap();
        assert! {
            matches!(
                DstFact::check( src.path(), dst.path() ).unwrap(),
                DstFact::Exist
            )
        };
        remove_file(dst.path()).unwrap();

        // 2. symlink collide
        symlink("/yeebie", dst.path()).unwrap();
        assert! {
            matches!(
                DstFact::check( src.path(), dst.path() ).unwrap(),
                DstFact::SymlinkNotSrc
            )
        };
        remove_file(dst.path()).unwrap();

        // 3. our symlink
        symlink(src.path(), dst.path()).unwrap();
        assert! {
            matches!(
                DstFact::check( src.path(), dst.path() ).unwrap(),
                DstFact::SymlinkToSrc
            )
        };
        remove_file(dst.path()).unwrap();

        // 4. coast is clear
        assert! {
            matches!(
                DstFact::check( src.path(), dst.path() ).unwrap(),
                DstFact::NotExist
            )
        };
    }

    #[test]
    fn create_symlink() {
        let top = make_tempdir!();
        let src =
            top.child(make_random_str!()).tap(|it| it.touch().unwrap());
        let dst = top.child(make_random_str!());

        let sym =
            make_symlink!(src.to_str().unwrap(), dst.to_str().unwrap());
        let step = Step::Create { new_symlink: sym };

        // 1. create symlink normally
        assert!(step.clone().execute().is_ok());
        // TODO structural error
        assert!(
            dst.path().is_symlink()
                && dst.path().read_link().unwrap() == src.path()
        );

        // 2. Our symlinks (it has been executed once, dst now is to src)
        assert!(step.execute().is_ok());

        // 3. dst is symlink but not ours
        let sym = make_symlink!("/bbbbbr", dst.path().to_str().unwrap());
        let step = Step::Create { new_symlink: sym };
        remove_file(dst.path()).unwrap();
        symlink(src.path(), dst.path()).unwrap();
        assert!(step.execute().is_err());

        // 4. create missing parent dirs
        {
            // don't create the dir
            let dir = top.child(make_random_str!());
            let src = top
                .child(make_random_str!())
                .tap(|it| it.touch().unwrap());
            let dst = dir.child(make_random_str!());

            let s = make_symlink!(
                src.to_str().unwrap(),
                dst.to_str().unwrap()
            );
            let s = Step::Create { new_symlink: s };

            assert!(s.execute().is_ok());
            assert!(dir.try_exists_no_traverse().unwrap());
            assert!(dir.symlink_metadata().unwrap().is_dir());
            assert!(dst.read_link().unwrap() == src.path());
        }
    }

    #[test]
    fn remove_symlink() {
        let top = make_tempdir!();
        let src = top.child("src").tap(|it| it.touch().unwrap());
        let dst = top.child("dst");

        let sym =
            make_symlink!(&src.to_str().unwrap(), &dst.to_str().unwrap());
        let step = Step::Remove { old_symlink: sym };

        // 1. normal case
        symlink(&src, &dst).unwrap();
        assert!(step.clone().execute().is_ok());
        assert!(!dst.try_exists().unwrap());

        // 2. Not our symlinks
        // the dst is removed last step, this symlink call
        // shouldn't fail because of "file already exists"
        symlink("/", &dst).unwrap();
        assert!(step.clone().execute().is_err());
        assert!(dst.try_exists_no_traverse().unwrap());

        // 3. dst already deleted
        remove_file(&dst).unwrap();
        assert!(step.execute().is_ok());

        // 4. clean up the remaining dirs
        {
            // don't create the dir
            let dir = top
                .child(make_random_str!())
                .tap(|it| it.create_dir_all().unwrap());
            let dir_dir = dir
                .child(make_random_str!())
                .tap(|it| it.create_dir_all().unwrap());
            let dir_dir_dir = dir_dir
                .child(make_random_str!())
                .tap(|it| it.create_dir_all().unwrap());

            let no_touch_text = make_random_str!();
            let no_touch = dir_dir
                .child(make_random_str!())
                .tap(|it| it.write_str(&no_touch_text).unwrap());

            let src = top
                .child(make_random_str!())
                .tap(|it| it.touch().unwrap());
            let dst = dir_dir_dir
                .child(make_random_str!())
                .tap(|it| it.symlink_to_file(&src).unwrap());

            let s = make_symlink!(
                src.to_str().unwrap(),
                dst.to_str().unwrap()
            );
            let s = Step::Remove { old_symlink: s };

            assert!(s.execute().is_ok());

            // Dir and dir_dir shouldn't be touched because
            // they are not empty
            assert!(dir.try_exists_no_traverse().unwrap());
            assert!(dir_dir.try_exists_no_traverse().unwrap());
            // but dir_dir_dir should be removed
            assert!(!dir_dir_dir.try_exists_no_traverse().unwrap());

            assert!(!dst.try_exists_no_traverse().unwrap());
            assert!(
                std::fs::read_to_string(no_touch).unwrap()
                    == no_touch_text
            );
        }
    }

    #[test]
    fn replace_symlink() {
        // 0. erroneous data
        {
            let new_symlink = make_symlink!("/yjay", "/ann");
            let old_symlink = make_symlink!("/yjay", "/buffoon");
            let s = Step::Replace {
                new_symlink,
                old_symlink,
            };
            assert!({
                let ret = s.execute();
                ret.is_err()
                    && ret.err().unwrap().to_string().contains("BUG")
            });
        }
        // 1. normal case
        {
            let top = make_tempdir!();
            let old_src =
                top.child("old_src").tap(|it| it.touch().unwrap());
            let new_src = top.child("src").tap(|it| it.touch().unwrap());
            let dst = top.child("dst");

            symlink(&old_src, &dst).unwrap();

            let new_symlink = make_symlink!(
                &new_src.to_str().unwrap(),
                &dst.to_str().unwrap()
            );
            let old_symlink = make_symlink!(
                &old_src.to_str().unwrap(),
                &dst.to_str().unwrap()
            );
            let s = Step::Replace {
                new_symlink,
                old_symlink,
            };

            assert!(s.execute().is_ok());
            assert!(dst.read_link().unwrap().as_path() == new_src.path());
        }
        // 2. not ours
        {
            let top = make_tempdir!();
            let old_src =
                top.child("old_src").tap(|it| it.touch().unwrap());
            let new_src = top.child("src").tap(|it| it.touch().unwrap());
            let dst = top.child("dst");

            let new_symlink = make_symlink!(
                &new_src.to_str().unwrap(),
                &dst.to_str().unwrap()
            );
            let old_symlink = make_symlink!(
                &old_src.to_str().unwrap(),
                &dst.to_str().unwrap()
            );
            let s = Step::Replace {
                new_symlink,
                old_symlink,
            };

            let trdsrc = top.child("trd").tap(|it| it.touch().unwrap());
            symlink(&trdsrc, &dst).unwrap();

            assert!(s.execute().is_err());
            assert!(dst.read_link().unwrap() == trdsrc.path());
        }
        // 3. subdirs
        {
            let top = make_tempdir!();
            let dir = top
                .child(make_random_str!())
                .tap(|it| it.create_dir_all().unwrap());

            let old_src = top
                .child(make_random_str!())
                .tap(|it| it.touch().unwrap());
            let new_src = top
                .child(make_random_str!())
                .tap(|it| it.touch().unwrap());

            let dst = dir
                .child(make_random_str!())
                .tap(|it| it.symlink_to_file(&old_src).unwrap());

            let new_symlink = make_symlink!(
                &new_src.to_str().unwrap(),
                &dst.to_str().unwrap()
            );
            let old_symlink = make_symlink!(
                &old_src.to_str().unwrap(),
                &dst.to_str().unwrap()
            );
            let s = Step::Replace {
                new_symlink,
                old_symlink,
            };

            assert!(s.execute().is_ok());
            assert!(dir.symlink_metadata().unwrap().is_dir());
        }
    }
}
