use std::path::{
    Path,
    PathBuf,
};

use assert_fs::prelude::*;
use assert_fs::fixture::ChildPath;


fn fixture( name: &str ) -> PathBuf {
    let dir = PathBuf::from( env!( "CARGO_MANIFEST_DIR" ) )
        .join( "tests" )
        .join( "fixture" )
    ;
    dir.join( name )
}


enum Version {
    MV,
    MZ
}

struct Layout {
    version: Version,
    dir: ChildPath,
}

impl Layout {
    fn new( version: Version, dir: ChildPath ) -> Self {
        Self { version, dir }
    }

    fn base_dir( &self ) -> ChildPath {
        match self.version {
            Version::MV => self.dir.child( "www" ),
            Version::MZ => self.dir.child( "." ),
        }
    }

    fn setup_system_json( &self ) {
        let dir = self.base_dir();
        dir.child( "data/System.json" )
            .write_str( include_str!( "./fixture/System.json" ) )
            .unwrap()
        ;
    }

    fn setup_layout( &self ) {
        self.dir.child( "nw.dll" ).touch().unwrap();

        let dir = self.base_dir();
        let mapping = match self.version {
            Version::MV => [
                ( "img/pictures/Clouds.rpgmvp",
                  fixture( "Clouds.rpgmvp" )
                ),
                ( "audio/bgm/Castle1.rpgmvo",
                  fixture( "Castle1.rpgmvo" )
                )
            ],
            Version::MZ => [
                ( "img/pictures/Clouds.png_",
                  fixture( "Clouds.rpgmvp" )
                ),
                ( "audio/bgm/Castle1.ogg_",
                  fixture( "Castle1.rpgmvo" )
                )
            ],
        };
        mapping.into_iter()
            .map( |( child, src_path )|
                ( child, std::fs::read( src_path ).unwrap() )
            )
            .for_each( |( child, data )|
                dir.child( child ).write_binary( &data ).unwrap()
            ) ;
        dir.child( "junk-to-be-ignored" ).touch().unwrap();
    }

    fn setup_expected( &self ) {
        let dir = self.base_dir();
        let mapping = [
            ( "img/pictures/Clouds.png",
              fixture( "Clouds.png" )
            ),
            ( "audio/bgm/Castle1.ogg",
              fixture( "Castle1.ogg" )
            )
        ];
        mapping.into_iter()
            .map( |( child, src_path )|
                ( child, std::fs::read( src_path ).unwrap() )
            )
            .for_each( |( child, data )|
                dir.child( child ).write_binary( &data ).unwrap()
            ) ;
    }
}


fn run_main_program( dir: &Path ) {
    let exe_path = std::env!( "CARGO_BIN_EXE_rpgdemake" );

    let mut program = std::process::Command::new( exe_path );
    program.env( "RUST_LOG", "debug" );
    program.arg( dir );

    let status = program
        .spawn().unwrap()
        .wait().unwrap()
    ;

    assert!( status.success() );
}


#[ test ]
fn test_mv_layout() {

    let tmpdir = assert_fs::TempDir::new().unwrap();

    let layout = Layout::new(
        Version::MV,
        tmpdir.child(".")
    );

    layout.setup_system_json();
    layout.setup_layout();

    run_main_program( tmpdir.path() );


    let expected_tmpdir = assert_fs::TempDir::new().unwrap();

    let expected_layout = Layout::new(
        Version::MV,
        expected_tmpdir.child( "." )
    );

    expected_layout.setup_system_json();
    expected_layout.setup_layout();
    expected_layout.setup_expected();


    assert! {
        ! dir_diff::is_different(
            tmpdir.path(), expected_tmpdir.path()
        ).unwrap()
    };

}


#[ test ]
fn test_mz_layout() {

    let tmpdir = assert_fs::TempDir::new().unwrap();

    let layout = Layout::new(
        Version::MZ,
        tmpdir.child(".")
    );

    layout.setup_system_json();
    layout.setup_layout();

    run_main_program( tmpdir.path() );


    let expected_tmpdir = assert_fs::TempDir::new().unwrap();

    let expected_layout = Layout::new(
        Version::MZ,
        expected_tmpdir.child( "." )
    );

    expected_layout.setup_system_json();
    expected_layout.setup_layout();
    expected_layout.setup_expected();

    assert! {
        ! dir_diff::is_different(
            tmpdir.path(), expected_tmpdir.path()
        ).unwrap()
    };
}
