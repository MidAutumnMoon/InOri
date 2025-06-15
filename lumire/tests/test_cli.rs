use assert_fs::prelude::*;
use assert_fs::TempDir;

use std::process::Command;

const VERSION: usize = 1;

fn main_program() -> Command {
    let exe = std::env!( "CARGO_BIN_EXE_lumire" );
    let mut cmd = std::process::Command::new( exe );
    cmd.env( "RUST_LOG", "trace" );
    cmd
}

macro_rules! create_tempdir {
    () => { {
        TempDir::new().expect( "Failed to setup tempdir" )
    } };
}

#[ test ]
#[ allow( clippy::unwrap_used ) ]
#[ allow( clippy::expect_used ) ]
fn test_create_symlink() {
    ino_tracing::init_tracing_subscriber();

    let mut app = main_program();
    let top = create_tempdir!();

    let message = "Helo!!!!!!!!!!!!!";
    let mode = "755";

    let src = top.child( "this-source" );
    src.write_str( message ).unwrap();

    let dst = top.child( "link-here" );

    let json = serde_json::json!( {
        "version": VERSION,
        "symlinks": [ {
            "src": src.path(),
            "dst": dst.path(),
            "mode": mode,
        } ]
    } ).to_string();

    let new = top.child( "new.json" );
    new.write_str( &json ).unwrap();

    let mut child = app
        .arg( "--new-plan" ).arg( new.path() )
        .spawn().unwrap();

    let ret = child.wait().unwrap();

    assert!( ret.success() );

}

#[ test ]
fn test_collinsion_precheck() {}

#[ test ]
#[ allow( clippy::unwrap_used ) ]
fn abs_path() {
    let mut app = main_program();
    let top = create_tempdir!();

    let json = serde_json::json!( {
        "version": VERSION,
        "symlinks": [
            {
                "src": "not abs",
                "dst": "not asb",
                "mode": "755",
            }
        ],
    } ).to_string();

    let new = top.child( "new.json" );
    new.write_str( &json ).unwrap();

    let res = app
        .arg( "--new-plan" ).arg( new.path() )
        .output().unwrap()
    ;

    assert!( !res.status.success() );
    assert!( String::from_utf8_lossy( &res.stderr )
        .contains( "Path must be absolute" )
    );
}
