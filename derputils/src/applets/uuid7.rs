//! `uuid7` — print a freshly generated `UUIDv7`.

use std::ffi::OsString;
use std::process::ExitCode;

use bpaf::OptionParser;
use bpaf::Parser as _;

use super::applet::Applet;
use super::applet::invoke;

const NAME: &str = "uuid7";
const SUMMARY: &str = "Print a freshly generated UUIDv7";
pub(super) const APPLET: Applet = Applet::new(NAME, SUMMARY, entry);

/// The applet's parser: no arguments.
fn cli() -> OptionParser<()> {
    bpaf::pure(()).to_options().descr(SUMMARY)
}

/// Parse a dispatcher or `argv[0]` invocation and run the applet.
fn entry(raw: &[OsString]) -> ExitCode {
    invoke(&cli(), NAME, raw, |()| {
        println!("{}", uuid::Uuid::now_v7());
        Ok(ExitCode::SUCCESS)
    })
}
