use std::collections::VecDeque;
use std::fs::remove_file;
use std::fs::rename;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::path::PathBuf;

use crate::blueprint::Symlink;

use anyhow::Context as _;
use anyhow::Result as AnyResult;
use anyhow::bail;
use anyhow::ensure;
use ino_path::PathExt as _;
use itertools::Itertools as _;
use rand::RngExt as _;
use tap::Tap as _;
use tracing::debug;
use tracing::info;
use tracing::trace;

#[derive(Debug, Clone)]
pub struct StepQueue {
    steps: VecDeque<Step>,
}

impl StepQueue {
    #[tracing::instrument(name = "step_queue_new", skip_all)]
    pub fn new(
        new_generation: Vec<Symlink>,
        old_generation: Vec<Symlink>,
    ) -> AnyResult<Self> {
        info!("Actualize blueprint");
        debug!("actualize blueprint into steps");
        trace!(
            new_symlinks = ?new_generation,
            old_symlinks = ?old_generation
        );

        let capacity = new_generation.len() + old_generation.len();
        let (mut new_blueprint_symlinks, mut old_blueprint_symlinks) =
            [new_generation, old_generation]
                .map(|symlinks| symlinks.into_iter().map(Some))
                .map(|iter| {
                    iter.collect_vec().tap(|symlinks| trace!(?symlinks))
                })
                .into();
        let mut planned_steps = Vec::with_capacity(capacity);

        // First resolve destinations that exist in both generations.
        for new_symlink in &mut new_blueprint_symlinks {
            let Some(new_symlink) = new_symlink.take() else {
                continue;
            };
            let mut found_old_symlink = None;

            for old_symlink in &mut old_blueprint_symlinks {
                if old_symlink
                    .as_ref()
                    .is_some_and(|old| old.same_dst(&new_symlink))
                {
                    found_old_symlink = old_symlink.take();
                    break;
                }
            }

            let step = if let Some(old_symlink) = found_old_symlink {
                if old_symlink.same_src(&new_symlink) {
                    Step::Nothing
                } else {
                    Step::Replace {
                        new_symlink,
                        old_symlink,
                    }
                }
            } else {
                Step::Create { new_symlink }
            };
            trace!(?step);
            planned_steps.push(Some(step));
        }

        // A newly managed ancestor replaces all old managed descendants as
        // one topology transition. Keeping these as independent Create and
        // Remove steps would either reject the existing directory in the
        // feasibility pass or traverse the newly created symlink on retry.
        for planned_step in &mut planned_steps {
            let Some(Step::Create {
                new_symlink: collapse_symlink,
            }) = planned_step.as_ref()
            else {
                continue;
            };
            let collapsed_symlinks = old_blueprint_symlinks
                .iter_mut()
                .filter_map(|old_symlink| {
                    if old_symlink.as_ref().is_some_and(|old_symlink| {
                        collapse_symlink.dst_is_ancestor_of(old_symlink)
                    }) {
                        old_symlink.take()
                    } else {
                        None
                    }
                })
                .collect_vec();

            if collapsed_symlinks.is_empty() {
                continue;
            }
            let Some(Step::Create {
                new_symlink: collapsed_parent,
            }) = planned_step.take()
            else {
                bail!("[BUG] collapse source is not a create step");
            };
            *planned_step = Some(Step::Collapse {
                new_symlink: collapsed_parent,
                old_symlinks: collapsed_symlinks,
            });
        }

        // The inverse transition also has to be one step: remove the old
        // ancestor before creating descendants, otherwise creation follows
        // the old symlink and writes into its source directory.
        for old_symlink in &mut old_blueprint_symlinks {
            let Some(old_symlink_ref) = old_symlink.as_ref() else {
                continue;
            };
            let mut first_new_index = None;
            let mut expanded_symlinks = Vec::new();

            for (index, planned_step) in
                planned_steps.iter_mut().enumerate()
            {
                let should_expand =
                    planned_step.as_ref().is_some_and(|planned_step| {
                        let Step::Create {
                            new_symlink: expanded_symlink,
                        } = planned_step
                        else {
                            return false;
                        };
                        old_symlink_ref
                            .dst_is_ancestor_of(expanded_symlink)
                    });
                if !should_expand {
                    continue;
                }

                let Some(Step::Create {
                    new_symlink: expanded_symlink,
                }) = planned_step.take()
                else {
                    bail!("[BUG] expansion target is not a create step");
                };
                first_new_index.get_or_insert(index);
                expanded_symlinks.push(expanded_symlink);
            }

            let Some(first_new_index) = first_new_index else {
                continue;
            };
            let Some(old_symlink) = old_symlink.take() else {
                bail!("[BUG] expansion source was already consumed");
            };
            let Some(first_new_step) =
                planned_steps.get_mut(first_new_index)
            else {
                bail!("[BUG] expansion target index is out of bounds");
            };
            *first_new_step = Some(Step::Expand {
                new_symlinks: expanded_symlinks,
                old_symlink,
            });
        }

        // Remaining old destinations are unrelated removals.
        for old_symlink in &mut old_blueprint_symlinks {
            let Some(old_symlink) = old_symlink.take() else {
                continue;
            };
            let step = Step::Remove { old_symlink };
            trace!(?step);
            planned_steps.push(Some(step));
        }

        ensure!(
            new_blueprint_symlinks
                .iter()
                .chain(old_blueprint_symlinks.iter())
                .all(Option::is_none),
            "[BUG] symlinks are not completely drained"
        );

        let steps = planned_steps.into_iter().flatten().collect();
        Ok(Self { steps })
    }

