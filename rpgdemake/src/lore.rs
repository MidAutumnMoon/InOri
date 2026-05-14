use std::path::Path;
use std::path::PathBuf;

pub const RPG_HEADER_LEN: usize = 16;

pub const RPG_HEADER: [u8; RPG_HEADER_LEN] = [
    // R P G M V -- SIGNATURE in rpg_core.js
    0x52, 0x50, 0x47, 0x4d, 0x56, // padding
    0x00, 0x00, 0x00, // version -- VER in rpg_core.js
    0x00, 0x03, 0x01, // padding
    0x00, 0x00, 0x00, 0x00, 0x00,
];

/// Length of the encrypted portion of the file.
pub const ENCRYPTED_PART_LEN: usize = 16;

/// Map known extensions of encrypted RPG Maker files
/// to their normal counterparts.
pub fn map_encrypted_extension(input: &str) -> Option<&'static str> {
    match input {
        "rpgmvp" | "png_" => Some("png"),
        "rpgmvo" | "ogg_" => Some("ogg"),
        "rpgmvm" | "m4a_" => Some("m4a"),
        _ => None,
    }
}

/// Replace the encrypted extension with the decrypted one.
pub fn fix_extension(origin: &Path) -> Option<PathBuf> {
    use std::ffi::OsStr;
    let ext = origin.extension().and_then(OsStr::to_str)?;
    let mut path = origin.to_owned();
    let _ = path.set_extension(map_encrypted_extension(ext)?);
    Some(path)
}
