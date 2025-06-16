#![ allow( clippy::unwrap_used ) ]
#![ allow( clippy::expect_used ) ]

use assert_fs::prelude::*;
use assert_fs::TempDir;
use tap::Tap;

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

    let src = top.child( "this-source" )
        .tap( |it| it.touch().unwrap() );

    let dst = top.child( "link-here" );

    let new_plan = {
        let json = serde_json::json!( {
            "version": VERSION,
            "symlinks": [ {
                "src": src.path(),
                "dst": dst.path(),
            } ]
        } ).to_string();
        top.child( "new_plan.json" )
            .tap( |it| it.write_str( &json ).unwrap() )
    };

    let mut cmd_process = app
        .arg( "--new-plan" ).arg( new_plan.path() )
        .spawn().unwrap();

    let ret = cmd_process.wait().unwrap();

    assert!( ret.success() );
    assert!( dst.path().is_symlink() );
    assert!( dst.path().read_link().unwrap() == src.path() );
}

#[ test ]
fn collision_create_symlink() {}

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
        top.child( "old_plan.json" )
            .tap( |it| it.write_str( &json ).unwrap() )
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
fn collision_remove_old_symlinks() {}

#[ test ]
fn replace_symlinks() {}

#[ test ]
fn collinsion_replace_symlinks() {}

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
