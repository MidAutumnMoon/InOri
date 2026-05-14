use tracing::debug;
use tracing::trace;

use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result as AnyResult;
use ino_color::cprint;
use ino_color::fg;
use ino_color::style;

const MAX_SYMLINK_FOLLOWS: u64 = 64;

fn main() -> AnyResult<()> {
    ino_tracing::init_tracing_subscriber();
    <App as clap::Parser>::parse().run()
}

/// Find executable in $PATH, and print each ancestor in its
/// symlink chain.
#[derive(clap::Parser)]
#[derive(Debug)]
struct App {
    /// The name of executable to find in $PATH.
    /// If it starts with "/", "../" or "./", the symlink walk
    /// will start with it directly instead of lookup an
    /// executable in $PATH.
    program: String,
}

impl App {
    #[tracing::instrument]
    fn run(&self) -> anyhow::Result<()> {
        let starter = if self.program.contains('/') {
            PathBuf::from(&self.program)
        } else {
            let errmsg = || {
                anyhow::anyhow!(r#"Program "{}" not found"#, &self.program)
            };
            coruma::lookup_executable_in_path(&self.program)
                .first()
                .ok_or_else(errmsg)?
                .to_owned()
        };

        debug!(?starter);

        let ancestors = SymlinkAncestor::new(&starter)
            .collect::<Result<Vec<_>, _>>()
            .context("Unable to walk through symlink")?;

        explain_paths(&ancestors)?;

        Ok(())
    }
}

#[derive(Debug)]
struct SymlinkAncestor {
    current: Option<PathBuf>,
    visited_paths: HashSet<PathBuf>,
    symlink_followed: u64,
}

impl SymlinkAncestor {
    fn new(starter: &Path) -> Self {
        Self {
            current: Some(starter.into()),
            visited_paths: HashSet::default(),
            symlink_followed: 0,
        }
    }
}

impl Iterator for SymlinkAncestor {
    type Item = anyhow::Result<PathBuf>;

    fn next(&mut self) -> Option<Self::Item> {
        let _s = tracing::debug_span!("symlink_iter_next").entered();

        // N.B. self.current became None after take()
        // it stays None as long as not set again
        let current = self.current.take()?;
        debug!(?current);

        // Check for symlink loop
        if self.visited_paths.contains(&current) {
            debug!("Already visited this path");
            return Some(Err(anyhow::anyhow!(
                r#"Symlink loop detected, path: "{}""#,
                current.display()
            )));
        }

        // Follow symlink if applicable
        if current.is_symlink() {
            if self.symlink_followed >= MAX_SYMLINK_FOLLOWS {
                return Some(Err(anyhow::anyhow!(
                    "Exceeded the maximum symlink follows allowed"
                )));
            }
            self.symlink_followed += 1;

            debug!("Found new symlink");
            let errmsg = || {
                format!(r#"Error reading symlink "{}""#, current.display())
            };
            let symlink_target =
                match current.read_link().with_context(errmsg) {
                    Ok(it) => it,
                    Err(err) => return Some(Err(err)),
                };
            // Resolve relative targets against the symlink's
            // parent directory. Without this, read_link()
            // returns the raw target (e.g. "../bin/foo") which
            // would be resolved against CWD on the next
            // iteration — silently following the wrong path.
            // This also ensures all stored paths are absolute,
            // so loop detection via HashSet comparison works
            // correctly (relative paths that resolve to the
            // same file would otherwise be distinct PathBufs).
            let next = if symlink_target.is_relative() {
                current
                    .parent()
                    .map(|dir| dir.join(&symlink_target))
                    .map(|p| path_clean::PathClean::clean(&p))
                    .unwrap_or(symlink_target)
            } else {
                symlink_target
            };
            self.current = Some(next);
        } else {
            trace!("Not a symlink, the end of symlink chain is reached");
        }

        self.visited_paths.insert(current.clone());

        Some(Ok(current))
    }
}

#[derive(Debug, Clone, Copy)]
enum SubjectKind {
    BootedSystem,
    CurrentSystem,
    NixStore,
    Normal,
    PerUserProfile,
    Relative,
}

#[derive(Debug)]
struct Subject {
    kind: SubjectKind,
    path: PathBuf,
}

impl Subject {
    fn new_guess(path: &Path) -> Self {
        #[allow(clippy::enum_glob_use)]
        use SubjectKind::*;

        const CHECKLIST: &[(&str, SubjectKind)] = &[
            ("/nix/store", NixStore),
            ("/etc/profiles/per-user", PerUserProfile),
            ("/run/current-system", CurrentSystem),
            ("/run/booted-system", BootedSystem),
        ];

        let kind = if path.is_absolute() {
            CHECKLIST
                .iter()
                .find(|(prefix, _)| path.starts_with(prefix))
                .map_or(Normal, |(_, kind)| *kind)
        } else {
            Relative
        };

        Self {
            kind,
            path: path.to_owned(),
        }
    }

    fn fix_relative(self, base: &Path) -> Self {
        // Note: `base` is assumed to be an absolute directory.
        // This holds because SymlinkAncestor resolves relative
        // symlink targets against their parent directory,
        // so paths in the ancestors vec are always absolute.
        if !matches!(self.kind, SubjectKind::Relative) {
            return self;
        }
        let cleaned = path_clean::PathClean::clean(&base.join(&self.path));
        Self::new_guess(&cleaned)
    }

    fn describe(&self) -> &'static str {
        #[allow(clippy::enum_glob_use)]
        use SubjectKind::*;
        match self.kind {
            BootedSystem => "The generation activated at boot time",
            CurrentSystem => "The current activated generation",
            NixStore => "Path in nix store",
            Normal => "Ordinary path",
            PerUserProfile => "Per user profile",
            Relative => "Relative path",
        }
    }
}

#[tracing::instrument]
fn explain_paths(paths: &[PathBuf]) -> anyhow::Result<()> {
    for (index, it) in paths.iter().enumerate() {
        trace!(?it);

        let subject = match Subject::new_guess(it) {
            // Try to fix up relative paths.
            it @ Subject {
                kind: SubjectKind::Relative,
                ..
            } => {
                debug!("Fixup relative path");
                let dirname = index
                    .checked_sub(1)
                    .and_then(|idx| paths.get(idx))
                    .and_then(|prev| prev.parent());
                if let Some(dirname) = dirname {
                    it.fix_relative(dirname)
                } else {
                    it
                }
            }
            anything => anything,
        };

        cprint!(fg::Blue, "{}", subject.path.display());
        if !matches!(subject.kind, SubjectKind::Normal) {
            cprint!(
                (fg::Default, style::Italic),
                " <- {}",
                subject.describe()
            );
        }
        println!();
    }

    Ok(())
}
