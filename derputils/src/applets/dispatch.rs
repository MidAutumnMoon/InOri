//! Applet registry, name-first dispatch, and the grouped help listing.

use std::ffi::OsStr;
use std::ffi::OsString;
use std::path::Path;
use std::process::ExitCode;

use super::applet::Applet;
use super::completion;
use super::hops;
use super::qr;
use super::upwards;
use super::uuid7;
use crate::BIN_NAME;

/// The complete applet registry, grouped as the listing shows them.
const APPLET_GROUPS: &[(&str, &[Applet])] = &[
    ("Generators", &[qr::APPLET, uuid7::APPLET]),
    ("Paths", &[upwards::APPLET, hops::APPLET]),
    ("Shell integration", &[completion::APPLET]),
];

/// Every applet, in registry order.
pub fn applets() -> impl Iterator<Item = &'static Applet> {
    APPLET_GROUPS.iter().flat_map(|&(_, applets)| applets)
}

pub fn find_applet(name: &str) -> Option<&'static Applet> {
    applets().find(|applet| applet.name() == name)
}

/// The applet listing, grouped in registry order.
fn listing() -> String {
    use std::fmt::Write as _;

    let width = applets()
        .map(|applet| applet.name().len())
        .max()
        .unwrap_or(0);

    let mut out = format!("Usage: {BIN_NAME} APPLET [OPTIONS]...\n");

    for &(heading, applets) in APPLET_GROUPS {
        if applets.is_empty() {
            continue;
        }
        _ = writeln!(out, "\n{heading}");
        for applet in applets {
            _ = writeln!(
                out,
                "    {:<width$}  {}",
                applet.name(),
                applet.summary()
            );
        }
    }

    out
}

fn print_listing() {
    print!("{}", listing());
}

/// Report a name that is not in the registry.
fn unknown_applet(name: &OsStr) -> ExitCode {
    eprintln!("{BIN_NAME}: unknown applet `{}`", name.to_string_lossy());
    eprintln!("Run `{BIN_NAME} --help` for the applet list.");
    ExitCode::FAILURE
}

/// Resolve and execute one multicall process invocation.
#[must_use]
pub fn dispatch(invoked_as: &OsStr, args: &[OsString]) -> ExitCode {
    // Multicall name: `argv[0]` selects the applet.
    let invoked = Path::new(invoked_as)
        .file_name()
        .unwrap_or_else(|| OsStr::new(BIN_NAME));
    if invoked != OsStr::new(BIN_NAME) {
        let Some(applet) = invoked.to_str().and_then(find_applet) else {
            return unknown_applet(invoked);
        };
        return applet.run(args);
    }

    // Dispatcher name: the first argument selects the applet.
    let Some((first, rest)) = args.split_first() else {
        print_listing();
        return ExitCode::SUCCESS;
    };

    match first.to_str() {
        Some("-h" | "--help" | "help") => {
            print_listing();
            ExitCode::SUCCESS
        }
        Some("-V" | "--version" | "version") => {
            println!("{BIN_NAME} {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some(name) => {
            let Some(applet) = find_applet(name) else {
                return unknown_applet(OsStr::new(name));
            };
            applet.run(rest)
        }
        None => unknown_applet(first.as_os_str()),
    }
}

#[cfg(test)]
mod test {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn listing_contains_every_group_and_applet() {
        let text = listing();
        for &(heading, applets) in APPLET_GROUPS {
            assert!(
                text.contains(heading),
                "missing {heading:?} in:\n{text}"
            );
            for applet in applets {
                assert!(
                    text.contains(applet.name()),
                    "missing applet {:?} in:\n{text}",
                    applet.name()
                );
                assert!(
                    text.contains(applet.summary()),
                    "missing summary for {:?} in:\n{text}",
                    applet.name()
                );
            }
        }
    }

    #[test]
    fn applet_names_are_unique_across_groups() {
        let mut names = HashSet::new();
        for applet in applets() {
            assert!(
                names.insert(applet.name()),
                "duplicate applet {:?}",
                applet.name()
            );
        }
    }
}
