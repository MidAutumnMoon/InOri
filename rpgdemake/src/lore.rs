pub const RPG_HEADER_LEN: usize = 16;

pub const RPG_HEADER: [ u8; RPG_HEADER_LEN ] = [
    // R P G M V -- SIGNATURE in rpg_core.js
    0x52, 0x50, 0x47, 0x4d, 0x56,
    // padding
    0x00, 0x00, 0x00,
    // version -- VER in rpg_core.js
    0x00, 0x03, 0x01,
    // padding
    0x00, 0x00, 0x00, 0x00, 0x00
];

/// Length of the encrypted portion of the file.
pub const ENCRYPTED_PART_LEN: usize = 16;

