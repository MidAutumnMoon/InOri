use std::path::Path;
use std::path::PathBuf;

use walkdir::WalkDir;

use rayon::prelude::*;

use crate::lore::DecryptMode;
use crate::lore::map_encrypted_extension;

/// Find encrypted files under `toplevel` according to `mode`.
///
/// - `DecryptMode::Light`: only encrypted PNG files (`.rpgmvp` / `.png_`).
/// - `DecryptMode::Full`: all encrypted RPG Maker asset types.
#[tracing::instrument]
pub fn find(
    toplevel: &Path,
    mode: DecryptMode,
) -> anyhow::Result<Vec<PathBuf>> {
    use itertools::Itertools;

    let files =
        WalkDir::new(toplevel).into_iter().process_results(|iter| {
            iter.par_bridge()
                .map(|entry| entry.path().to_owned())
                .filter(|path| path.is_file())
                .filter_map(|path| {
                    let ext = path.extension()?.to_str()?;
                    match mode {
                        DecryptMode::Light => match ext {
                            "rpgmvp" | "png_" => Some(path),
                            _ => None,
                        },
                        DecryptMode::Full => {
                            map_encrypted_extension(ext)?;
                            Some(path)
                        }
                    }
                })
                .collect()
        })?;

    Ok(files)
}
