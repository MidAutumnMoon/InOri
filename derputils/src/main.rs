//! Multicall entry point: dispatches to applets by `argv[0]` / first argument.

use std::ffi::OsString;
use std::process::ExitCode;

use derputils::BIN_NAME;
use derputils::Outcome;
use derputils::RunFailure;
use derputils::Selection;
use derputils::select;
use derputils::usage;

fn main() -> ExitCode {
    ino_tracing::init_tracing_subscriber();

    let mut argv_iter = std::env::args_os();
    let invoked_as =
        argv_iter.next().unwrap_or_else(|| OsString::from(BIN_NAME));
    let args: Vec<OsString> = argv_iter.collect();

    match select(&invoked_as, &args) {
        Selection::Run { applet, args } => match (applet.run)(args) {
            Ok(Outcome::Quiet) => ExitCode::SUCCESS,
            Ok(Outcome::Notice(msg)) => {
                eprintln!("{msg}");
                ExitCode::SUCCESS
            }
            Err(RunFailure::Cli(failure)) => {
                // 100 is bpaf's own default max width.
                failure.print_message(100);
                ExitCode::from(
                    u8::try_from(failure.exit_code()).unwrap_or(1),
                )
            }
            Err(RunFailure::Applet(report)) => {
                eprintln!("{report}");
                ExitCode::FAILURE
            }
        },
        Selection::Help => {
            print!("{}", usage());
            ExitCode::SUCCESS
        }
        Selection::Version => {
            let version = env!("CARGO_PKG_VERSION");
            println!("{BIN_NAME} {version}");
            ExitCode::SUCCESS
        }
        Selection::NoApplet => {
            eprint!("{}", usage());
            ExitCode::FAILURE
        }
        Selection::UnknownApplet { name } => {
            eprintln!("{BIN_NAME}: unknown applet '{name}'");
            eprint!("{}", usage());
            ExitCode::FAILURE
        }
    }
}
