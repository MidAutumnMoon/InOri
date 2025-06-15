
#![ allow( clippy::expect_used ) ]

use assert_fs::prelude::*;
use assert_fs::TempDir;

use std::process::Command;

const VERSION: usize = 1;

fn main_program() -> Command {
    let exe = std::env!( "CARGO_BIN_EXE_lumire" );
    #[ allow( unused_mut ) ]
    let mut cmd = std::process::Command::new( exe );
    // cmd.env( "RUST_LOG", "trace" );
    cmd
}

macro_rules! create_tempdir {
    () => { {
        TempDir::new().expect( "Failed to setup tempdir" )
    } };
}

#[ test ]
fn create_symlink() {
    let mut app = main_program();
    let top = create_tempdir!();

    let src = top.child( "this-source" );
    src.write_str( "hellllooo" ).unwrap();
    let dst = top.child( "link-here" );

    let json = serde_json::json!( {
        "version": VERSION,
        "symlinks": [ {
            "src": src.path(),
            "dst": dst.path(),
        } ]
    } ).to_string();

    let new_plan = top.child( "new_plan.json" );
    new_plan.write_str( &json ).unwrap();

    let mut cmd_process = app
        .arg( "--new-plan" ).arg( new_plan.path() )
        .spawn().unwrap();

    let ret = cmd_process.wait().unwrap();

    assert!( ret.success() );
    assert!( dst.path().is_symlink() );
    assert!( dst.path().read_link().unwrap() == src.path() );
}

#[ test ]
fn remove_old_symlinks() {
    use std::os::unix::fs::symlink;

    let mut app = main_program();
    let top = create_tempdir!();

    let src = top.child( "this-source" );
    src.write_str( "hellllooo" ).unwrap();
    let dst = top.child( "link-here" );

    symlink( src.path(), dst.path() ).unwrap();

    let old_plan = {
        let json = serde_json::json!{ {
            "version": VERSION,
            "symlinks": [ { "src": src.path(), "dst": dst.path(), } ]
        } }.to_string();
        let child = top.child( "old_plan.json" );
        child.write_str( &json ).unwrap();
        child
    };

    let mut cmd_process = app
        .arg( "--old-plan" ).arg( old_plan.path() )
        .spawn().unwrap();

    let ret = cmd_process.wait().unwrap();

    unimplemented!();
    assert!( ret.success() );
    assert!( dst.path().is_symlink() );
    assert!( dst.path().read_link().unwrap() == src.path() );

}

#[ test ]
fn test_collinsion_precheck() {
}

#[ test ]
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
