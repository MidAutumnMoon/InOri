use std::path::Path;
use std::path::PathBuf;

use crate::key::Key;

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

/// Kind of encrypted RPG Maker asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetKind {
    Png,
    Ogg,
    M4a,
}

impl AssetKind {
    /// Parse an encrypted file extension into an `AssetKind`.
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

/// An encrypted RPG Maker asset: a file path paired with its kind.
#[derive(Debug)]
pub struct EncryptedAsset {
    path: PathBuf,
    kind: AssetKind,
}

impl EncryptedAsset {
    /// Create from a path by inspecting its extension.
    ///
    /// Returns `None` if the extension isn't a recognized
    /// RPG Maker encrypted extension.
    pub fn new(path: PathBuf) -> Option<Self> {
        use std::ffi::OsStr;

        let ext = path.extension().and_then(OsStr::to_str)?;
        let kind = AssetKind::from_ext(ext)?;
        Some(Self { path, kind })
    }

    /// The encrypted file path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The asset kind (PNG, OGG, M4A).
    pub fn kind(&self) -> AssetKind {
        self.kind
    }

    /// Compute the decrypted output path
    /// (same path with the extension replaced).
    pub fn decrypted_path(&self) -> PathBuf {
        let mut out = self.path.clone();
        let _ = out.set_extension(self.kind.decrypted_extension());
        out
    }

    /// Whether this is a PNG asset.
    pub fn is_png(&self) -> bool {
        self.kind.is_png()
    }
}

/// A fully-resolved decryption action.
///
/// Unlike the CLI `Mode` (which is just a user selection),
/// an `Action` carries all data needed to decrypt.
#[derive(Debug)]
pub enum DecryptAction {
    /// Stamp the known PNG header — only valid for PNG assets.
    Light,
    /// XOR with the encryption key — valid for all asset kinds.
    Full(Key),
}
