mod cli;
mod plan;
pub use cli::clean_cli;

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::SystemTime;

use regex::Regex;
use rootcause::Result;
use rootcause::option_ext::OptionExt as _;
use rootcause::prelude::ResultExt as _;
use tracing::debug;
use tracing::info;
use tracing::instrument;
use tracing::warn;

use crate::command::ElevationStrategy;
use crate::command::SudoConfig;
use crate::runtime::RuntimeEnv;
#[derive(Debug, Clone)]
pub struct Request {
    pub scope: Scope,
    pub options: Options,
}

#[derive(Debug, Clone)]
pub enum Scope {
    All,
    User,
    Profile(PathBuf),
}

#[derive(Clone, Debug)]
pub struct Options {
    pub keep: u32,
    pub keep_since: humantime::Duration,
    pub dry: bool,
    pub ask: bool,
    pub no_gc: bool,
    pub no_gcroots: bool,
    pub no_direnv: bool,
    pub optimise: bool,
    pub max: Option<String>,
    pub keep_one: bool,
    pub cross_filesystems: bool,
}

// Nix impl:
// https://github.com/NixOS/nix/blob/master/src/nix-collect-garbage/nix-collect-garbage.cc

static DIRENV_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    #[expect(
        clippy::expect_used,
        reason = "static regex literal; compilation failure is a programmer error"
    )]
    Regex::new(r".*/(?:\.direnv|direnv/layouts)/.*")
        .expect("Failed to compile direnv regex")
});

static GENERATION_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    #[expect(
        clippy::expect_used,
        reason = "static regex literal; compilation failure is a programmer error"
    )]
    Regex::new(r"^(.*)-(\d+)-link$")
        .expect("Failed to compile generation regex")
});

static RESULT_LINK_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    #[expect(
        clippy::expect_used,
        reason = "static regex literal; compilation failure is a programmer error"
    )]
    Regex::new("^result(-.*)?$")
        .expect("Failed to compile result link regex")
});

const AUTO_GCROOTS_DIR: &str = "/nix/var/nix/gcroots/auto";

#[derive(Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
struct Generation {
    number: u32,
    last_modified: SystemTime,
    path: PathBuf,
}

type ToBeRemoved = bool;
// BTreeMap to automatically sort generations by id
type GenerationsTagged = BTreeMap<Generation, ToBeRemoved>;
type ProfilesTagged = HashMap<PathBuf, GenerationsTagged>;

#[derive(Debug)]
struct GcRootTagged {
    src: PathBuf,
    dst: PathBuf,
    tbr: ToBeRemoved,
}

/// Filter paths to only include existing directories, logging warnings for
/// missing ones.
fn filter_existing_dirs<I>(paths: I) -> impl Iterator<Item = PathBuf>
where
    I: IntoIterator<Item = PathBuf>,
{
    paths.into_iter().filter_map(|path| {
        if path.is_dir() {
            Some(path)
        } else {
            warn!(
                "Profiles directory not found, skipping: {}",
                path.display()
            );
            None
        }
    })
}

/// Run a clean request.
///
/// # Errors
///
/// Returns an error if any IO, Nix, or environment operation fails.
pub fn run(
    request: &Request,
    elevate: ElevationStrategy,
    runtime_env: &RuntimeEnv,
    sudo_config: &SudoConfig,
) -> Result<()> {
    plan::run(request, elevate, runtime_env, sudo_config)
}

#[instrument(ret, level = "debug")]
fn profiles_in_dir<P>(dir: P) -> Vec<PathBuf>
where
    P: AsRef<Path> + fmt::Debug,
{
    let mut res = Vec::new();
    let dir = dir.as_ref();

    match dir.read_dir() {
        Ok(read_dir) => {
            for entry in read_dir {
                match entry {
                    Ok(err) => {
                        let path = err.path();

                        if let Ok(dst) = path.read_link() {
                            let name = if let Some(file_name) =
                                dst.file_name()
                            {
                                file_name.to_string_lossy()
                            } else {
                                warn!(
                                    "Failed to get filename for {dst:?}"
                                );
                                continue;
                            };

                            if GENERATION_REGEX.captures(&name).is_some() {
                                res.push(path);
                            }
                        }
                    }
                    Err(error) => {
                        warn!(
                            ?dir,
                            ?error,
                            "Failed to read folder element"
                        );
                    }
                }
            }
        }
        Err(error) => {
            warn!(?dir, ?error, "Failed to read profiles directory");
        }
    }

    res
}

