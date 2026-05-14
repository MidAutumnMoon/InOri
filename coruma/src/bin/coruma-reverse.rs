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

/// A `PathBuf` guaranteed to be absolute and cleaned
/// (no `.` or `..` segments).
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct AbsolutePath(PathBuf);

impl AbsolutePath {
    /// Resolve a possibly-relative path to absolute.
    /// Relative paths are resolved against CWD.
    /// The result is cleaned via `PathClean`.
    fn resolve(path: &Path) -> AnyResult<Self> {
        let absolute = if path.is_absolute() {
            path.to_owned()
        } else {
            std::env::current_dir()
                .context("Unable to determine current directory")?
                .join(path)
        };
        Ok(Self(path_clean::PathClean::clean(&absolute)))
    }

    /// Resolve a symlink target relative to this path's
    /// parent directory.
    ///
    /// If `target` is absolute, it is wrapped and cleaned.
    /// If relative, it is joined with this path's parent
    /// directory and cleaned, producing an absolute path.
    fn resolve_target(&self, target: &Path) -> Self {
        let resolved = if target.is_relative() {
            let parent_dir =
                self.0.parent().expect("symlink path always has a parent");
            path_clean::PathClean::clean(&parent_dir.join(target))
        } else {
            path_clean::PathClean::clean(target)
        };
        Self(resolved)
    }

    fn is_symlink(&self) -> bool {
        self.0.is_symlink()
    }

    fn read_link(&self) -> std::io::Result<PathBuf> {
        self.0.read_link()
    }
}

impl AsRef<Path> for AbsolutePath {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl std::fmt::Display for AbsolutePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.display())
    }
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
            AbsolutePath::resolve(Path::new(&self.program))?
        } else {
            let errmsg = || {
                anyhow::anyhow!(r#"Program "{}" not found"#, &self.program)
            };
            let hits = coruma::lookup_executable_in_path(&self.program);
            AbsolutePath::resolve(hits.first().ok_or_else(errmsg)?)?
        };

        debug!(?starter);

        let ancestors = SymlinkAncestor::new(starter)
            .collect::<Result<Vec<_>, _>>()
            .context("Unable to walk through symlink")?;

        explain_paths(&ancestors)?;

        Ok(())
    }
}

#[derive(Debug)]
struct SymlinkAncestor {
    current: Option<AbsolutePath>,
    visited_paths: HashSet<AbsolutePath>,
    symlink_followed: u64,
}

impl SymlinkAncestor {
    fn new(starter: AbsolutePath) -> Self {
        Self {
            current: Some(starter),
            visited_paths: HashSet::default(),
            symlink_followed: 0,
        }
    }
}

impl Iterator for SymlinkAncestor {
    type Item = anyhow::Result<AbsolutePath>;

    fn next(&mut self) -> Option<Self::Item> {
        let _s = tracing::debug_span!("symlink_iter_next").entered();

        let current = self.current.take()?;
        debug!(path = %current);

        // Check for symlink loop
        if self.visited_paths.contains(&current) {
            debug!("Already visited this path");
            return Some(Err(anyhow::anyhow!(
                r#"Symlink loop detected, path: \"{current}\""#,
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
            let errmsg =
                || format!(r#"Error reading symlink \"{current}\""#);
            let target = match current.read_link().with_context(errmsg) {
                Ok(it) => it,
                Err(err) => return Some(Err(err)),
            };
            self.current = Some(current.resolve_target(&target));
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
}

#[derive(Debug)]
struct Subject {
    kind: SubjectKind,
    path: AbsolutePath,
}

impl Subject {
    fn new(path: AbsolutePath) -> Self {
        #[allow(clippy::enum_glob_use)]
        use SubjectKind::*;

        const CHECKLIST: &[(&str, SubjectKind)] = &[
            ("/nix/store", NixStore),
            ("/etc/profiles/per-user", PerUserProfile),
            ("/run/current-system", CurrentSystem),
            ("/run/booted-system", BootedSystem),
        ];

        let kind = CHECKLIST
            .iter()
            .find(|(prefix, _)| path.as_ref().starts_with(prefix))
            .map_or(Normal, |(_, kind)| *kind);

        Self { kind, path }
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
        }
    }
}

#[tracing::instrument]
fn explain_paths(paths: &[AbsolutePath]) -> anyhow::Result<()> {
    for it in paths {
        trace!(?it);

        let subject = Subject::new(it.clone());

        cprint!(fg::Blue, "{}", subject.path);
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
