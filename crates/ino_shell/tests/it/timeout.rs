use crate::setup;
use std::time::Duration;

use ino_shell::cmd;

/// Deadline for the *failure* cases: the command under test sleeps 5s, so the
/// timeout must fire well inside that window or the suite waits the full sleep.
const FAILURE_TIMEOUT: Duration = Duration::from_millis(500);
/// Upper bound for the *success* cases, which finish after ~1s. Generous so the
/// assertions are robust to slow CI runners.
const SUCCESS_TIMEOUT: Duration = Duration::from_secs(3);

#[test]
fn test_run_timeout_success() {
    let sh = setup();
    let command = cmd!(sh, "xsleep 1"); // Command that xsleeps for 1 second

    // Run the command with a timeout
    let result = command.timeout(SUCCESS_TIMEOUT).run();
    assert!(
        result.is_ok(),
        "Command should complete successfully within the timeout"
    );
}

#[test]
fn test_run_timeout_failure() {
    let sh = setup();
    let command = cmd!(sh, "xsleep 5"); // Command that xsleeps for 5 seconds

    // Run the command with a timeout
    let result = command.timeout(FAILURE_TIMEOUT).run();
    assert!(result.is_err(), "Command should fail due to timeout");
}

#[test]
fn test_read_timeout_success() {
    let sh = setup();
    let command = cmd!(sh, "xecho Hello, world!"); // Command that prints a message

    // Run the command with a timeout and read stdout
    let result = command.timeout(SUCCESS_TIMEOUT).read();
    assert!(
        result.is_ok(),
        "Command should complete successfully within the timeout"
    );
    assert_eq!(result.unwrap(), "Hello, world!");
}

#[test]
fn test_read_timeout_failure() {
    let sh = setup();
    let command = cmd!(sh, "xsleep 5"); // Command that xsleeps for 5 seconds

    // Run the command with a timeout and read stdout
    let result = command.timeout(FAILURE_TIMEOUT).read();
    assert!(result.is_err(), "Command should fail due to timeout");
}

#[test]
fn test_read_stderr_timeout_success() {
    let sh = setup();
    let command = cmd!(sh, "xecho -e Error message"); // Command that prints an error message to stderr

    // Run the command with a timeout and read stderr
    let result = command.timeout(SUCCESS_TIMEOUT).read_stderr();
    assert!(
        result.is_ok(),
        "Command should complete successfully within the timeout"
    );
    assert_eq!(result.unwrap(), "Error message");
}

#[test]
fn test_read_stderr_timeout_failure() {
    let sh = setup();
    let command = cmd!(sh, "xsleep 5"); // Command that xsleeps for 5 seconds

    // Run the command with a timeout and read stderr
    let result = command.timeout(FAILURE_TIMEOUT).read_stderr();
    assert!(result.is_err(), "Command should fail due to timeout");
}

#[test]
fn test_output_timeout_success() {
    let sh = setup();
    let command = cmd!(sh, "xecho Hello, world!"); // Command that prints a message

    // Run the command with a timeout and get the full output
    let result = command.timeout(SUCCESS_TIMEOUT).output();
    assert!(
        result.is_ok(),
        "Command should complete successfully within the timeout"
    );
    let output = result.unwrap();
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "Hello, world!"
    );
}

#[test]
fn test_output_timeout_failure() {
    let sh = setup();
    let command = cmd!(sh, "xsleep 5"); // Command that xsleeps for 5 seconds

    // Run the command with a timeout and get the full output
    let result = command.timeout(FAILURE_TIMEOUT).output();
    assert!(result.is_err(), "Command should fail due to timeout");
}
