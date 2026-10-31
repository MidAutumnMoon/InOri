//! Multicall applet implementations.
//!
//! Shared applet machinery lives in `applet`; the registry and process
//! dispatch live in `dispatch`. Each concrete applet owns its bpaf parser and
//! exposes one `APPLET` descriptor.

mod applet;
mod completion;
pub mod dispatch;
mod hops;
mod qr;
mod upwards;
mod uuid7;
