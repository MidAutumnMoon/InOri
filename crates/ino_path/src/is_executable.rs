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
    fn is_executable( &self ) -> std::io::Result<bool>;
}

#[ cfg( unix ) ]
mod unix {

    use tap::Pipe;

    use std::path::Path;
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    use super::IsExecutable;

    use libc::faccessat;
    use libc::AT_FDCWD;
    use libc::X_OK;

    impl IsExecutable for Path {
        #[ inline( always ) ]
        fn is_executable( &self ) -> std::io::Result<bool> {
            let path = self.as_os_str()
                .as_bytes()
                .pipe( CString::new )?
            ;
            let ret = unsafe {
                // Check if `path` is executable (X_OK)
                // using the real user id (`0` means no addtional flags,
                // which invokes the default behivior)
                // (Note: Use AT_EACCESS to use the effcitive user id instead)
                faccessat( AT_FDCWD, path.as_ptr(), X_OK, 0 )
            };
            if ret == 0 {
                Ok( true )
            } else {
                use std::io::ErrorKind;
                use std::io::Error;
                let err = Error::last_os_error();
                match err.kind() {
                    ErrorKind::PermissionDenied => Ok( false ),
                    _ => Err( err )
                }
            }
        }
    }

    #[ test ]
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
            .pipe( PathBuf::from )
        ;
        assert! {
            !manifest.is_executable()
                .inspect_err( |err| println!( "{err:?}" ) )
                .unwrap()
        };
    }

}
