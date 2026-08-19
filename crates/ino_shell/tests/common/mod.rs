//! Shared test harness for the `ino_shell` integration test binaries.
//!
//! Cargo auto-discovers `tests/*.rs` as independent test binaries but does
//! *not* pick up `tests/common/mod.rs` as one, so this module is safe to share
//! via `mod common;` from each sibling test file.

use std::path::Path;

use ino_shell::Shell;

/// Creates a [`Shell`] whose `PATH` points at the directory holding the
/// `xecho` / `xsleep` fixture binaries built by Cargo as `[[bin]]` targets.
///
/// Cargo exposes each bin target's path via `CARGO_BIN_EXE_<name>` at test
/// runtime, so the fixtures are built once, cached by the build graph, and
/// clippy-checked under `--all-targets`.
#[expect(clippy::unwrap_used, reason = "test code")]
pub fn setup() -> Shell {
    let mut sh = Shell::new().unwrap();
    let xecho = std::env::var("CARGO_BIN_EXE_xecho")
        .expect("CARGO_BIN_EXE_xecho must be set by the test harness");
    let bin_dir = Path::new(&xecho)
        .parent()
        .expect("CARGO_BIN_EXE_xecho must have a parent directory");
    sh.set_var("PATH", bin_dir);
    sh
}
