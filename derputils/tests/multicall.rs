//! The multicall contract: a symlink named after an applet runs that
//! applet's own CLI, named after the symlink rather than the dispatcher.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::tests_outside_test_module,
    reason = "Tests"
)]

use std::os::unix::fs::symlink;
use std::process::Command;

use assert_fs::TempDir;
use assert_fs::prelude::*;

/// Runs the binary through a symlink named `name`, as a multicall install
/// invokes it.
fn via_symlink(name: &str, args: &[&str]) -> std::process::Output {
    let tmp = TempDir::new().unwrap();
    let link = tmp.child(name);
    symlink(env!("CARGO_BIN_EXE_derputils"), link.path()).unwrap();

    Command::new(link.path())
        .args(args)
        // Keep the applet's output free of ambient tracing filter noise.
        .env_remove("RUST_LOG")
        .output()
        .unwrap()
}

#[test]
fn argv0_selects_the_applet_and_names_it() {
    // Going through the dispatcher would spell this `derputils qr`.
    let output = via_symlink("qr", &["--help"]);
    assert!(output.status.success());

    let help = String::from_utf8_lossy(&output.stdout);
    assert!(
        help.lines().any(|line| line == "Usage: qr (-c | -s)"),
        "{help}"
    );
}

#[test]
fn argv0_invocation_runs_the_applet() {
    let output = via_symlink("uuid7", &[]);
    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    let uuid: uuid::Uuid = stdout.trim().parse().unwrap();
    assert_eq!(uuid.get_version_num(), 7, "{stdout}");
}

#[test]
fn unknown_argv0_reports_the_name_it_was_invoked_with() {
    let output = via_symlink("hop", &[]);
    assert_eq!(output.status.code(), Some(1));

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("`hop`"), "{stderr}");
}
