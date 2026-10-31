use std::ffi::OsString;
use std::process::ExitCode;

use bpaf::Args;
use bpaf::OptionParser;

/// bpaf's default wrap width for parse-failure messages.
const MESSAGE_WIDTH: usize = 100;

#[derive(Debug, Clone, Copy)]
pub struct Applet {
    name: &'static str,
    summary: &'static str,
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

    pub fn run(&self, args: &[OsString]) -> ExitCode {
        (self.entry)(args)
    }
}

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
            failure.print_message(MESSAGE_WIDTH);
            u8::try_from(failure.exit_code())
                .map_or(ExitCode::FAILURE, ExitCode::from)
        }
    }
}
