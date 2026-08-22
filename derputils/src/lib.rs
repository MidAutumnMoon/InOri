//! Multicall utility binary: dispatch to applets by `argv[0]` or first argument.

#![expect(
    clippy::exhaustive_structs,
    clippy::exhaustive_enums,
    reason = "App only, not published"
)]

pub mod applet;
pub(crate) mod applets;

/// Basename that selects dispatcher mode (first argument names the applet).
pub const BIN_NAME: &str = "derputils";
