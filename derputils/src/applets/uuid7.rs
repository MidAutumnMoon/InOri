//! `uuid7` — print a freshly generated `UUIDv7`.

use std::ffi::OsString;

use bpaf::Args;
use bpaf::OptionParser;
use bpaf::Parser;

use crate::applet::Outcome;
use crate::applet::RunFailure;

/// Applet selector name.
pub const NAME: &str = "uuid7";

/// CLI parser for `uuid7`; takes no arguments.
#[must_use]
pub fn cli() -> OptionParser<()> {
    bpaf::pure(())
        .to_options()
        .descr("Print a freshly generated UUIDv7")
        .version(env!("CARGO_PKG_VERSION"))
}

/// Multicall entry: parse `args` (applet `argv[1..]`) and run.
///
/// # Errors
///
/// Returns a [`RunFailure::Cli`] for parse/help/version exits and a
/// [`RunFailure::Applet`] for runtime failures.
pub fn applet_main(args: &[OsString]) -> Result<Outcome, RunFailure> {
    cli()
        .run_inner(Args::from(args).set_name(NAME))
        .map_err(RunFailure::Cli)?;
    Ok(run())
}

fn run() -> Outcome {
    println!("{}", uuid::Uuid::now_v7());
    Outcome::Quiet
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    use super::*;

    fn parse(args: &[&str]) -> Result<(), bpaf::ParseFailure> {
        cli().run_inner(Args::from(&args[..]).set_name(NAME))
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
