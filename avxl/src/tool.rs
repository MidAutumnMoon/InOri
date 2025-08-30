use std::path::Path;

use anyhow::Context;
use anyhow::Result as AnyResult;
use anyhow::bail;
use ino_path::PathExt;
use tracing::debug;
use tracing::debug_span;
use tracing::trace;
use walkdir::WalkDir;

use crate::BACKUP_DIR_NAME;
use crate::Task;

// #[tracing::instrument(skip_all)]
// pub fn list_pictures_recursively(
//     topleve: &Path,
//     input_extensions: StaticStrs,
//     output_extension: &'static str,
// ) -> AnyResult<Vec<Task>> {
//     debug!("list all files");

//     let mut input_files = vec![];
//     for entry in WalkDir::new(topleve).follow_links(false) {
//         let entry = entry.context("Failed to read entry")?;
//         let path = entry.path();
//         let _span = debug_span!("inspect_path", ?path).entered();

//         if path.is_dir_no_traverse()? {
//             trace!("dir, ignore");
//             continue;
//         }

//         if let Some(ext) = path.extension()
//             && let Some(ext) = ext.to_str()
//             && input_extensions.contains(&ext)
//         {
//             trace!(?path, "found picture");
//             input_files.push(path.to_owned());
//         } else {
//             trace!(?path, "ignore path, ext not supported");
//         }
//     }

//     let mut pictures = vec![];
//     for input in input_files {
//         let _span = debug_span!("path_to_picture", ?input).entered();
//         let output = {
//             let mut p = input.clone();
//             if !p.set_extension(output_extension) {
//                 bail!("[BUG] Failed to set extension for {}", input.display());
//             }
//             p
//         };
//         let backup = {
//             let Ok(base) = input.strip_prefix(topleve) else {
//                 bail!("[BUG] Failed to remove toplevel prefix");
//             };
//             topleve.join(BACKUP_DIR_NAME).join(base)
//         };
//         pictures.push(Task {
//             input,
//             output,
//             backup,
//         });
//     }

//     Ok(pictures)
// }
