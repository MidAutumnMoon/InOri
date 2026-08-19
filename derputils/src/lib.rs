//! Multicall utility binary: dispatch to applets by `argv[0]` or first argument.

pub mod applet;
pub mod applets;
pub use applet::*;

/// Name of the multicall dispatcher binary.
pub const BIN_NAME: &str = "derputils";
