use std::ffi::OsString;
use std::process::ExitCode;

use bpaf::OptionParser;
use bpaf::Parser as _;

use super::applet::Applet;
use super::applet::invoke;

const NAME: &str = "uuid7";
const SUMMARY: &str = "Print a freshly generated UUIDv7";
pub(super) const APPLET: Applet = Applet::new(NAME, SUMMARY, entry);

fn cli() -> OptionParser<()> {
    bpaf::pure(()).to_options().descr(SUMMARY)
}

fn entry(args: &[OsString]) -> ExitCode {
    invoke(&cli(), NAME, args, |()| {
        println!("{}", uuid::Uuid::now_v7());
        Ok(ExitCode::SUCCESS)
    })
}
