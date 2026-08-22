//! `uuid7` — print a freshly generated `UUIDv7`.

use std::ffi::OsString;

use bpaf::Args;
use bpaf::OptionParser;
use bpaf::Parser as _;

use crate::applet::RunFailure;

pub const NAME: &str = "uuid7";

#[must_use]
pub fn cli() -> OptionParser<()> {
    bpaf::pure(())
        .to_options()
        .descr("Print a freshly generated UUIDv7")
        .version(env!("CARGO_PKG_VERSION"))
}

pub fn applet_main(args: &[OsString]) -> Result<(), RunFailure> {
    cli()
        .run_inner(Args::from(args).set_name(NAME))
        .map_err(RunFailure::Cli)?;
    println!("{}", uuid::Uuid::now_v7());
    Ok(())
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
