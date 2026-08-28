//! `hops` — trace a symlink to its origin.
//!
//! Prints the whole chain of an executable or a path: the path itself,
//! every symlink hop, and the final target, annotating well-known
//! Nix-related locations.

use std::collections::HashSet;
use std::ffi::OsString;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;

use bpaf::Args;
use bpaf::OptionParser;
use bpaf::Parser as _;
use bpaf::positional;
use ino_color::cprint;
use ino_color::fg;
use ino_color::style;
use ino_path::is_executable::IsExecutable as _;
use rootcause::prelude::ResultExt as _;
use rootcause::report;
use tracing::debug;
use tracing::instrument;
use tracing::trace;

use crate::applet::RunFailure;

pub const NAME: &str = "hops";
pub const DESCR: &str = "Trace a symlink to its origin";

/// The most symlink hops allowed before assuming an unbroken loop.
const MAX_SYMLINK_FOLLOWS: u64 = 64;

pub fn applet_main(args: &[OsString]) -> Result<ExitCode, RunFailure> {
    let program = cli()
        .run_inner(Args::from(args).set_name(NAME))
        .map_err(RunFailure::Cli)?;
    run(&program).map_err(RunFailure::Applet)?;
    Ok(ExitCode::SUCCESS)
}

#[must_use]
pub fn cli() -> OptionParser<String> {
    positional::<String>("PROGRAM")
        .help(
            "Executable name to find in $PATH; a path containing '/' \
             starts the walk directly instead",
        )
        .to_options()
        .descr(DESCR)
}

