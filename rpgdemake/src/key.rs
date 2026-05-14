use anyhow::ensure;

use tracing::debug;

/// Length of encryption key.
/// Since the encryption method is a naive XOR,
/// the key length should equal to the length of the encrypted part.
pub const KEY_LEN: usize = crate::lore::ENCRYPTED_PART_LEN;

/// The per-project key used to encrypt assets.
#[derive(Debug, Clone)]
pub struct Key {
    pub value: [u8; KEY_LEN],
}

impl Key {
    /// Decode a hex-encoded encryption key string.
    fn from_hex(raw_key: &str) -> anyhow::Result<Self> {
        ensure! { raw_key.len() == 2 * KEY_LEN,
            "\"{raw_key}\" is not a valid encryption key. \
            Maybe it's fake, obfuscated or broken.",
        };

        let value = hex::decode(raw_key)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("key length mismatch"))?;

        Ok(Self { value })
    }

    #[tracing::instrument(skip_all)]
    pub fn parse_json(json: &str) -> anyhow::Result<Option<Self>> {
        use serde_json::Value;

        debug!("try find encryption key in JSON");

        let fields: Value = serde_json::from_str(json)?;

        let Some(Value::String(key)) = fields.get("encryptionKey") else {
            return Ok(None);
        };

        debug!(key, "found key");

        Ok(Some(Self::from_hex(key)?))
    }
}

#[cfg(test)]
#[expect(clippy::unwrap_used)]
mod tests {

    use super::*;
    const JSON: &str = include_str!("../tests/fixture/System.json");
    const EMPTY_JSON: &str = "{}";
    const KEY_STR: &str = "bb145893824d809dcab45febae756d2b";
    const KEY_STR_INVALID: &str = "wow";
    const EXPECTED_KEY: &[u8] = &[
        187, 20, 88, 147, 130, 77, 128, 157, 202, 180, 95, 235, 174, 117,
        109, 43,
    ];

    #[test]
    fn from_hex() {
        let key = Key::from_hex(KEY_STR).unwrap();
        assert_eq!(key.value, EXPECTED_KEY);
    }

    #[test]
    fn from_hex_invalid() {
        assert!(Key::from_hex(KEY_STR_INVALID).is_err());
    }

    #[test]
    fn parse_json() {
        let key = Key::parse_json(JSON).unwrap().unwrap();
        assert_eq!(key.value, EXPECTED_KEY);
    }

    #[test]
    fn parse_json_no_key() {
        assert!(Key::parse_json(EMPTY_JSON).unwrap().is_none());
    }
}
