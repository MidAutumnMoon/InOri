//! `uuid7` — print a freshly generated `UUIDv7`.

use std::ffi::OsString;

use bpaf::Args;
use bpaf::OptionParser;
use bpaf::Parser;

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

    fn parse(args: &[&str]) -> Result<(), bpaf::ParseFailure> {
        cli().run_inner(Args::from(args).set_name(NAME))
    }

    #[test]
    fn no_arguments_accepted() {
        assert!(parse(&[]).is_ok());
    }

    #[test]
    fn stray_argument_rejected() {
        assert!(matches!(
            parse(&["x"]),
            Err(bpaf::ParseFailure::Stderr(_))
        ));
    }
}