#[instrument]
fn run(program: &str) -> rootcause::Result<()> {
    let starter = if program.contains('/') {
        AbsolutePath::resolve(Path::new(program))?
    } else {
        let hit = find_in_path(program)
            .ok_or_else(|| report!(r#"Program "{program}" not found"#))?;
        AbsolutePath::resolve(&hit)?
    };
    debug!(?starter);

    let ancestors = SymlinkAncestor::new(starter)
        .collect::<rootcause::Result<Vec<_>>>()
        .context("Unable to walk through the symlink chain")?;

    explain_paths(&ancestors);

    Ok(())
}

/// Finds the first executable named `program` in `$PATH`.
///
/// Returns `None` when `$PATH` is unset or nothing matches, leaving
/// the failure message to the caller which knows the program name.
fn find_in_path(program: &str) -> Option<PathBuf> {
    let env_path = std::env::var_os("PATH")?;

    std::env::split_paths(&env_path)
        .map(|dir| dir.join(program))
        .find(|candidate| {
            trace!(?candidate, "Looking into");
            candidate.is_executable()
        })
        .inspect(|hit| debug!(?hit, "Found executable"))
}

/// A `PathBuf` guaranteed to be absolute and cleaned
/// (no `.` or `..` segments).
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct AbsolutePath(PathBuf);

impl AbsolutePath {
    /// Resolve a possibly-relative path to absolute.
    /// Relative paths are resolved against CWD.
    /// The result is cleaned via `PathClean`.
    fn resolve(path: &Path) -> rootcause::Result<Self> {
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
            #[expect(
                clippy::expect_used,
                reason = "absolute paths always have a parent"
            )]
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
    type Item = rootcause::Result<AbsolutePath>;

    fn next(&mut self) -> Option<Self::Item> {
        let _s = tracing::debug_span!("symlink_iter_next").entered();

        let current = self.current.take()?;
        debug!(path = %current);

        // Check for symlink loop
        if self.visited_paths.contains(&current) {
            debug!("Already visited this path");
            return Some(Err(report!(
                r#"Symlink loop detected, path: "{current}""#
            )));
        }

        // Follow symlink if applicable
        if current.is_symlink() {
            if self.symlink_followed >= MAX_SYMLINK_FOLLOWS {
                return Some(Err(report!(
                    "Exceeded the maximum symlink follows allowed"
                )));
            }
            self.symlink_followed += 1;

            debug!("Found new symlink");
            let errmsg =
                || format!(r#"Error reading symlink "{current}""#);
            let target = match current.read_link().context_with(errmsg) {
                Ok(it) => it,
                Err(err) => return Some(Err(err.into())),
            };
            self.current = Some(current.resolve_target(&target));
        } else {
            trace!("Not a symlink, the end of symlink chain is reached");
        }

        self.visited_paths.insert(current.clone());

        Some(Ok(current))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
        // N.B. #[expect] has subtle bugs with enum_glob_use :/
        #![allow(clippy::enum_glob_use, reason = "Nicer to work with")]

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
        #![allow(clippy::enum_glob_use, reason = "Nicer to work with")]

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

fn explain_paths(paths: &[AbsolutePath]) {
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
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "Tests")]
mod test {
    use std::assert_matches;
    use std::os::unix::fs::symlink;

    use assert_fs::TempDir;
    use assert_fs::prelude::*;

    use super::*;

    fn walk(start: &Path) -> rootcause::Result<Vec<AbsolutePath>> {
        let starter = AbsolutePath::resolve(start)?;
        SymlinkAncestor::new(starter).collect()
    }

    fn parse(args: &[&str]) -> Result<String, bpaf::ParseFailure> {
        cli().run_inner(Args::from(args).set_name(NAME))
    }

    #[test]
    fn program_argument_is_required() {
        assert_matches!(parse(&[]), Err(bpaf::ParseFailure::Stderr(_)));
        parse(&["ls"]).unwrap();
        parse(&["/bin/sh"]).unwrap();
    }

    #[test]
    fn plain_file_chain_is_just_itself() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.child("file");
        file.write_str("x").unwrap();

        let chain = walk(file.path()).unwrap();
        assert_eq!(chain, vec![AbsolutePath::resolve(file.path()).unwrap()]);
    }

    #[test]
    fn walks_absolute_symlink_chain() {
        let tmp = TempDir::new().unwrap();
        let real = tmp.child("real");
        real.write_str("x").unwrap();
        let mid = tmp.child("mid");
        let start = tmp.child("start");
        symlink(real.path(), mid.path()).unwrap();
        symlink(mid.path(), start.path()).unwrap();

        let chain = walk(start.path()).unwrap();
        let rendered: Vec<String> =
            chain.iter().map(ToString::to_string).collect();
        assert_eq!(
            rendered,
            vec![
                start.path().to_string_lossy().into_owned(),
                mid.path().to_string_lossy().into_owned(),
                real.path().to_string_lossy().into_owned(),
            ]
        );
    }

    #[test]
    fn resolves_relative_target_against_parent() {
        let tmp = TempDir::new().unwrap();
        let real = tmp.child("real");
        real.write_str("x").unwrap();
        tmp.child("dir").create_dir_all().unwrap();
        let link = tmp.child("dir/link");
        symlink("../real", link.path()).unwrap();

        let chain = walk(link.path()).unwrap();
        let expected = vec![
            AbsolutePath::resolve(link.path()).unwrap(),
            AbsolutePath::resolve(real.path()).unwrap(),
        ];
        assert_eq!(chain, expected);
    }

    #[test]
    fn detects_symlink_loop() {
        let tmp = TempDir::new().unwrap();
        let link_a = tmp.child("a");
        let link_b = tmp.child("b");
        symlink(link_b.path(), link_a.path()).unwrap();
        symlink(link_a.path(), link_b.path()).unwrap();

        let result = walk(link_a.path());
        assert_matches!(result, Err(_));
        let message = result.unwrap_err().to_string();
        assert!(message.contains("Symlink loop detected"), "{message}");
    }

    #[test]
    fn stops_at_max_follow_limit() {
        let tmp = TempDir::new().unwrap();
        let real = tmp.child("real");
        real.write_str("x").unwrap();

        let links: Vec<_> = (0..=MAX_SYMLINK_FOLLOWS)
            .map(|i| tmp.child(format!("link{i}")))
            .collect();
        for (i, link) in links.iter().enumerate() {
            let target = links
                .get(i + 1)
                .map_or_else(|| real.path(), |next| next.path());
            symlink(target, link.path()).unwrap();
        }

        let result = walk(links.first().unwrap().path());
        assert_matches!(result, Err(_));
        let message = result.unwrap_err().to_string();
        assert!(
            message.contains("maximum symlink follows"),
            "{message}"
        );
    }

    #[test]
    fn classifies_well_known_prefixes() {
        let kind_of = |path: &str| {
            Subject::new(AbsolutePath::resolve(Path::new(path)).unwrap()).kind
        };

        assert_eq!(
            kind_of("/nix/store/abc-hello/bin/hello"),
            SubjectKind::NixStore
        );
        assert_eq!(
            kind_of("/etc/profiles/per-user/teapot/bin/ls"),
            SubjectKind::PerUserProfile
        );
        assert_eq!(
            kind_of("/run/current-system/sw/bin/ls"),
            SubjectKind::CurrentSystem
        );
        assert_eq!(
            kind_of("/run/booted-system/sw/bin/ls"),
            SubjectKind::BootedSystem
        );
        assert_eq!(kind_of("/home/teapot"), SubjectKind::Normal);
        // Prefixes match whole path components only.
        assert_eq!(kind_of("/nix/storefoo"), SubjectKind::Normal);
        assert_eq!(kind_of("/run/current-systemd"), SubjectKind::Normal);
    }

    #[test]
    fn describes_nix_store() {
        let subject = Subject::new(
            AbsolutePath::resolve(Path::new("/nix/store/abc-hello")).unwrap(),
        );
        assert_eq!(subject.describe(), "Path in nix store");
    }
}
