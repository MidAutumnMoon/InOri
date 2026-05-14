use anyhow::Context;
use anyhow::ensure;
use ino_color::ceprintln;
use ino_color::fg;

use crate::lore::DecryptMethod;
use crate::lore::ENCRYPTED_PART_LEN;
use crate::lore::EncryptedAsset;
use crate::lore::PNG_HEADER;
use crate::lore::RPG_HEADER;
use crate::lore::RPG_HEADER_LEN;

/// Decrypt a single RPG Maker encrypted file.
///
/// - `DecryptMethod::Light`: stamps the known PNG header over the
///   encrypted bytes. Only valid for PNG assets.
/// - `DecryptMethod::Full`: XORs the first 16 bytes after the RPG
///   header with the key. Valid for all asset kinds.
#[tracing::instrument(skip_all)]
pub fn decrypt(
    asset: &EncryptedAsset,
    method: &DecryptMethod,
) -> anyhow::Result<()> {
    if matches!(method, DecryptMethod::Light) && !asset.is_png() {
        anyhow::bail!(
            "light mode only supports PNG, got {:?}",
            asset.kind()
        );
    }

    let target = asset.decrypted_path();

    let mut content = std::fs::read(asset.path()).with_context(|| {
        format!("failed to read {}", asset.path().display())
    })?;

    ensure! {
        content.len() >= RPG_HEADER_LEN + ENCRYPTED_PART_LEN,
        "Insufficient data to decode"
    };
    ensure! {
        content.get(..RPG_HEADER_LEN).is_some_and(|h| h == RPG_HEADER),
        "RPG Maker header mismatch"
    };

    // Strip RPG header; the rest is the original file content
    // with its first 16 bytes XOR'd by the key.
    content.drain(..RPG_HEADER_LEN);

    match method {
        DecryptMethod::Full(key) => {
            for (b, cell) in key.value.iter().zip(content.iter_mut()) {
                *cell ^= b;
            }
        }
        DecryptMethod::Light => {
            // Stamp the known PNG header over the XOR'd bytes
            content
                .get_mut(..ENCRYPTED_PART_LEN)
                .expect("length validated above")
                .copy_from_slice(&PNG_HEADER);
        }
    }

    std::fs::write(&target, content).with_context(|| {
        format!("failed to write {}", target.display())
    })?;

    Ok(())
}

/// Run decryption over all assets in parallel.
#[tracing::instrument(skip_all)]
pub fn run(
    assets: &[EncryptedAsset],
    method: &DecryptMethod,
) -> anyhow::Result<()> {
    use rayon::prelude::*;

    let errors: Vec<_> = assets
        .par_iter()
        .enumerate()
        .filter_map(|(idx, asset)| {
            let idx = idx + 1;
            match decrypt(asset, method) {
                Ok(()) => {
                    ceprintln!(
                        fg::Blue,
                        "{idx}/{}: (ok) {}",
                        assets.len(),
                        asset.decrypted_path().display()
                    );
                    None
                }
                Err(e) => {
                    ceprintln!(
                        fg::Red,
                        "{idx}/{}: (err) {}: {e:#}",
                        assets.len(),
                        asset.path().display()
                    );
                    Some(e)
                }
            }
        })
        .collect();

    if errors.is_empty() {
        Ok(())
    } else {
        anyhow::bail!(
            "{} of {} file(s) failed to decrypt",
            errors.len(),
            assets.len()
        )
    }
}
