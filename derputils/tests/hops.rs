//! The `hops` shell contract: a failure keeps the hops already walked,
//! error reports carry only the real cause, and an absent start path
//! fails without output.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::tests_outside_test_module,
    reason = "Tests"
)]

use std::os::unix::fs::symlink;
use std::path::Path;
use std::process::Command;

use assert_fs::TempDir;
use assert_fs::prelude::*;

fn hops(program: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_derputils"))
        .args(["hops"])
        .arg(program)
        .output()
        .unwrap()
}

#[test]
fn loop_failure_keeps_the_walked_chain() {
    let tmp = TempDir::new().unwrap();
    let link_a = tmp.child("loop-a");
    let link_b = tmp.child("loop-b");
    symlink(link_b.path(), link_a.path()).unwrap();
    symlink(link_a.path(), link_b.path()).unwrap();

    let output = hops(link_a.path());
    assert_eq!(output.status.code(), Some(1));

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("loop-a"), "{stdout}");
    assert!(stdout.contains("loop-b"), "{stdout}");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Symlink loop detected"), "{stderr}");
    // No generic wrapper layer around the real cause.
    assert!(!stderr.contains("Unable to walk through"), "{stderr}");
}

#[test]
fn missing_start_path_fails_without_output() {
    let tmp = TempDir::new().unwrap();
    let output = hops(tmp.child("absent").path());

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("does not exist"), "{stderr}");
}
