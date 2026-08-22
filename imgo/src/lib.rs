#![expect(
    clippy::exhaustive_structs,
    clippy::exhaustive_enums,
    reason = "App only, not published"
)]

pub mod img;

pub mod fs;

pub mod transcoder;

pub mod tomato;

pub mod pipeline;

pub const BACKUP_DIR_NAME: &str = ".backup";
