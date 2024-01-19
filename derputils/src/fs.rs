use std::path::Path;
use std::os::unix::fs::PermissionsExt;


/// Check if some file is executable.
pub fn is_executable( path: &Path ) -> std::io::Result<bool> {
    let metadata = path.metadata()?;
    let permission = metadata.permissions();
    Ok(
        metadata.is_file()
        && ( permission.mode() & 0o111 != 0 )
    )
}
