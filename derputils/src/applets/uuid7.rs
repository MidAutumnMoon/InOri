//! `uuid7` — print a freshly generated `UUIDv7`.

use std::ffi::OsString;
use std::process::ExitCode;

use bpaf::Args;
use bpaf::OptionParser;
use bpaf::Parser as _;

use super::Applet;
use super::RunFailure;

const NAME: &str = "uuid7";
const SUMMARY: &str = "Print a freshly generated UUIDv7";
pub(super) const APPLET: Applet = Applet::new(NAME, SUMMARY, applet_main);

#[must_use]
fn cli() -> OptionParser<()> {
    bpaf::pure(()).to_options().descr(SUMMARY)
}

fn applet_main(args: &[OsString]) -> Result<ExitCode, RunFailure> {
    cli()
        .run_inner(Args::from(args).set_name(NAME))
        .map_err(RunFailure::Cli)?;
    println!("{}", uuid::Uuid::now_v7());
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod test {
    use super::*;
    use std::assert_matches;

    fn parse(args: &[&str]) -> Result<(), bpaf::ParseFailure> {
        cli().run_inner(Args::from(args).set_name(NAME))
    }

    #[test]
    fn stray_argument_rejected() {
        assert_matches!(parse(&["x"]), Err(bpaf::ParseFailure::Stderr(_)));
    }
}
