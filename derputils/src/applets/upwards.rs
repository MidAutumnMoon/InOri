//! `upwards` — print the "toplevel" directory of the CWD.
//! Prints the absolute toplevel on stdout, or exits with status 1 and no
//! output when there is no upward boundary.

use std::env::current_dir;
use std::ffi::OsString;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;

use bpaf::OptionParser;
use bpaf::Parser as _;
use gix_discover::upwards;
use rootcause::prelude::ResultExt as _;
use tracing::debug;
use tracing::instrument;
use tracing::trace;

use super::applet::Applet;
use super::applet::invoke;

const NAME: &str = "upwards";
const SUMMARY: &str = "Find upward boundaries that `feel` right";
pub(super) const APPLET: Applet = Applet::new(NAME, SUMMARY, entry);

const NIX_STORE: &str = "/nix/store";

fn cli() -> OptionParser<()> {
    bpaf::pure(()).to_options().descr(SUMMARY)
}

fn entry(args: &[OsString]) -> ExitCode {
    invoke(&cli(), NAME, args, |()| run())
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

#[derive(Debug, PartialEq, Eq)]
enum Decision {
    StayHere,
    GoUpward(PathBuf),
}

/// `None` means the heuristic does not apply, deferring to the next one.
type HeuristicResult = rootcause::Result<Option<Decision>>;

#[instrument]
fn heuristic_nix_store(cwd: &Path) -> HeuristicResult {
    let Some(rest) = cwd.strip_prefix(NIX_STORE).ok() else {
        trace!("CWD is outside of the nix store, skipping this heuristic");
        return Ok(None);
    };

    let Some(package) = rest.components().next() else {
        return Ok(Some(Decision::StayHere));
    };

    Ok(Some(Decision::GoUpward(Path::new(NIX_STORE).join(package))))
}

#[instrument]
fn heuristic_git(cwd: &Path) -> HeuristicResult {
    let (repo, _trust) = match upwards(cwd) {
        Ok(found) => found,
        Err(err) if err.is_not_found() => {
            trace!(
                "CWD is not inside a git repository, skipping this heuristic"
            );
            return Ok(None);
        }
        Err(err) => {
            Err(err).context("Failed to search for a git repository")?
        }
    };

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
        assert_eq!(nix_decision("/nix/store"), Some(Decision::StayHere));
        assert_eq!(nix_decision("/nix"), None);
        assert_eq!(nix_decision("/nix/storefoo"), None);
        assert_eq!(nix_decision("/home/teapot"), None);

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
