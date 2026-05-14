use tracing::debug;
use tracing::trace;

use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;

use std::fmt::Display;
use std::iter::Iterator;

use anyhow::Context;
use anyhow::Result as AnyResult;
use ino_color::cprint;
use ino_color::fg;
use ino_color::style;
use tap::Pipe;

const MAX_SYMLINK_FOLLOWS: u64 = 64;

fn main() -> AnyResult<()> {
    ino_tracing::init_tracing_subscriber();
    <App as clap::Parser>::parse().run()
}

///  Find executable in $PATH, and print each ancestor in its symlink chain.
#[derive(clap::Parser)]
#[derive(Debug)]
struct App {
    /// The name of executable to find in $PATH.
    /// If it starts with "/", "../" or "./", the symlink walk
    /// will start with it directly instead of lookup an executable in $PATH.
    program: String,
}

enum ProgramKind {
    ExeName(String),
    SomePath(String),
}

impl ProgramKind {
    fn new(input: &str) -> Self {
        if input.contains('/') {
            Self::SomePath(input.into())
        } else {
            Self::ExeName(input.into())
        }
    }
}

impl App {
    #[tracing::instrument]
    fn run(&self) -> anyhow::Result<()> {
        trace!("Start application");

        let starter = match ProgramKind::new(&self.program) {
            ProgramKind::ExeName(name) => {
                let errmsg =
                    || anyhow::anyhow!(r#"Program "{}" not found"#, &name);
                coruma::lookup_executable_in_path(&name)
                    .first()
                    .ok_or_else(errmsg)?
                    .to_owned()
            }
            ProgramKind::SomePath(it) => PathBuf::from(it),
        };

        debug!(?starter);

        let ancestors = SymlinkAncestor::new(&starter)
            .collect::<Result<Vec<_>, _>>()
            .context("Unable to walk through symlink")?;

        Explainer::explain_paths(&ancestors)?;

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

        // Step 1. Check for symlink loop

        if self.visited_paths.contains(&current) {
            debug!("Already visited this path");
            return anyhow::anyhow!(
                r#"Symlink loop detected, path: "{}""#,
                current.display()
            )
            .pipe(Err)
            .pipe(Some);
        }

        // Step 2. Prepare for next iteration

        // is_symlink() does not traverse symlink
        if current.is_symlink() {
            // Guard: only count actual symlink follows
            if self.symlink_followed >= MAX_SYMLINK_FOLLOWS {
                return anyhow::anyhow!(
                    "Exceeded the maximum symlink follows allowed"
                )
                .pipe(Err)
                .pipe(Some);
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
            // parent directory. Without this, read_link() returns
            // the raw target (e.g. "../bin/foo") which would be
            // resolved against CWD on the next iteration —
            // silently following the wrong path.
            // This also ensures all stored paths are absolute,
            // so loop detection via HashSet comparison works
            // correctly (relative paths that resolve to the same
            // file would otherwise be distinct PathBufs).
            let next = if symlink_target.is_relative() {
                current
                    .parent()
                    .map(|dir| dir.join(&symlink_target))
                    .map(|p| path_clean::PathClean::clean(&p))
                    .unwrap_or(symlink_target)
            } else {
                symlink_target
            };
            // Set self.current to Some,
            // so that the next iteration will happen
            self.current = Some(next);
        } else {
            // Here, self.current is not set and stays None,
            // which skips next iteration
            trace!("Not a symlink, the end of symlink chain is reached");
        }

        // Step 4. Book current as traversed and yield

        self.visited_paths.insert(current.clone());

        Some(Ok(current))
    }
}

#[derive(Debug, Clone)]
enum SubjectKind {
    BootedSystem,
    CurrentSystem,
    NixStore,
    Normal,
    PerUserProfile,
    /// A special entry whose meaning depends on the context.
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

        let it = path.to_string_lossy();

        let kind = if path.is_absolute() {
            CHECKLIST
                .iter()
                .find(|(prefix, _)| it.starts_with(prefix))
                .map_or(&SubjectKind::Normal, |(_, kind)| kind)
                .to_owned()
        } else {
            Relative
        };

        Self {
            kind,
            path: path.to_owned(),
        }
    }

    fn fix_relative(self, base: &Path) -> anyhow::Result<Self> {
        // Note: `base` is assumed to be an absolute directory.
        // This holds because SymlinkAncestor resolves relative
        // symlink targets against their parent directory,
        // so paths in the ancestors vec are always absolute.
        if !matches!(self.kind, SubjectKind::Relative) {
            return Ok(self);
        }
        base.join(self.path)
            .pipe_ref(path_clean::PathClean::clean)
            .pipe_as_ref(Self::new_guess)
            .pipe(Ok)
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

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Display for Subject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.path().display())?;
        if !matches!(self.kind, SubjectKind::Normal) {
            write!(f, " <- {}", self.describe())?;
        }
        Ok(())
    }
}

struct Explainer;

impl Explainer {
    #[tracing::instrument]
    fn explain_paths(paths: &[PathBuf]) -> anyhow::Result<()> {
        for (index, it) in paths.iter().enumerate() {
            trace!(?it);

            let subject = match Subject::new_guess(it) {
                // Try it's best to fix up relative path.
                it @ Subject {
                    kind: SubjectKind::Relative,
                    ..
                } => {
                    debug!("Fixup relative path");
                    if let Some(dirname) = index
                        // get the index of previous item
                        .checked_sub(1)
                        // get the previous path
                        .and_then(|idx| paths.get(idx))
                        // get the parent aka dirname
                        .and_then(|prev| prev.parent())
                    {
                        it.fix_relative(dirname)?
                    } else {
                        // If nothing works, meh just give up
                        it
                    }
                }
                anything => anything,
            };

            cprint!(fg::Blue, "{}", subject.path().display());
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
}
