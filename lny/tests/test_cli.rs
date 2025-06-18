#![ allow( clippy::unwrap_used ) ]
#![ allow( clippy::expect_used ) ]

use assert_fs::prelude::*;
use assert_fs::TempDir;
use tap::Tap;

use std::process::Command;

const VERSION: usize = 1;

fn make_main_program() -> Command {
    let exe = std::env!( "CARGO_BIN_EXE_lny" );
    #[ allow( unused_mut ) ]
    let mut cmd = std::process::Command::new( exe );
    // cmd.env( "RUST_LOG", "trace" );
    cmd
}

macro_rules! make_tempdir {
    () => { {
        TempDir::new().expect( "Failed to setup tempdir" )
    } };
}

// Everything works according to plan.
// #[ test ]
// fn create_symlink_ok() {
//     let mut app = make_main_program();
//     let top = make_tempdir!();
//
//     let src = top.child( "this-source" ).tap( |it| it.touch().unwrap() );
//     let dst = top.child( "link-here" );
//
//     let new_blueprint = {
//         let json = serde_json::json!( {
//             "version": VERSION,
//             "symlinks": [ {
//                 "src": src.path(),
//                 "dst": dst.path(),
//             } ]
//         } ).to_string();
//         top.child( "new_blueprint.json" )
//             .tap( |it| it.write_str( &json ).unwrap() )
//     };
//
//     let mut cmd_process = app
//         .arg( "--new-blueprint" ).arg( new_blueprint.path() )
//         .spawn().unwrap();
//
//     let ret = cmd_process.wait().unwrap();
//
//     assert!( ret.success() );
//     assert!( dst.path().is_symlink() );
//     assert!( dst.path().read_link().unwrap() == src.path() );
// }
//
//
// #[ test ]
// fn remove_old_symlinks() {
//     use std::os::unix::fs::symlink;
//
//     let mut app = make_main_program();
//     let top = make_tempdir!();
//
//     let src = top.child( "this-source" );
//     src.write_str( "hellllooo" ).unwrap();
//     let dst = top.child( "link-here" );
//
//     symlink( src.path(), dst.path() ).unwrap();
//
//     let old_blueprint = {
//         let json = serde_json::json!{ {
//             "version": VERSION,
//             "symlinks": [ { "src": src.path(), "dst": dst.path(), } ]
//         } }.to_string();
//         top.child( "old_blueprint.json" )
//             .tap( |it| it.write_str( &json ).unwrap() )
//     };
//
//     let mut cmd_process = app
//         .arg( "--old-blueprint" ).arg( old_blueprint.path() )
//         .spawn().unwrap();
//
//     let ret = cmd_process.wait().unwrap();
//
//     unimplemented!();
//     assert!( ret.success() );
//     assert!( dst.path().is_symlink() );
//     assert!( dst.path().read_link().unwrap() == src.path() );
//
// }

#[ test ]
fn abs_path() {
    let mut app = make_main_program();
    let top = make_tempdir!();

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
        .arg( "--new-blueprint" ).arg( new.path() )
        .output().unwrap()
    ;

    assert!( !res.status.success() );
    assert!( String::from_utf8_lossy( &res.stderr )
        .contains( "Path must be absolute" )
    );
}
