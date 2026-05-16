//! Extra functions for working with paths.

use std::path::Path;

/// Extension trait for checking if the given path
/// is an executable.
///
/// The interface is based on crate `is_executable`.
/// However, the implementation of that crate is primitive
/// and straight up isn't accurate.
///
/// This implementation is based on what used in `find(1)`
/// and `faccess` crate[1].
///
/// [1]: Not using `faccess` because the interface is slightly off
/// to my taste.
pub trait IsExecutable
where
    Self: AsRef<Path>,
{
    /// Check whether the file pointed by given path is an executable.
    ///
    /// Returns `false` if the path doesn't exist, isn't accessible,
    /// or any other error occurs — following the same convention as
    /// [`Path::exists`] and [`Path::is_dir`].
    fn is_executable(&self) -> bool;
}

#[cfg(not(unix))]
compile_error!("`IsExecutable` is only implemented for unix targets");

#[cfg(unix)]
mod unix {

    use super::IsExecutable;
    use std::path::Path;

    impl IsExecutable for Path {
        #[inline]
        fn is_executable(&self) -> bool {
            use rustix::fs::Access;
            use rustix::fs::AtFlags;
            use rustix::fs::CWD;
            use rustix::fs::accessat;

            accessat(CWD, self, Access::EXEC_OK, AtFlags::empty()).is_ok()
        }
    }

    #[cfg(test)]
    use assert2::assert;

    #[test]
    fn unix_test() {
        use std::path::PathBuf;
        use tap::Pipe;

        let binsh = Path::new("/bin/sh");
        assert!(binsh.is_executable());

        // Unix directory is also executable
        let root = Path::new("/");
        assert!(root.is_executable());

        let manifest = env!("CARGO_MANIFEST_PATH").pipe(PathBuf::from);
        assert!(!manifest.is_executable());
    }
}
