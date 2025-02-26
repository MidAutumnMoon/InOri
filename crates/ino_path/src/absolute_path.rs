use std::path::PathBuf;
use std::path::Path;

use crate::InoPathError;

pub struct AbsolutePath( PathBuf );

impl AbsolutePath {
    pub fn new<T>( path: T ) -> Result<Self, InoPathError>
    where
        T: AsRef<Path>
    {
        let path = path.as_ref();
        if path.is_absolute() {
            Ok( Self( path.to_owned() ) )
        } else {
            Err( InoPathError::NotAbsolute )
        }
    }
}

impl std::ops::Deref for AbsolutePath {
    type Target = PathBuf;
    fn deref( &self ) -> &Self::Target {
        &self.0
    }
}