#[instrument(err, level = "debug")]
fn cleanable_generations(
    profile: &Path,
    keep: u32,
    keep_since: humantime::Duration,
) -> Result<GenerationsTagged> {
    let name = profile
        .file_name()
        .context("Checking profile's name")?
        .to_str()
        .context("Profile name is not valid UTF-8")?;

    let mut result = GenerationsTagged::new();

    for entry in profile
        .parent()
        .context("Reading profile's parent dir")?
        .read_dir()
        .context("Reading profile's generations")?
    {
        let path = entry?.path();
        let captures = {
            let file_name =
                path.file_name().context("Failed to get filename")?;
            let file_name_str = file_name
                .to_str()
                .context("Filename is not valid UTF-8")?;
            GENERATION_REGEX.captures(file_name_str)
        };

        if let Some(caps) = captures {
            // Check if this generation belongs to the current profile
            if let Some(profile_name) = caps.get(1)
                && profile_name.as_str() != name
            {
                continue;
            }
            if let Some(number) = caps.get(2) {
                let last_modified = path
                    .symlink_metadata()
                    .context("Checking symlink metadata")?
                    .modified()
                    .context("Reading modified time")?;

                result.insert(
                    Generation {
                        number: number.as_str().parse().context(
                            "Failed to parse generation number",
                        )?,
                        last_modified,
                        path,
                    },
                    true,
                );
            }
        }
    }

    let now = SystemTime::now();
    for (generation, tbr) in &mut result {
        match now.duration_since(generation.last_modified) {
            Err(err) => {
                warn!(?err, ?now, ?generation, "Failed to compare time!");
            }
            Ok(val) if val <= std::time::Duration::from(keep_since) => {
                *tbr = false;
            }
            Ok(_) => {}
        }
    }

    for (_, tbr) in result.iter_mut().rev().take(keep as usize) {
        *tbr = false;
    }

    debug!("{:#?}", result);
    Ok(result)
}

fn is_nix_store_direct_child(path: &Path) -> bool {
    path.strip_prefix("/nix/store")
        .is_ok_and(|suffix| suffix.components().count() == 1)
}

fn gcroot_matches_filter(
    src: &Path,
    dst: &Path,
    regexes: &[&Regex],
) -> bool {
    let resolved_dst = if dst.is_symlink() {
        dst.read_link().unwrap_or_else(|_| dst.to_path_buf())
    } else {
        dst.to_path_buf()
    };

    regexes
        .iter()
        .any(|next| next.is_match(&dst.to_string_lossy()))
        || (is_auto_gcroot_entry(src)
            && is_nix_store_direct_child(&resolved_dst)
            && is_build_result_link(dst))
}

fn is_auto_gcroot_entry(path: &Path) -> bool {
    path.starts_with(AUTO_GCROOTS_DIR)
}

/// Whether `path`'s basename looks like an ephemeral `nix build` result
/// symlink (`result`, `result-dev`, etc.), as opposed to a live indirect
/// gcroot such as `current-system`. Only paths matching this are subject
/// to age-based (`--keep-since`) cleanup; anything else that resolves
/// straight into `/nix/store` is left alone unless it becomes orphaned.
fn is_build_result_link(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| RESULT_LINK_REGEX.is_match(name))
}

fn gcroot_path_to_remove(gcroot: &GcRootTagged) -> &Path {
    &gcroot.src
}

fn remove_path_nofail(path: &Path) {
    info!("Removing {}", path.to_string_lossy());
    if let Err(err) = std::fs::remove_file(path) {
        warn!(?path, ?err, "Failed to remove path");
    }
}

#[cfg(test)]
#[expect(clippy::expect_used, reason = "Tests")]
mod tests {
    use super::*;

