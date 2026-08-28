//! The `upwards` shell contract: stdout carries the toplevel path exactly
//! when the exit status is success.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::tests_outside_test_module,
    reason = "Tests"
)]

use assert_fs::TempDir;
use assert_fs::fixture::ChildPath;
use assert_fs::prelude::*;
use std::process::Command;

fn upwards() -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_derputils"))
        .arg("upwards")
        .output()
        .unwrap()
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
fn output_iff_exit_success() {
    // The applet reads the CWD, so the whole journey runs from one test
    // to keep the process-wide CWD transitions race-free.
    let original = std::env::current_dir().unwrap();
    let top = TempDir::new().unwrap();

    // No boundary anywhere near: no output, exit status 1.
    let plain = top.child("plain");
    plain.create_dir_all().unwrap();
    std::env::set_current_dir(plain.path()).unwrap();
    let not_found = upwards();
    assert_eq!(not_found.status.code(), Some(1));
    assert!(not_found.stdout.is_empty());
    assert!(not_found.stderr.is_empty());

    // Inside a repository: the toplevel on stdout, exit status 0.
    let repo = top.child("repo");
    plant_git_files(&repo.child(".git"));
    let nested = repo.child("deep/nested");
    nested.create_dir_all().unwrap();
    std::env::set_current_dir(nested.path()).unwrap();
    let found = upwards();
    assert!(found.status.success());
    assert_eq!(
        String::from_utf8_lossy(&found.stdout),
        format!("{}\n", repo.path().display())
    );

    std::env::set_current_dir(original).unwrap();
}
