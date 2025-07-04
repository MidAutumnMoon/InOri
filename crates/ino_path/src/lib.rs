mod is_executable;
pub use is_executable::IsExecutable;
use tap::Pipe;

use std::io;
use std::path::Path;
use std::path::PathBuf;

#[ derive( thiserror::Error, Debug ) ]
pub enum PathExtError {
    #[ error( r#"Path "{0}" is not absolute"# ) ]
    NotAbsolute( PathBuf )
}

/// Extra functions to work with [`Path`].
#[ allow( clippy::missing_errors_doc ) ]
pub trait PathExt {
    /// Like [`Path::try_exists`], but **does not** traverse
    /// symlinks automatically.
    fn try_exists_no_traverse( &self ) -> io::Result<bool>;

    /// Like [`Path::is_absolute`], but returns error if
    /// this path is not absolute.
    fn must_absolute( &self ) -> Result<&Self, PathExtError>;
}

impl PathExt for Path {
    #[ inline ]
    fn try_exists_no_traverse( &self ) -> io::Result<bool> {
        use std::io::ErrorKind;
        match self.symlink_metadata() {
            Err( err ) => {
                match err.kind() {
                    ErrorKind::NotFound => Ok( false ),
                    _ => Err( err )
                }
            },
            Ok( _ ) => Ok( true )
        }
    }

    #[ inline ]
    fn must_absolute( &self ) -> Result<&Self, PathExtError> {
        if self.is_absolute() {
            Ok( self )
        } else {
            PathExtError::NotAbsolute( self.into() )
                .pipe( Err )
        }
    }
}

#[ cfg( test ) ]
#[ allow( clippy::unwrap_used ) ]
mod test {

    use std::fs::remove_file;
    use std::os::unix::fs::symlink;

    use super::*;

    use assert_fs::prelude::*;
    use assert_fs::TempDir;

    #[ macro_export ]
    macro_rules! make_tempdir {
        () => { {
            TempDir::new().expect( "Failed to setup tempdir" )
        } };
    }

    #[ test ]
    fn try_exists_no_traverse() {
        let top = make_tempdir!();
        let p = top.child( "p" );

        assert!( !p.try_exists_no_traverse().unwrap() );
        p.touch().unwrap();
        assert!( p.try_exists_no_traverse().unwrap() );
        remove_file( &p ).unwrap();
        symlink( "/sys/bbbbbbbroken", &p ).unwrap();
        assert!( p.try_exists_no_traverse().unwrap() );
    }

}
