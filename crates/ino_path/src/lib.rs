pub mod is_executable;
pub use is_executable::IsExecutable;

pub mod absolute_path;
pub use absolute_path::AbsolutePath;

#[ derive( thiserror::Error, Debug ) ]
pub enum InoPathError {
    #[ error( "The given path in not absolute" ) ]
    NotAbsolute,
}
