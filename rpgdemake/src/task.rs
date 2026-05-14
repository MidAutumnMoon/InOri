use std::path::{Path, PathBuf};

use anyhow::Context;
use anyhow::ensure;

use crate::key::Key;
use crate::lore::{
    ENCRYPTED_PART_LEN, RPG_HEADER, RPG_HEADER_LEN, fix_extension,
};

/// Decrypt a single RPG Maker encrypted file.
#[tracing::instrument(skip(key))]
pub fn decrypt_file(path: &Path, key: &Key) -> anyhow::Result<PathBuf> {
    validate_header(path).with_context(|| {
        format!("header validation failed for {}", path.display())
    })?;

    let target = fix_extension(path).ok_or_else(|| {
        anyhow::anyhow!("unknown extension for {}", path.display())
    })?;

    let mut content = std::fs::read(path)
        .with_context(|| format!("failed to read {}", path.display()))?;

    // Strip RPG header; the rest is the original file content
    // with its first 16 bytes XOR'd by the key.
    let mut body = content.split_off(RPG_HEADER_LEN);
    key.value.iter().zip(body.iter_mut()).for_each(|(b, cell)| {
        *cell ^= b;
    });

    std::fs::write(&target, body).with_context(|| {
        format!("failed to write {}", target.display())
    })?;

    Ok(target)
}

/// Read file and ensure it has the proper RPG Maker header.
fn validate_header(file: &Path) -> anyhow::Result<()> {
    use std::io::{ErrorKind as IOError, prelude::*};

    let mut file = std::fs::File::open(file)?;
    let mut buf = [0; RPG_HEADER_LEN + ENCRYPTED_PART_LEN];

    file.read_exact(&mut buf).map_err(|e| match e.kind() {
        IOError::UnexpectedEof => {
            anyhow::anyhow!("Insufficient data to decode")
        }
        _ => e.into(),
    })?;

    ensure! { buf[..RPG_HEADER_LEN] == RPG_HEADER,
        "RPG Maker header mismatch"
    };

    Ok(())
}

/// Run decryption over all files in parallel.
#[tracing::instrument(skip_all)]
pub fn run(paths: &[PathBuf], key: &Key) -> anyhow::Result<()> {
    use rayon::prelude::*;

    paths.par_iter().enumerate().for_each(|(idx, path)| {
        let idx = idx + 1;
        let message = match decrypt_file(path, key) {
            Ok(target) => format!("(ok) {}", target.display()),
            Err(e) => {
                format!("(err) {}: {e:#}", path.display())
            }
        };
        println!("{idx}/{}: {message}", paths.len());
    });

    Ok(())
}
