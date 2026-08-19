//! Multicall utility binary: dispatch to applets by `argv[0]` or first argument.

pub mod applet;
pub(crate) mod applets;
pub use applet::*;

/// Basename that selects dispatcher mode (first argument names the applet).
pub const BIN_NAME: &str = "derputils";
