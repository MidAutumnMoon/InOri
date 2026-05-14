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

/// The first 16 bytes of every valid PNG file:
///   8-byte PNG signature + 4-byte IHDR chunk length (always 13) + "IHDR" tag.
///
/// Used by light mode to restore the header without the encryption key.
pub const PNG_HEADER: [u8; ENCRYPTED_PART_LEN] = [
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // PNG signature
    0x00, 0x00, 0x00, 0x0D, // IHDR chunk length (always 13)
    0x49, 0x48, 0x44, 0x52, // "IHDR"
];

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
