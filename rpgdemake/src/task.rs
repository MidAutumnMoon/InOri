use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::ensure;
use ino_color::ceprintln;
use ino_color::fg;

use crate::key::Key;
use crate::lore::ENCRYPTED_PART_LEN;
use crate::lore::PNG_HEADER;
use crate::lore::RPG_HEADER;
use crate::lore::RPG_HEADER_LEN;
use crate::lore::fix_extension;

/// Decrypt a single RPG Maker encrypted file.
///
/// If `key` is `Some`, XORs the first 16 bytes after the RPG header
/// with the key (full mode). If `None`, stamps the known PNG header
/// over those bytes instead (light mode).
#[tracing::instrument(skip(key))]
pub fn decrypt(path: &Path, key: Option<&Key>) -> anyhow::Result<PathBuf> {
    let target = fix_extension(path).ok_or_else(|| {
        anyhow::anyhow!("unknown extension for {}", path.display())
    })?;

    let mut content = std::fs::read(path)
        .with_context(|| format!("failed to read {}", path.display()))?;

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

    match key {
        Some(k) => {
            for (b, cell) in k.value.iter().zip(content.iter_mut()) {
                *cell ^= b;
            }
        }
        None => {
            // Light mode: stamp the known PNG header over the
            // XOR'd bytes
            content
                .get_mut(..ENCRYPTED_PART_LEN)
                .expect("length validated above")
                .copy_from_slice(&PNG_HEADER);
        }
    }

    std::fs::write(&target, content).with_context(|| {
        format!("failed to write {}", target.display())
    })?;

    Ok(target)
}

/// Run decryption over all files in parallel.
#[tracing::instrument(skip_all)]
pub fn run(paths: &[PathBuf], key: Option<&Key>) -> anyhow::Result<()> {
    use rayon::prelude::*;

    let errors: Vec<_> = paths
        .par_iter()
        .enumerate()
        .filter_map(|(idx, path)| {
            let idx = idx + 1;
            match decrypt(path, key) {
                Ok(target) => {
                    ceprintln!(
                        fg::Blue,
                        "{idx}/{}: (ok) {}",
                        paths.len(),
                        target.display()
                    );
                    None
                }
                Err(e) => {
                    ceprintln!(
                        fg::Red,
                        "{idx}/{}: (err) {}: {e:#}",
                        paths.len(),
                        path.display()
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
            paths.len()
        )
    }
}
