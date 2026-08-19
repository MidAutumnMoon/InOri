#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code"
)]

mod common;

use std::time::Duration;

use ino_shell::cmd;

use common::setup;

/// Deadline for the *failure* cases: the command under test sleeps 5s, so the
/// timeout must fire well inside that window or the suite waits the full sleep.
const FAILURE_TIMEOUT: Duration = Duration::from_millis(500);
/// Upper bound for the *success* cases, which finish after ~1s. Generous so the
/// assertions are robust to slow CI runners.
const SUCCESS_TIMEOUT: Duration = Duration::from_secs(3);

#[test]
fn timeout_success() {
    let sh = setup();

    let output = cmd!(sh, "xecho Hello, world!")
        .timeout(SUCCESS_TIMEOUT)
        .read()
        .unwrap();
    assert_eq!(output, "Hello, world!");
}

#[test]
fn timeout_failure_run() {
    let sh = setup();

    let result = cmd!(sh, "xsleep 5").timeout(FAILURE_TIMEOUT).run();
    assert!(result.is_err(), "command should fail due to timeout");
}

#[test]
fn timeout_failure_read() {
    // Exercises the capture-thread + kill interplay that `run` skips:
    // the deadline fires while stdout/stderr capture threads are live, so
    // the child must be killed and the threads reaped without deadlock.
    let sh = setup();

    let result = cmd!(sh, "xsleep 5").timeout(FAILURE_TIMEOUT).read();
    assert!(result.is_err(), "command should fail due to timeout");
}
