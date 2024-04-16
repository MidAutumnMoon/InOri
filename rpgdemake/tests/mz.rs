//! RPG Maker MZ
//!
//! ## Structure
//!
//! - Game.exe
//! - www/
//!     - data/System.json
//!     - audio/
//!     - img/
//!


mod fixture;


#[ test ]
fn test_mz_layout() {

    use assert_fs::prelude::*;

    let mut demake = assert_cmd::Command
        ::cargo_bin( env!("CARGO_PKG_NAME") )
        .unwrap()
    ;

    demake.env( "RUST_LOG", "debug" );

    // Setup with encrypted files

    let fxt = fixture::Fixture::new().unwrap();

    let tempdir = assert_fs::TempDir::new()
        .unwrap()
    ;

    tempdir.child( "data/System.json" )
        .write_file( &fxt.get( "System.json" ).unwrap() )
        .unwrap()
    ;

    tempdir.child( "img/battlebacks1/Clouds.png_" )
        .write_file( &fxt.get( "Clouds.rpgmvp" ).unwrap() )
        .unwrap()
    ;

    tempdir.child( "audio/bgm/Castle1.ogg_" )
        .write_file( &fxt.get( "Castle1.rpgmvo" ).unwrap() )
        .unwrap()
    ;

    tempdir.child( "img/junk-to-be-ignored" )
        .touch()
        .unwrap()
    ;


    // Setup with decrypted files as the expected

    let expected = assert_fs::TempDir::new()
        .unwrap()
    ;

    expected.copy_from( tempdir.path(), &["**"] ).unwrap();

    expected.child( "img/battlebacks1/Clouds.png" )
        .write_file( &fxt.get( "Clouds.png" ).unwrap() )
        .unwrap()
    ;

    expected.child( "audio/bgm/Castle1.ogg" )
        .write_file( &fxt.get( "Castle1.ogg" ).unwrap() )
        .unwrap()
    ;


    // Run our tool

    use std::io::Write;

    let output = demake.arg( tempdir.path() ).unwrap();

    std::io::stdout().write_all(&output.stdout).unwrap();
    std::io::stderr().write_all(&output.stderr).unwrap();


    // If things went alright, there shouldn't be different

    use dir_diff::is_different;

    assert! {
        ! is_different( tempdir.path(), expected.path())
            .unwrap()
    };

}
