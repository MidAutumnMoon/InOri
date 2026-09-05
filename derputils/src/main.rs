//! Multicall utility binary: dispatch to applets by `argv[0]` or first argument.

use std::ffi::OsString;
use std::process::ExitCode;

mod applets;

const BIN_NAME: &str = "derputils";

fn main() -> ExitCode {
    let _log_guard = ino_tracing::init_tracing_subscriber();

    let mut argv_iter = std::env::args_os();
    // `argv[0]` always exists in practice; default to the dispatcher name.
    let invoked_as =
        argv_iter.next().unwrap_or_else(|| OsString::from(BIN_NAME));
    let args: Vec<OsString> = argv_iter.collect();

    applets::dispatch(&invoked_as, &args)
}
