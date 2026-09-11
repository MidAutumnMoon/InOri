//! `uuid7` — print a freshly generated `UUIDv7`.

use std::process::ExitCode;

use bpaf::OptionParser;

use super::Applet;
use super::Invocation;

const NAME: &str = "uuid7";
const SUMMARY: &str = "Print a freshly generated UUIDv7";
pub(super) const APPLET: Applet = Applet::new(NAME, cli);

/// The applet's CLI: a fresh `UUIDv7`, no arguments.
fn cli() -> OptionParser<Invocation> {
    Invocation::cli(bpaf::pure(()), SUMMARY, |()| {
        println!("{}", uuid::Uuid::now_v7());
        Ok(ExitCode::SUCCESS)
    })
}

#[cfg(test)]
mod test {
    use super::*;
    use bpaf::Args;

    fn parse(items: &[&str]) -> Result<Invocation, bpaf::ParseFailure> {
        cli().run_inner(Args::from(items).set_name(NAME))
    }

    #[test]
    fn stray_argument_rejected() {
        assert!(matches!(
            parse(&["x"]),
            Err(bpaf::ParseFailure::Stderr(_))
        ));
    }
}