    #[test]
    fn store_direct_child_accepts_top_level_entry() {
        assert!(is_nix_store_direct_child(Path::new(
            "/nix/store/abc123zzz-foo-1.0"
        )));
    }

    #[test]
    fn store_direct_child_rejects_nested_path() {
        assert!(!is_nix_store_direct_child(Path::new(
            "/nix/store/abc123zzz-foo-1.0/bin/foo"
        )));
    }

    #[test]
    fn store_direct_child_rejects_store_root_itself() {
        assert!(!is_nix_store_direct_child(Path::new("/nix/store")));
    }

    #[test]
    fn store_direct_child_rejects_unrelated_paths() {
        assert!(!is_nix_store_direct_child(Path::new(
            "/home/user/result"
        )));
        assert!(!is_nix_store_direct_child(Path::new("/result")));
        assert!(!is_nix_store_direct_child(Path::new(
            "/nix/store-backup/abc"
        )));
    }

    #[test]
    fn direnv_regex_matches_dotdirenv_subpath() {
        assert!(
            DIRENV_REGEX.is_match("/home/user/project/.direnv/python3.11")
        );
    }

    #[test]
    fn direnv_regex_matches_layouts_subpath() {
        assert!(
            DIRENV_REGEX
                .is_match("/home/user/project/direnv/layouts/python3.11")
        );
    }

    #[test]
    fn direnv_regex_rejects_result_and_store_paths() {
        assert!(!DIRENV_REGEX.is_match("/home/user/project/result"));
        assert!(!DIRENV_REGEX.is_match("/nix/store/abc123zzz-foo-1.0"));
    }

    #[test]
    fn gcroot_filter_passes_direnv_path() {
        let src = Path::new("/nix/var/nix/gcroots/project-direnv");
        let dst = Path::new("/home/user/project/.direnv/something");
        let regexes = [&*DIRENV_REGEX];
        assert!(gcroot_matches_filter(src, dst, &regexes));
    }

    #[test]
    fn gcroot_filter_rejects_auto_store_direct_child_without_result_name()
    {
        // A direct-store-child target that isn't reached through a
        // `result`-shaped path (e.g., a raw store path registered as an
        // indirect root) is not eligible for age-based cleanup.
        let src = Path::new("/nix/var/nix/gcroots/auto/example");
        let dst = Path::new("/nix/store/abc123zzz-foo-1.0");
        let regexes = [&*DIRENV_REGEX];
        assert!(!gcroot_matches_filter(src, dst, &regexes));
    }

    #[test]
    fn gcroot_filter_rejects_live_system_shaped_auto_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        let link = dir.path().join("current-system");
        std::os::unix::fs::symlink("/nix/store/abc123zzz-foo-1.0", &link)
            .expect("symlink");