    pub fn check_feasibility(&self) -> AnyResult<()> {
        for step in &self.steps {
            step.check_feasibility()?;
        }
        Ok(())
    }
}

impl Iterator for StepQueue {
    type Item = Step;
    fn next(&mut self) -> Option<Self::Item> {
        self.steps.pop_front()
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
    /// Replace managed descendants and their directory tree with one link.
    Collapse {
        new_symlink: Symlink,
        old_symlinks: Vec<Symlink>,
    },
    /// Replace one managed ancestor link with links below its destination.
    Expand {
        new_symlinks: Vec<Symlink>,
        old_symlink: Symlink,
    },
    Nothing,
}

impl Step {
    /// Check whether this step is feasible without mutating the filesystem.
    ///
    /// N.B. Catches path topology mistakes and collisions but intentionally
    /// does not probe writability — see [`Self::ensure_creatable_topology`]
    /// for the rationale. ENOSPC, permission errors, and similar surface
    /// only at [`Self::execute`] time.
    #[inline]
    pub fn check_feasibility(&self) -> AnyResult<()> {
        self.real_execute(true)
    }

    #[inline]
    pub fn execute(self) -> AnyResult<()> {
        self.real_execute(false)
    }

    #[tracing::instrument(name = "step_execute", skip(self))]
    fn real_execute(&self, dry: bool) -> AnyResult<()> {
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
            Self::Collapse {
                new_symlink,
                old_symlinks,
            } => Self::collapse_symlinks(new_symlink, old_symlinks, dry)?,
            Self::Expand {
                new_symlinks,
                old_symlink,
            } => Self::expand_symlinks(new_symlinks, old_symlink, dry)?,
            Self::Nothing => {
                let _span =
                    tracing::trace_span!("nothing_to_do").entered();
                debug!("do nothing");
            }
        }
        Ok(())
    }

