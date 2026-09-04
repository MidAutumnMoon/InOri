//! Invoking `nix` from nh.
//!
//! [`command::NixCommand`] runs a `nix <kind>` invocation: the kind
//! contributes the subcommand word, and the run uses the stream handling
//! appropriate to it — interactive commands inherit the standard streams,
//! non-interactive ones drain stdout and stderr concurrently without
//! deadlock. [`options`] parses the nix build flags nh accepts, and
//! [`build`] runs `nix build`, optionally piped through `nom`.
#![expect(
    clippy::module_name_repetitions,
    reason = "the Nix-prefixed names stay unambiguous next to crate::command"
)]

pub mod build;
pub mod command;
pub mod options;