        let src = Path::new("/nix/var/nix/gcroots/auto/example");
        let regexes = [&*DIRENV_REGEX];
        assert!(
            !gcroot_matches_filter(src, &link, &regexes),
            "current-system-shaped roots must not be subject to age-based \
             cleanup"
        );
    }

    #[test]
    fn build_result_link_matches_result_and_variants() {
        assert!(is_build_result_link(Path::new(
            "/home/user/project/result"
        )));
        assert!(is_build_result_link(Path::new(
            "/home/user/project/result-dev"
        )));
        assert!(is_build_result_link(Path::new(
            "/home/user/project/result-bin"
        )));
    }

    #[test]
    fn build_result_link_rejects_current_system() {
        assert!(!is_build_result_link(Path::new("/run/current-system")));
        assert!(!is_build_result_link(Path::new(
            "/nix/store/abc123zzz-foo-1.0"
        )));
    }

    #[test]
    fn gcroot_filter_rejects_non_auto_store_direct_child() {
        let src = Path::new("/nix/var/nix/gcroots/current-system");
        let dst = Path::new("/nix/store/abc123zzz-foo-1.0");
        let regexes = [&*DIRENV_REGEX];
        assert!(!gcroot_matches_filter(src, dst, &regexes));
    }

    #[test]
    fn gcroot_filter_passes_auto_symlink_to_store_direct_child() {
        let dir = tempfile::tempdir().expect("tempdir");
        let link = dir.path().join("result");
        std::os::unix::fs::symlink("/nix/store/abc123zzz-foo-1.0", &link)
            .expect("symlink");

        let src = Path::new("/nix/var/nix/gcroots/auto/example");
        let regexes = [&*DIRENV_REGEX];
        assert!(gcroot_matches_filter(src, &link, &regexes));
    }

    #[test]
    fn direct_store_filter_is_limited_to_auto_gcroots() {
        assert!(is_auto_gcroot_entry(Path::new(
            "/nix/var/nix/gcroots/auto/example"
        )));
        assert!(!is_auto_gcroot_entry(Path::new(
            "/nix/var/nix/gcroots/current-system"
        )));
        assert!(!is_auto_gcroot_entry(Path::new(
            "/nix/var/nix/gcroots/booted-system"
        )));
        assert!(!is_auto_gcroot_entry(Path::new(
            "/nix/var/nix/gcroots/profiles/system"
        )));
    }

    #[test]
    fn gcroot_cleanup_removes_source_not_system_destination() {
        let gcroot = GcRootTagged {
            src: PathBuf::from("/nix/var/nix/gcroots/auto/example"),
            dst: PathBuf::from("/run/current-system"),
            tbr: true,
        };

        assert_eq!(
            gcroot_path_to_remove(&gcroot),
            Path::new("/nix/var/nix/gcroots/auto/example")
        );
        assert_ne!(gcroot_path_to_remove(&gcroot), gcroot.dst.as_path());
    }

    #[test]
    fn gcroot_cleanup_removes_source_not_profile_destination() {
        let gcroot = GcRootTagged {
            src: PathBuf::from("/nix/var/nix/gcroots/auto/example"),
            dst: PathBuf::from("/nix/var/nix/profiles/system-2-link"),
            tbr: true,
        };

        assert_eq!(
            gcroot_path_to_remove(&gcroot),
            Path::new("/nix/var/nix/gcroots/auto/example")
        );
        assert_ne!(gcroot_path_to_remove(&gcroot), gcroot.dst.as_path());
    }

    #[test]
    fn gcroot_filter_rejects_arbitrary_path() {
        let src = Path::new("/nix/var/nix/gcroots/auto/example");
        let dst = Path::new("/home/user/some-random-link");
        let regexes = [&*DIRENV_REGEX];
        assert!(!gcroot_matches_filter(src, dst, &regexes));
    }

    #[test]
    fn missing_path_triggers_case_a_condition() {
        let dir = tempfile::tempdir().expect("tempdir");
        let gone = dir.path().join("gone");
        assert!(!gone.is_symlink() && !gone.exists());
    }

    #[test]
    fn broken_symlink_does_not_trigger_case_a() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("gone");
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&target, &link).expect("symlink");
        assert!(link.is_symlink());
        assert!(!link.exists());
        assert!(link.is_symlink() || link.exists());
    }

    #[test]
    fn broken_symlink_metadata_fails() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("gone");
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&target, &link).expect("symlink");

        assert!(link.is_symlink(), "symlink should exist");
        assert!(
            link.metadata().is_err(),
            "following broken symlink should fail"
        );
    }

    #[test]
    fn orphaned_live_system_shaped_root_is_still_removable() {
        // Even though a current-system-shaped root is exempt from age-based
        // cleanup (also see: gcroot_filter_rejects_live_system_shaped_auto_root),
        // once its target is actually gone it must still be detected as
        // broken. This is sorta universal, and filter independent.
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("gone-system");
        let link = dir.path().join("current-system");
        std::os::unix::fs::symlink(&target, &link).expect("symlink");

        assert!(!is_build_result_link(&link));
        assert!(
            link.metadata().is_err(),
            "broken current-system-shaped symlink must be detected as orphaned \
       regardless of its name"
        );
    }

    #[test]
    fn live_symlink_metadata_succeeds() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("real");
        std::fs::write(&target, b"").expect("write");
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&target, &link).expect("symlink");

        assert!(link.is_symlink(), "symlink should exist");
        assert!(
            link.metadata().is_ok(),
            "live symlink metadata should succeed"
        );
    }
}
