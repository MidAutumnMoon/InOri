#![expect(
    clippy::exhaustive_structs,
    clippy::exhaustive_enums,
    reason = "App only, not published"
)]

pub mod automation;
pub mod classify;
pub mod img;

pub mod fs;

pub mod transcoder;

pub mod recipe;
pub mod tomato;

pub mod pipeline;

pub const BACKUP_DIR_NAME: &str = ".backup";
pub const REVIEW_DIR_NAME: &str = ".imgo-review";
