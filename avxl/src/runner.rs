use std::fs::create_dir_all;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Result as AnyResult;
use anyhow::bail;
use ino_path::PathExt;
use tracing::debug;

use crate::App;

pub fn run_app(app: App) -> AnyResult<()> {
    let App {
        transcoder,
        root_dir,
        backup_dir,
        work_dir,
        no_backup,
        show_logs,
        pictures,
        ..
    } = app;

    if !pictures.is_empty() {
        if !backup_dir.try_exists_no_traverse()? {
            debug!("create backup dir");
            create_dir_all(&backup_dir)?;
        }
        if !work_dir.try_exists_no_traverse()? {
            debug!("create work dir");
            create_dir_all(&work_dir)?;
        }
    }

    // TODO: async?
    for (pic, _format) in pictures {
        // If the picture is under root_dir then
        // strip the prefix to make the paths shorter in backup_dir.
        // If not, just give up.
        let backup = pic.strip_prefix(&root_dir).map_or_else(
            |_| backup_dir.join(&pic),
            |suffix| backup_dir.join(suffix),
        );

        let [output_ext, ..] = transcoder.output_format().exts() else {
            bail!("[BUG] Transcoder implements no output format")
        };
        let tempfile = tempfile_in_workdir(&work_dir, output_ext);

        let cmd = transcoder.generate_command(&pic, &tempfile);
    }

    todo!()
}
// TODO: Name clash is not handled, but on real hardware
// it probably won't happen within the lifespan of Rust.
#[inline]
fn tempfile_in_workdir(work_dir: &Path, ext: &str) -> PathBuf {
    use rand::Rng;
    use rand::distr::Alphanumeric;
    let prefix = rand::rng()
        .sample_iter(Alphanumeric)
        .take(8)
        .map(char::from)
        .collect::<String>();
    work_dir.join(format!("{prefix}.{ext}"))
}
