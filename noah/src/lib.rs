#![expect(
    clippy::exhaustive_structs,
    clippy::exhaustive_enums,
    reason = "App only, not published"
)]

pub mod args;
pub mod clean;
pub mod command;
pub mod diff;
pub mod nixos;
pub mod progress;
pub mod remote;
pub mod runtime;
pub mod search;
pub mod update;
pub mod util;

/// Converts an error produced by an external crate into a
/// [`rootcause::Report`].
///
/// Dependencies that don't speak rootcause (e.g. `dix`) report their failures
/// through the boxed-error representation; this is the seam where such
/// errors enter rootcause.
pub(crate) fn external_report<E>(err: E) -> rootcause::Report
where
    Box<dyn std::error::Error + Send + Sync>: From<E>,
{
    rootcause::report!(Box::<dyn std::error::Error + Send + Sync>::from(
        err
    ))
    .into()
}
