use std::path::PathBuf;

use anyhow::ensure;


pub struct Fixture {
    root: PathBuf,
}

impl Fixture {

    pub fn new() -> anyhow::Result<Self> {

        let project_dir = PathBuf::from(
            env!( "CARGO_MANIFEST_DIR" )
        );

        let root = project_dir.join( "fixture" );

        ensure!( root.is_dir() );

        Ok( Self { root } )
    }

    pub fn get( &self, name: &str )
        -> Option<PathBuf>
    {
        let maybe = self.root.join( name );
        maybe.try_exists().unwrap().then( || maybe )
    }

}