    #[tracing::instrument]
    #[inline]
    fn create_symlink(new_symlink: &Symlink, dry: bool) -> AnyResult<()> {
        let Symlink { src, dst } = new_symlink;
        let dst_fact = DstFact::check(src, dst)?;

        if dst_fact.is_collision() {
            debug!("dst collides");
            bail!(
                r#"Symlink target "{}" is occupied by another file"#,
                dst.display()
            );
        }

        // N.B. We deliberately allow src to not exist — creating links to
        // not-yet-existing targets is a legitimate use case (e.g. linking
        // before installing). Emit a trace so typos are diagnosable.
        if !src.try_exists().unwrap_or(false) {
            trace!(
                ?src,
                "src does not exist; will create a dangling symlink"
            );
        }

        if dry {
            debug!("dry run, check feasibility");
            Self::ensure_creatable_topology(dst)?;
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
        symlink(src, dst).with_context(|| {
            format!(r#"Failed to create symlink "{}""#, dst.display())
        })?;

        Ok(())
    }

    #[tracing::instrument]
    #[inline]
    fn replace_symlink(
        new_symlink: &Symlink,
        old_symlink: &Symlink,
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
        let dst_fact = DstFact::check(old_src, dst)?;

        if dst_fact.is_collision() {
            debug!("dst collides");
            bail!(
                r#"Symlink target "{}" is not controlled by us, \
                refuse to replace"#,
                dst.display(),
            );
        }

        if matches!(dst_fact, DstFact::NotExist) {
            debug!("dst not exist, ignore");
        }

        if dry {
            debug!("dry run, check feasibility");
            Self::ensure_creatable_topology(dst)?;
            return Ok(());
        }

        debug!("not dry run, replace symlink");

        if new_src == old_src {
            debug!("srcs are the same, nothing to replace");
            return Ok(());
        }

        // dst may not exist if the old blueprint was never applied or was
        // removed out of band. In that case replacement behaves like create.
        if let Some(parent) = dst.parent() {
            Self::create_parent_dirs(parent)?;
        }

        let tmp_dst = {
            use rand::distr::Alphanumeric;
            trace!("generate temporary dst");
            let suffix = rand::rng()
                .sample_iter(&Alphanumeric)
                .take(6)
                .map(char::from)
                .collect::<String>();
            let mut ostr = dst.as_os_str().to_owned();
            ostr.push(suffix);
            let tmp_dst = PathBuf::from(ostr);
            trace!(?tmp_dst);
            tmp_dst
        };
        symlink(new_src, &tmp_dst).with_context(|| {
            format!(
                r#"Failed to link to the temporary target "{}", \
                    the existing symlink is intact"#,
                tmp_dst.display(),
            )
        })?;
        let rename_ret = rename(&tmp_dst, dst).with_context(|| {
            format!(r#"Failed to replace symlink "{}""#, dst.display())
        });
        if let Err(rename_err) = rename_ret {
            debug!("error when renaming symlink, remove tmp file");
            if let Err(cleanup_err) = remove_file(&tmp_dst) {
                return Err(cleanup_err).context(format!(
                    r#"Failed to remove intermediate symlink \
                        "{}", filesystem might be cooked. \
                        Original rename error: {rename_err}"#,
                    tmp_dst.display(),
                ));
            }
            return Err(rename_err);
        }
        Ok(())
    }

    fn remove_symlink(old_symlink: &Symlink, dry: bool) -> AnyResult<()> {
        let Symlink { src, dst } = old_symlink;
        let dst_fact = DstFact::check(src, dst)?;

        if dst_fact.is_collision() {
            debug!("dst collides");
            bail!(
                r#"Symlink target "{}" is not controlled by us, \
                refuse to remove"#,
                dst.display(),
            );
        }

        if dry {
            debug!("dry run");
            return Ok(());
        }

        debug!("not dry run, remove symlink");
        if matches!(dst_fact, DstFact::NotExist) {
            debug!("dst does not exist, do nothing");
            return Ok(());
        }

        debug!("ready to remove the old symlink");
        remove_file(dst).with_context(|| {
            format!(r#"Failed to remove symlink "{}""#, dst.display())
        })?;

        if let Some(parent) = dst.parent() {
            Self::remove_empty_parent_dirs(parent)?;
        }

        Ok(())
    }

    #[tracing::instrument(skip(old_symlinks))]
    fn collapse_symlinks(
        new_symlink: &Symlink,
        old_symlinks: &[Symlink],
        dry: bool,
    ) -> AnyResult<()> {
        ensure!(
            !old_symlinks.is_empty()
                && old_symlinks.iter().all(|old_symlink| {
                    new_symlink.dst_is_ancestor_of(old_symlink)
                }),
            "[BUG] invalid collapse topology"
        );

        match DstFact::check(&new_symlink.src, &new_symlink.dst)? {
            DstFact::SymlinkToSrc => {
                // A previous run completed the collapse. Descendant paths now
                // traverse the new link and must not be treated as old links.
                debug!("collapsed symlink already exists, do nothing");
                return Ok(());
            }
            DstFact::NotExist => {
                if dry {
                    Self::ensure_creatable_topology(&new_symlink.dst)?;
                    return Ok(());
                }
                return Self::create_symlink(new_symlink, false);
            }
            DstFact::SymlinkNotSrc => {
                bail!(
                    r#"Symlink target "{}" is occupied by another symlink"#,
                    new_symlink.dst.display()
                );
            }
            DstFact::Exist => {
                let metadata =
                    new_symlink.dst.symlink_metadata().with_context(|| {
                        format!(
                            r#"Failed to inspect collapse destination "{}""#,
                            new_symlink.dst.display()
                        )
                    })?;
                ensure!(
                    metadata.is_dir(),
                    r#"Symlink target "{}" is occupied by another file"#,
                    new_symlink.dst.display()
                );
            }
        }

        // Refuse before mutation if any declared child is no longer ours or
        // any unrelated entry would keep the destination directory alive.
        for old_symlink in old_symlinks {
            Self::remove_symlink(old_symlink, true)?;
        }
        ensure!(
            Self::directory_will_be_pruned(
                &new_symlink.dst,
                old_symlinks
            )?,
            r#"Directory "{}" contains paths not controlled by lny, \
                refuse to replace it with a symlink"#,
            new_symlink.dst.display()
        );
        Self::ensure_creatable_topology(&new_symlink.dst)?;

        if dry {
            return Ok(());
        }

        for old_symlink in old_symlinks {
            Self::remove_symlink(old_symlink, false)?;
        }
        // Missing descendants may leave empty directories from an
        // interrupted prior run. This cleanup is safe here because the
        // feasibility pass proved the entire tree is managed by this
        // collapse transition.
        for old_symlink in old_symlinks {
            if let Some(parent) = old_symlink.dst.parent() {
                Self::remove_empty_parent_dirs(parent)?;
            }
        }
        Self::create_symlink(new_symlink, false)
    }

    #[tracing::instrument(skip(new_symlinks))]
    fn expand_symlinks(
        new_symlinks: &[Symlink],
        old_symlink: &Symlink,
        dry: bool,
    ) -> AnyResult<()> {
        ensure!(
            !new_symlinks.is_empty()
                && new_symlinks.iter().all(|new_symlink| {
                    old_symlink.dst_is_ancestor_of(new_symlink)
                }),
            "[BUG] invalid expansion topology"
        );

        let dst_fact = DstFact::check(&old_symlink.src, &old_symlink.dst)?;
        match dst_fact {
            DstFact::SymlinkToSrc => {
                // All descendant destinations disappear when this link is
                // removed, so checking them through the link would inspect
                // the old source tree rather than the future filesystem.
                Self::ensure_creatable_topology(&old_symlink.dst)?;
                if dry {
                    return Ok(());
                }
                Self::remove_symlink(old_symlink, false)?;
            }
            DstFact::NotExist => {}
            DstFact::Exist => {
                let metadata =
                    old_symlink.dst.symlink_metadata().with_context(|| {
                        format!(
                            r#"Failed to inspect expansion destination "{}""#,
                            old_symlink.dst.display()
                        )
                    })?;
                ensure!(
                    metadata.is_dir(),
                    r#"Symlink target "{}" is not controlled by us, \
                        refuse to expand"#,
                    old_symlink.dst.display()
                );
            }
            DstFact::SymlinkNotSrc => {
                bail!(
                    r#"Symlink target "{}" is not controlled by us, \
                        refuse to expand"#,
                    old_symlink.dst.display()
                );
            }
        }

        for new_symlink in new_symlinks {
            Self::create_symlink(new_symlink, dry)?;
        }
        Ok(())
    }

    fn directory_will_be_pruned(
        path: &Path,
        old_symlinks: &[Symlink],
    ) -> AnyResult<bool> {
        let has_managed_descendant = old_symlinks
            .iter()
            .any(|old_symlink| old_symlink.dst.starts_with(path));
        if !has_managed_descendant {
            return Ok(false);
        }

        let entries = path.read_dir().with_context(|| {
            format!(r#"Failed to inspect directory "{}""#, path.display())
        })?;
        for entry in entries {
            let entry = entry.with_context(|| {
                format!(
                    r#"Failed to inspect an entry in directory "{}""#,
                    path.display()
                )
            })?;
            let entry_path = entry.path();
            if old_symlinks.iter().any(|old_symlink| {
                old_symlink.dst.as_ref() == entry_path.as_path()
            }) {
                continue;
            }

            let file_type = entry.file_type().with_context(|| {
                format!(r#"Failed to inspect "{}""#, entry_path.display())
            })?;
            if file_type.is_dir()
                && Self::directory_will_be_pruned(
                    &entry_path,
                    old_symlinks,
                )?
            {
                continue;
            }
            return Ok(false);
        }
        Ok(true)
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

    /// Walk up from `dst`'s parent and bail if any existing ancestor
    /// is neither a directory nor a symlink. Catches typo'd paths like
    /// `/etc/hosts/foo` before the real pass commits anything.
    ///
    /// Topology-only by design: writability probing via `access(2)` is
    /// TOCTOU-prone and opens an unbounded scope (quotas, ACLs, MAC,
    /// read-only filesystems). Permission errors from the real write
    /// path produce clear OS-level messages; topology errors do not.
    #[inline]
    #[tracing::instrument]
    fn ensure_creatable_topology(dst: &Path) -> AnyResult<()> {
        debug!("check topology of dst");
        let Some(parent) = dst.parent() else {
            bail!(r#"dst "{}" has no parent"#, dst.display());
        };
        for ancestor in parent.ancestors() {
            match ancestor.symlink_metadata() {
                Ok(md) if md.is_dir() || md.is_symlink() => return Ok(()),
                Ok(_) => bail!(
                    r#"Path component "{}" exists but is not a directory, \
                        cannot create symlink at "{}""#,
                    ancestor.display(),
                    dst.display(),
                ),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    // Do nothing, skip
                }
                Err(err) => {
                    return Err(err).with_context(|| {
                        format!(
                            r#"Failed to stat ancestor "{}""#,
                            ancestor.display()
                        )
                    });
                }
            }
        }
        bail!(r#"No existing ancestor found for dst "{}""#, dst.display());
    }

    /// Walk up from `path` removing empty ancestor directories, stopping
    /// at the first non-empty one. The walk is unbounded (up to `/`) and
    /// does not track ownership — lny is stateless by design, so dirs you
    /// created manually may be pruned if they become empty. This is
    /// accepted given the assumption that all symlinks in a managed tree
    /// belong to lny.
    #[inline]
    #[tracing::instrument]
    fn remove_empty_parent_dirs(path: &Path) -> AnyResult<()> {
        debug!("attempt to remove empty parent dirs");
        trace!(?path);
        for ancestor in path.ancestors() {
            trace!(?ancestor, "parent's ancestor");
            let metadata = match ancestor.symlink_metadata() {
                Ok(metadata) => metadata,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    continue;
                }
                Err(err) => {
                    return Err(err).with_context(|| {
                        format!(
                            r#"Failed to read ancestor metadata for "{}""#,
                            ancestor.display()
                        )
                    });
                }
            };
            let is_empty_dir = metadata.is_dir()
                && ancestor
                    .read_dir()
                    .with_context(|| {
                        format!(
                            r#"Failed to read ancestor directory "{}""#,
                            ancestor.display()
                        )
                    })?
                    .next()
                    .is_none();
            if !is_empty_dir {
                debug!("not empty, skip remaining ancestors");
                return Ok(());
            }

            debug!("ancestor dir is empty, remove it");
            std::fs::remove_dir(ancestor).with_context(|| {
                format!(
                    r#"Failed to remove empty ancestor directory "{}""#,
                    ancestor.display()
                )
            })?;
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

#[expect(clippy::unwrap_used, reason = "Tests")]
#[cfg(test)]
mod test {

    use super::*;
    use crate::template::RenderedPath;
    use std::assert_matches;

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
            Symlink { src, dst }
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
            let queue = StepQueue::new(vec![], vec![]);
            assert!(queue.is_ok_and(|it| it.steps.is_empty()));
        }
        // create
        {
            let sym = make_symlink!();
            let queue = StepQueue::new(vec![sym.clone()], vec![]);
            assert! {
                queue.is_ok_and( |mut it| {
                    it.steps.len() == 1
                    && it.steps.pop_back().unwrap()
                        == Step::Create { new_symlink: sym }
                } )
            };
        }
        // remove
        {
            let sym = make_symlink!();
            let queue = StepQueue::new(vec![], vec![sym.clone()]);
            assert! {
                queue.is_ok_and( |mut it| {
                    it.steps.len() == 1
                    && it.steps.pop_back().unwrap()
                        == Step::Remove { old_symlink: sym }
                } )
            };
        }
        // Replace
        {
            let new_symlink = make_symlink!("/src_new", "/dst");
            let old_symlink = make_symlink!("/src_old", "/dst");

            let queue = StepQueue::new(
                vec![new_symlink.clone()],
                vec![old_symlink.clone()],
            );
            assert! {
                queue.is_ok_and( |mut it| {
                    it.steps.len() == 1
                    && it.steps.pop_back().unwrap()
                        == Step::Replace { new_symlink, old_symlink }
                } )
            };
        }
        // Nothing
        {
            let new_symlink = make_symlink!("/src_x", "/dst");
            let old_symlink = make_symlink!("/src_x", "/dst");

            let queue =
                StepQueue::new(vec![new_symlink], vec![old_symlink]);
            assert! {
                queue.is_ok_and( |mut it| {
                    it.steps.len() == 1
                    && it.steps.pop_back().unwrap() == Step::Nothing
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

            let new_symlinks = vec![
                unc_symlink.clone(),
                new_symlink.clone(),
                rep_symlink_new.clone(),
            ];

            let old_symlinks = vec![
                unc_symlink,
                del_symlink.clone(),
                rep_symlink_old.clone(),
            ];

            let queue = StepQueue::new(new_symlinks, old_symlinks);
            assert!(queue.is_ok());
            let queue = queue.unwrap();
            assert_eq!(queue.steps.len(), 4);
            assert! {
                queue.steps.into_iter()
                    .all( |it|
                        it == Step::Nothing
                        || it == Step::Create {
                            new_symlink: new_symlink.clone()
                        }
                        || it == Step::Remove {
                            old_symlink: del_symlink.clone()
                        }
                        || it == Step::Replace {
                            new_symlink: rep_symlink_new.clone(),
                            old_symlink: rep_symlink_old.clone()
                        } )
            };
        }
    }

    /// Regression for BUGS.md #4: the iterator must yield steps in
    /// insertion order (FIFO), not LIFO.
    #[test]
    fn step_queue_is_fifo() {
        // Push order: new-bp symlinks first (Create/Replace/Nothing),
        // then leftover old-bp symlinks (Remove).
        let new_symlinks = vec![
            make_symlink!("/a", "/dst_a"),
            make_symlink!("/b", "/dst_b"),
        ];
        let old_symlinks = vec![make_symlink!("/c", "/dst_c")];

        let mut queue =
            StepQueue::new(new_symlinks, old_symlinks).unwrap();

        assert_matches!(queue.next(), Some(Step::Create { .. }));
        assert_matches!(queue.next(), Some(Step::Create { .. }));
        assert_matches!(queue.next(), Some(Step::Remove { .. }));
        assert!(queue.next().is_none());
    }

    #[test]
    fn check_collision() {
        let top = make_tempdir!();
        let src = top.child("src");
        let dst = top.child("dst");

        // 1. collide
        dst.touch().unwrap();
        assert_matches!(
            DstFact::check(src.path(), dst.path()).unwrap(),
            DstFact::Exist
        );
        remove_file(dst.path()).unwrap();

        // 2. symlink collide
        symlink("/yeebie", dst.path()).unwrap();
        assert_matches!(
            DstFact::check(src.path(), dst.path()).unwrap(),
            DstFact::SymlinkNotSrc
        );
        remove_file(dst.path()).unwrap();

        // 3. our symlink
        symlink(src.path(), dst.path()).unwrap();
        assert_matches!(
            DstFact::check(src.path(), dst.path()).unwrap(),
            DstFact::SymlinkToSrc
        );
        remove_file(dst.path()).unwrap();

        // 4. coast is clear
        assert_matches!(
            DstFact::check(src.path(), dst.path()).unwrap(),
            DstFact::NotExist
        );
    }

    #[test]
    fn ensure_creatable_topology() {
        let top = make_tempdir!();

        // 1. parent exists and is a dir
        {
            let dst = top.child(make_random_str!());
            Step::ensure_creatable_topology(dst.path()).unwrap();
        }

        // 2. partial chain missing, no obstacle
        {
            let grandparent = top.child(make_random_str!());
            let parent = grandparent.child(make_random_str!());
            let dst = parent.child(make_random_str!());
            // parent and grandparent don't exist yet
            Step::ensure_creatable_topology(dst.path()).unwrap();
        }

        // 3. ancestor is a regular file — the bug we're catching
        {
            let file = top
                .child(make_random_str!())
                .tap(|it| it.touch().unwrap());
            let dst = file.child(make_random_str!());
            assert!(Step::ensure_creatable_topology(dst.path()).is_err());
        }

        // 4. ancestor is a symlink (deferred to OS at write time)
        {
            let real_dir = top
                .child(make_random_str!())
                .tap(|it| it.create_dir_all().unwrap());
            let link = top.child(make_random_str!());
            symlink(real_dir.path(), link.path()).unwrap();
            let dst = link.child(make_random_str!());
            Step::ensure_creatable_topology(dst.path()).unwrap();
        }
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
        step.clone().execute().unwrap();
        // TODO structural error
        assert!(
            dst.path().is_symlink()
                && dst.path().read_link().unwrap() == src.path()
        );

        // 2. Our symlinks (it has been executed once, dst now is to src)
        step.execute().unwrap();

        // 3. dst is symlink but not ours
        let foreign_sym =
            make_symlink!("/bbbbbr", dst.path().to_str().unwrap());
        let foreign_step = Step::Create {
            new_symlink: foreign_sym,
        };
        remove_file(dst.path()).unwrap();
        symlink(src.path(), dst.path()).unwrap();
        assert!(foreign_step.execute().is_err());

        // 4. create missing parent dirs
        {
            // don't create the dir
            let dir = top.child(make_random_str!());
            let nested_src = top
                .child(make_random_str!())
                .tap(|it| it.touch().unwrap());
            let nested_dst = dir.child(make_random_str!());

            let symlink = make_symlink!(
                nested_src.to_str().unwrap(),
                nested_dst.to_str().unwrap()
            );
            let symlink = Step::Create {
                new_symlink: symlink,
            };

            symlink.execute().unwrap();
            assert!(dir.try_exists_no_traverse().unwrap());
            assert!(dir.symlink_metadata().unwrap().is_dir());
            assert_eq!(nested_dst.read_link().unwrap(), nested_src.path());
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
        step.clone().execute().unwrap();
        assert!(!dst.try_exists().unwrap());

        // 2. Not our symlinks
        // the dst is removed last step, this symlink call
        // shouldn't fail because of "file already exists"
        symlink("/", &dst).unwrap();
        assert!(step.clone().execute().is_err());
        assert!(dst.try_exists_no_traverse().unwrap());

        // 3. dst already deleted
        remove_file(&dst).unwrap();
        step.execute().unwrap();

        // A missing managed link is a no-op, including its empty parent.
        {
            let empty_parent = top
                .child(make_random_str!())
                .tap(|it| it.create_dir_all().unwrap());
            let missing_dst = empty_parent.child(make_random_str!());
            let missing_symlink = make_symlink!(
                src.to_str().unwrap(),
                missing_dst.to_str().unwrap()
            );

            Step::Remove {
                old_symlink: missing_symlink,
            }
            .execute()
            .unwrap();

            assert!(empty_parent.try_exists_no_traverse().unwrap());
        }

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

            let nested_src = top
                .child(make_random_str!())
                .tap(|it| it.touch().unwrap());
            let nested_dst = dir_dir_dir
                .child(make_random_str!())
                .tap(|it| it.symlink_to_file(&nested_src).unwrap());

            let symlink = make_symlink!(
                nested_src.to_str().unwrap(),
                nested_dst.to_str().unwrap()
            );
            let nested_step = Step::Remove {
                old_symlink: symlink,
            };

            nested_step.execute().unwrap();

            // Dir and dir_dir shouldn't be touched because
            // they are not empty
            assert!(dir.try_exists_no_traverse().unwrap());
            assert!(dir_dir.try_exists_no_traverse().unwrap());
            // but dir_dir_dir should be removed
            assert!(!dir_dir_dir.try_exists_no_traverse().unwrap());

            assert!(!nested_dst.try_exists_no_traverse().unwrap());
            assert_eq!(
                std::fs::read_to_string(no_touch).unwrap(),
                no_touch_text
            );
        }
    }

    #[test]
    fn replace_symlink() {
        // 0. erroneous data
        {
            let new_symlink = make_symlink!("/yjay", "/ann");
            let old_symlink = make_symlink!("/yjay", "/buffoon");
            let step = Step::Replace {
                new_symlink,
                old_symlink,
            };
            assert!({
                let ret = step.execute();
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
            let step = Step::Replace {
                new_symlink,
                old_symlink,
            };

            step.execute().unwrap();
            assert_eq!(dst.read_link().unwrap().as_path(), new_src.path());
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
            let step = Step::Replace {
                new_symlink,
                old_symlink,
            };

            let trdsrc = top.child("trd").tap(|it| it.touch().unwrap());
            symlink(&trdsrc, &dst).unwrap();

            assert!(step.execute().is_err());
            assert_eq!(dst.read_link().unwrap(), trdsrc.path());
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
            let step = Step::Replace {
                new_symlink,
                old_symlink,
            };

            step.execute().unwrap();
            assert!(dir.symlink_metadata().unwrap().is_dir());
        }
        // 4. parent dir doesn't exist (regression for BUGS.md #1)
        {
            let top = make_tempdir!();
            let dir = top.child(make_random_str!()); // deliberately not created
            let old_src = top
                .child(make_random_str!())
                .tap(|it| it.touch().unwrap());
            let new_src = top
                .child(make_random_str!())
                .tap(|it| it.touch().unwrap());
            let dst = dir.child(make_random_str!());

            let new_symlink = make_symlink!(
                new_src.to_str().unwrap(),
                dst.to_str().unwrap()
            );
            let old_symlink = make_symlink!(
                old_src.to_str().unwrap(),
                dst.to_str().unwrap()
            );
            let step = Step::Replace {
                new_symlink,
                old_symlink,
            };

            step.execute().unwrap();
            assert!(dir.try_exists_no_traverse().unwrap());
            assert!(dir.symlink_metadata().unwrap().is_dir());
            assert!(
                dst.is_symlink()
                    && dst.read_link().unwrap() == new_src.path()
            );
        }
    }
}
