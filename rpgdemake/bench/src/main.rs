use std::process::Command;

use assert_fs::{
    prelude::*,
    TempDir,
    fixture::ChildPath,
};


const SYSTEM_JSON: &str =
    include_str!( "../../tests/fixture/System.json" );

// TODO: reduce the file size to speed up benchmark feedback loop.
const BINARY_ASSET: &[u8] =
    include_bytes!( "../../tests/fixture/Clouds.rpgmvp" );


#[ monoio::main ]
async fn main() {

    /*
     * Setup used CLI programs
     */

    rlimit::increase_nofile_limit( u64::MAX )
        .expect( "Failed to increase nofile limit" );

    let mut hyperfine = Command::new( "hyperfine" );

    hyperfine
        .arg( "--warmup=3" )
        .arg( "--" )
    ;

    let fixture_dir = setup_fs_layout().await;
    let dir_path = fixture_dir.path();

    let command: String = [
        "cargo run",
        "--package=rpgdemake",
        "--",
        &dir_path.to_string_lossy()
    ].join( " " );

    colour::e_blue_ln!( "Run hyperfine" );

    hyperfine.arg( command );

    hyperfine.spawn().unwrap()
        .wait().unwrap();

}


async fn setup_fs_layout() -> TempDir {

    colour::e_blue_ln!( "Setup fixture layout" );

    /*
     * Get tempdir
     */

    let dir = TempDir::new()
        .expect( "Failed to obtain tempdir" )
        //.into_persistent()
    ;

    /*
     * Pretent that this is MV
     */

    dir.child( "nw.dll" ).touch().unwrap();
    dir.child( "Game.exe" ).touch().unwrap();

    dir.child( "www/data" ).create_dir_all().unwrap();
    dir.child( "www/img" ).create_dir_all().unwrap();
    dir.child( "www/audio" ).create_dir_all().unwrap();

    /*
     * Write System.json
     */

    dir.child( "www/data/System.json" ).write_str( SYSTEM_JSON ).unwrap();

    /*
     * Setup fixture
     */

    let mut handles = vec![];

    async fn write_asset( dir: ChildPath ) {
        let path = dir.path();
        let ( result, _ ) = monoio::fs::write( &path, BINARY_ASSET ).await;
        result.expect( "Failed to write binary assets" );
    }

    for id in 0..5_000 {
        let path = dir.child( format!( "www/img/{id}.rpgmvp" ) );
        handles.push(
            monoio::spawn( write_asset( path ) )
        );
    }

    futures::future::join_all( handles ).await;


    /*
     * Return the tempdir
     */

    dir

}
