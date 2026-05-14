use std::path::Path;
use std::path::PathBuf;

pub const RPG_HEADER_LEN: usize = 16;

/// Length of the encrypted portion of the file.
pub const ENCRYPTED_PART_LEN: usize = 16;

#[rustfmt::skip]
pub const RPG_HEADER: [u8; RPG_HEADER_LEN] = [
    0x52, 0x50, 0x47, 0x4d, 0x56, // "RPGMV", SIGNATURE in rpg_core.js
    0x00, 0x00, 0x00, // padding
    0x00, 0x03, 0x01, // version -- VER in rpg_core.js
    0x00, 0x00, 0x00, 0x00, 0x00, // padding
];

/// The first 16 bytes of every valid PNG file:
///   8-byte PNG signature + 4-byte IHDR chunk length (always 13) + "IHDR" tag.
///
/// Used by light mode to restore the header without the encryption key.
#[rustfmt::skip]
pub const PNG_HEADER: [u8; ENCRYPTED_PART_LEN] = [
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // PNG signature
    0x00, 0x00, 0x00, 0x0D, // IHDR chunk length (always 13)
    0x49, 0x48, 0x44, 0x52, // "IHDR"
];

/// Decrypt mode.
#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecryptMode {
    /// Decrypt all assets using the encryption key from System.json.
    Full,
    /// Decrypt PNG images only, without needing the encryption key.
    Light,
}

/// Kind of encrypted RPG Maker asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncryptedKind {
    Png,
    Ogg,
    M4a,
}

impl EncryptedKind {
    /// Parse an encrypted file extension into an `EncryptedKind`.
    ///
    /// MV extensions: `.rpgmvp`, `.rpgmvo`, `.rpgmvm`
    /// MZ extensions: `.png_`, `.ogg_`, `.m4a_`
    pub fn from_ext(ext: &str) -> Option<Self> {
        match ext {
            "rpgmvp" | "png_" => Some(Self::Png),
            "rpgmvo" | "ogg_" => Some(Self::Ogg),
            "rpgmvm" | "m4a_" => Some(Self::M4a),
            _ => None,
        }
    }

    /// The decrypted file extension for this kind.
    pub fn decrypted_extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Ogg => "ogg",
            Self::M4a => "m4a",
        }
    }

    /// Whether this kind is PNG.
    pub fn is_png(self) -> bool {
        self == Self::Png
    }
}

/// Replace the encrypted extension with the decrypted one.
pub fn fix_extension(origin: &Path) -> Option<PathBuf> {
    use std::ffi::OsStr;
    let ext = origin.extension().and_then(OsStr::to_str)?;
    let mut path = origin.to_owned();
    let _ = path.set_extension(
        EncryptedKind::from_ext(ext)?.decrypted_extension(),
    );
    Some(path)
}
