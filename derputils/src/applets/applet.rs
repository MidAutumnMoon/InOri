//! Shared applet descriptor and invocation wrapper.

use std::ffi::OsString;
use std::process::ExitCode;

use bpaf::Args;
use bpaf::OptionParser;

/// One multicall applet.
#[derive(Debug, Clone, Copy)]
pub struct Applet {
    /// Selector matched against `argv[0]` and the dispatcher's first word.
    name: &'static str,
    /// One-line summary shown in the applet listing.
    summary: &'static str,
    /// Parse the applet's own arguments and produce its process exit code.
    entry: fn(&[OsString]) -> ExitCode,
}

impl Applet {
    pub const fn new(
        name: &'static str,
        summary: &'static str,
        entry: fn(&[OsString]) -> ExitCode,
    ) -> Self {
        Self {
            name,
            summary,
            entry,
        }
    }

    pub const fn name(&self) -> &'static str {
        self.name
    }

    pub const fn summary(&self) -> &'static str {
        self.summary
    }

    /// Run this applet's process entry point.
    pub fn run(&self, args: &[OsString]) -> ExitCode {
        (self.entry)(args)
    }
}

/// Parse one applet invocation, run it, and render either kind of failure.
///
/// The applet supplies its own parser and effect; only this wrapper is shared.
pub fn invoke<A, F>(
    parser: &OptionParser<A>,
    name: &str,
    args: &[OsString],
    effect: F,
) -> ExitCode
where
    A: 'static,
    F: FnOnce(A) -> rootcause::Result<ExitCode>,
{
    match parser.run_inner(Args::from(args).set_name(name)) {
        Ok(invocation) => match effect(invocation) {
            Ok(exit_code) => exit_code,
            Err(report) => {
                eprintln!("{report}");
                ExitCode::FAILURE
            }
        },
        Err(failure) => {
            // 100 is bpaf's own default max width.
            failure.print_message(100);
            ExitCode::from(u8::try_from(failure.exit_code()).unwrap_or(1))
        }
    }
}
