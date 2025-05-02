//! Extra functions for working with paths.

use std::path::Path;

/// Extension trait for checking if the given path
/// is an executable.
///
/// The interface is based on crate `is_executable`.
/// However the implementation of that crate is primitive
/// and straight up isn't accurate.
///
/// This implementation is based on what used in `find(1)`
/// and `faccess` crate[1].
///
/// [1]: Not using `faccess` because the interface is slightly off
/// to my taste.
pub trait IsExecutable
where
    Self: AsRef<Path>
{
    /// Check whether the file pointed by given path is an executable.
    ///
    /// # Errors
    ///
    /// See [`std::io::Error`]
    fn is_executable( &self ) -> std::io::Result<bool>;
}

#[ cfg( unix ) ]
mod unix {

    use super::IsExecutable;
    use std::path::Path;

    impl IsExecutable for Path {
        #[ inline ]
        fn is_executable( &self ) -> std::io::Result<bool> {
            let ret = {
                use rustix::fs::accessat;
                use rustix::fs::CWD;
                use rustix::fs::Access;
                use rustix::fs::AtFlags;
                accessat( CWD, self, Access::EXEC_OK, AtFlags::empty() )
            };
            match ret {
                Err( err ) => {
                    use std::io::ErrorKind;
                    if matches!( err.kind(), ErrorKind::PermissionDenied ) {
                        Ok( false )
                    } else {
                        Err( err.into() )
                    }
                },
                Ok(()) => Ok( true ),
            }
        }
    }

    #[ test ]
    #[ allow( clippy::unwrap_used ) ]
    fn unix_test() {
        use tap::Pipe;
        use std::path::PathBuf;

        let binsh = Path::new( "/bin/sh" );
        assert! {
            binsh.is_executable()
                .inspect_err( |err| println!( "{err:?}" ) )
                .unwrap()
        };

        // Unix directory is also executable
        let root = Path::new( "/" );
        assert! {
            root.is_executable()
                .inspect_err( |err| println!( "{err:?}" ) )
                .unwrap()
        };

        let manifest = env!( "CARGO_MANIFEST_PATH" )
            .pipe( PathBuf::from );
        assert! {
            !manifest.is_executable()
                .inspect_err( |err| println!( "{err:?}" ) )
                .unwrap()
        };

    }

}
