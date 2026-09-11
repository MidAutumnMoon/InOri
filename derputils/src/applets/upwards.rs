//! `upwards` — print the "toplevel" directory of the CWD.
//!
//! Exits successfully with the absolute toplevel path on stdout, or with
//! status 1 and no output when there is no upward boundary to report.

use std::env::current_dir;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;

use bpaf::OptionParser;
use gix_discover::upwards;
use gix_discover::upwards::Error as UpwardsError;
use rootcause::prelude::ResultExt as _;
use tracing::debug;
use tracing::instrument;
use tracing::trace;

use super::Applet;
use super::Invocation;

const NAME: &str = "upwards";
const SUMMARY: &str = "Find upward boundaries that `feel` right";
pub(super) const APPLET: Applet = Applet::new(NAME, cli);
const NIX_STORE: &str = "/nix/store";

/// The applet's CLI: probe the CWD, no arguments.
fn cli() -> OptionParser<Invocation> {
    Invocation::cli(bpaf::pure(()), SUMMARY, |()| run())
}

#[instrument]
fn run() -> rootcause::Result<ExitCode> {
    let cwd = current_dir().context("Failed to get CWD")?;

    // Heuristics run cheapest-first: pure path comparison before parent
    // traversal with IO.
    let decision = match heuristic_nix_store(&cwd)? {
        Some(decision) => decision,
        None => heuristic_git(&cwd)?.unwrap_or(Decision::StayHere),
    };
    debug!(?decision);

    match decision {
        // Fail silently, so a shell wrapper can tell "found" from "not
        // found" by the exit status alone.
        Decision::StayHere => Ok(ExitCode::FAILURE),
        Decision::GoUpward(toplevel) => {
            print_toplevel(&toplevel)?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// Writes `toplevel` to stdout as raw bytes. Callers consume the output
/// as a path, and `Display` would lossily replace non-UTF-8 path bytes.
fn print_toplevel(toplevel: &Path) -> rootcause::Result<()> {
    use std::io::Write as _;
    use std::io::stdout;

    let mut out = stdout().lock();
    out.write_all(toplevel.as_os_str().as_encoded_bytes())
        .and_then(|()| out.write_all(b"\n"))
        .context("Failed to write the toplevel path")?;
    Ok(())
}

/// What a heuristic concluded about the CWD.
#[derive(Debug, PartialEq, Eq)]
enum Decision {
    /// No upward path to report; the caller stays put.
    StayHere,
    /// Print this path as the toplevel of the CWD.
    GoUpward(PathBuf),
}

/// What one heuristic concludes about the CWD: `None` means it does not
/// apply, deferring the decision to the next heuristic.
type HeuristicResult = rootcause::Result<Option<Decision>>;

/// Classifies the CWD inside the nix store.
#[instrument]
fn heuristic_nix_store(cwd: &Path) -> HeuristicResult {
    let Some(rest) = cwd.strip_prefix(NIX_STORE).ok() else {
        trace!("CWD is outside of the nix store, skipping this heuristic");
        return Ok(None);
    };

    // The store itself is already the desired toplevel.
    let Some(package) = rest.components().next() else {
        return Ok(Some(Decision::StayHere));
    };

    // Anything below a store path belongs to the package it names.
    Ok(Some(Decision::GoUpward(Path::new(NIX_STORE).join(package))))
}

/// Classifies the CWD inside a git repository.
#[instrument]
fn heuristic_git(cwd: &Path) -> HeuristicResult {
    let (repo, _trust) = match upwards(cwd) {
        Ok(found) => found,
        // Discovery completed without finding a repository we may use:
        // nothing found, the search stopped at a filesystem boundary, or
        // the candidate was discarded for dubious ownership.
        Err(
            UpwardsError::NoGitRepository { .. }
            | UpwardsError::NoGitRepositoryWithinFs { .. }
            | UpwardsError::NoTrustedGitRepository { .. },
        ) => {
            trace!(
                "CWD is not inside a git repository, skipping this heuristic"
            );
            return Ok(None);
        }
        Err(err) => {
            Err(err).context("Failed to search for a git repository")?
        }
    };

    // Bare repositories have no work tree; the git dir is their toplevel.
    let (git_dir, work_dir) =
        repo.into_repository_and_work_tree_directories();
    Ok(Some(Decision::GoUpward(work_dir.unwrap_or(git_dir))))
}

#[cfg(test)]
#[expect(clippy::unwrap_used, reason = "Tests")]
mod test {
    use super::*;
    use assert_fs::TempDir;
    use assert_fs::fixture::ChildPath;
    use assert_fs::prelude::*;

    fn nix_decision(path: &str) -> Option<Decision> {
        heuristic_nix_store(Path::new(path)).unwrap()
    }

    #[test]
    fn nix_store_heuristic() {
        // The store itself and anything outside it stay put...
        assert_eq!(nix_decision("/nix/store"), Some(Decision::StayHere));
        assert_eq!(nix_decision("/nix"), None);
        assert_eq!(nix_decision("/nix/storefoo"), None);
        assert_eq!(nix_decision("/home/teapot"), None);

        // ...while everything below a store path belongs to the package.
        assert_eq!(
            nix_decision("/nix/store/abc-glibc-2.39"),
            Some(Decision::GoUpward("/nix/store/abc-glibc-2.39".into()))
        );
        assert_eq!(
            nix_decision("/nix/store/abc-glibc-2.39/share/man"),
            Some(Decision::GoUpward("/nix/store/abc-glibc-2.39".into()))
        );
    }

    /// The minimum `.git` layout `gix-discover` accepts as a repository.
    fn plant_git_files(git_dir: &ChildPath) {
        git_dir.create_dir_all().unwrap();
        git_dir
            .child("HEAD")
            .write_str("ref: refs/heads/main\n")
            .unwrap();
        git_dir.child("objects").create_dir_all().unwrap();
        git_dir.child("refs").create_dir_all().unwrap();
    }

    #[test]
    fn git_work_tree_resolves_to_its_root() {
        let repo = TempDir::new().unwrap();
        plant_git_files(&repo.child(".git"));
        let nested = repo.child("deep/nested");
        nested.create_dir_all().unwrap();

        // From the root itself and from any directory below it.
        for start in [repo.path(), nested.path()] {
            assert_eq!(
                heuristic_git(start).unwrap(),
                Some(Decision::GoUpward(repo.path().to_path_buf()))
            );
        }
    }

    #[test]
    fn inside_git_dir_toplevel_is_work_tree() {
        let repo = TempDir::new().unwrap();
        plant_git_files(&repo.child(".git"));
        let inner = repo.child(".git/objects");

        assert_eq!(
            heuristic_git(inner.path()).unwrap(),
            Some(Decision::GoUpward(repo.path().to_path_buf()))
        );
    }

    #[test]
    fn bare_git_repo_toplevel_is_git_dir() {
        let tmp = TempDir::new().unwrap();
        let bare = tmp.child("bare.git");
        plant_git_files(&bare);
        let inner = bare.child("objects");

        assert_eq!(
            heuristic_git(inner.path()).unwrap(),
            Some(Decision::GoUpward(bare.to_path_buf()))
        );
    }

    #[test]
    fn outside_git_repo_is_unclassified() {
        let tmp = TempDir::new().unwrap();
        tmp.child("plain").create_dir_all().unwrap();
        assert_eq!(
            heuristic_git(tmp.child("plain").path()).unwrap(),
            None
        );
    }
}
