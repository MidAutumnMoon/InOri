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

#[cfg( test )]
mod test_is_executable {
    use super::*;
    use std::process::Command;
    use tempfile::tempdir;

    #[test]
    fn ok() {
        let tmp_dir = tempdir().unwrap();
        let tmp_file = tmp_dir.path().join( "is_executable" );
        Command::new( "touch" )
            .arg( &tmp_file )
            .output()
            .unwrap();
        Command::new( "chmod" )
            .arg( "+x" )
            .arg( &tmp_file )
            .output()
            .unwrap();
        let result = is_executable( &tmp_file );
        assert!(
            result.is_ok() && result.unwrap()
        )
    }

    #[test]
    fn ok_not_executable() {
        let tmp_dir = tempdir().unwrap();
        let tmp_file = tmp_dir.path().join( "not_executable" );
        Command::new( "touch" )
            .arg( &tmp_file )
            .output()
            .unwrap();
        let result = is_executable( &tmp_file );
        assert!(
            result.is_ok() && !result.unwrap()
       )
    }

    #[test]
    fn error() {
        let tmpdir = tempdir().unwrap();
        let file_unexist = tmpdir.path()
            .join( ulid::Ulid::new().to_string() );
        let result = is_executable( &file_unexist );
        assert!( result.is_err() )
    }
}
