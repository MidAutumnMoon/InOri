#![ allow( clippy::unwrap_used ) ]
#![ allow( clippy::expect_used ) ]

use assert_fs::prelude::*;
use assert_fs::TempDir;
use ino_path::PathExt;
use rand::RngExt;
use rand::rngs::ThreadRng;
use tap::Tap;

use std::process::Command;
use std::sync::LazyLock;

const VERSION: usize = 1;

macro_rules! make_app {
    () => { {
        let exe = std::env!( "CARGO_BIN_EXE_lny");
        let cmd = std::process::Command::new( exe );
        cmd
    } };
}

macro_rules! make_tempdir {
    () => { {
        TempDir::new().expect( "Failed to setup tempdir" )
    } };
}

macro_rules! make_random_str {
    () => { {
        use rand::distr::Alphanumeric;
        rand::rng().sample_iter( &Alphanumeric )
            .take( 8 )
            .map( char::from )
            .collect::<String>()
    } };
}

#[ test ]
fn typical_workload() {

    use std::os::unix::fs::symlink;

    // first run

    {
        let top = make_tempdir!();
        let mut app = make_app!();

        let sym_src = top.child( "sym_src" )
            .tap( |it| it.touch().unwrap() );

        let sym_dst = top.child( "sym_dst" );

        let norm_file = top.child( "f" )
            .tap( |it| it.write_str( "f" ).unwrap() );

        let new_bp = {
            let j = serde_json::json!{ {
                "version": VERSION,
                "symlinks": [
                    { "src": sym_src.path(), "dst": sym_dst.path() },
                ]
            } };
            top.child( "new_blueprint.json" )
                .tap( |it| it.write_str( &j.to_string() ).unwrap() )
        };

        let mut cmd_process = app
            .arg( "--new-blueprint" ).arg( new_bp.path() )
            .spawn().unwrap();

        let ret = cmd_process.wait().unwrap();

        assert!( ret.success() );
        assert!( sym_dst.is_symlink()
            && sym_dst.read_link().unwrap() == sym_src.path()
        );
        assert!( std::fs::read_to_string( norm_file ).unwrap() == "f" );
    }

    // normal uses

    {
        let top = make_tempdir!();
        let mut app = make_app!();

        let dir = top.child( make_random_str!() )
            .tap( |it| it.create_dir_all().unwrap() );

        let new_subdir = dir.child( make_random_str!() )
            .tap( |it| it.create_dir_all().unwrap() );
        let old_subdir = top.child( make_random_str!() )
            .tap( |it| it.create_dir_all().unwrap() );

        let norm_file_content = make_random_str!();
        let norm_file = dir.child( make_random_str!() )
            .tap( |it| it.write_str( &norm_file_content ).unwrap() );

        let to_remove_src = top.child( make_random_str!() )
            .tap( |it| it.touch().unwrap() );
        let to_remove_dst = old_subdir.child( make_random_str!() );
        symlink( &to_remove_src, &to_remove_dst ).unwrap();

        let to_replace_dst = top.child( make_random_str!() );
        let to_replace_old_src = top.child( make_random_str!() )
            .tap( |it| it.touch().unwrap() );
        symlink( &to_replace_old_src, &to_replace_dst ).unwrap();
        let to_replace_new_src = top.child( make_random_str!() )
            .tap( |it| it.touch().unwrap() );

        let to_create_src = top.child( make_random_str!() )
            .tap( |it| it.touch().unwrap() );
        let to_create_dst = new_subdir.child( make_random_str!() );

        let nothing_src = top.child( make_random_str!() )
            .tap( |it| it.touch().unwrap() );
        let nothing_dst = top.child( make_random_str!() )
            .tap( |it| it.symlink_to_file( &nothing_src ).unwrap() );

        let old_bp = {
            let j = serde_json::json!{ {
                "version": VERSION,
                "symlinks": [
                    { "src": to_remove_src.path(), "dst": to_remove_dst.path() },
                    {
                        "src": to_replace_old_src.path(),
                        "dst": to_replace_dst.path()
                    },
                ]
            } };
            top.child( make_random_str!() )
                .tap( |it| it.write_str( &j.to_string() ).unwrap() )
        };

        let new_bp = {
            let j = serde_json::json!{ {
                "version": VERSION,
                "symlinks": [
                    {
                        "src": to_replace_new_src.path(),
                        "dst": to_replace_dst.path()
                    },
                    { "src": to_create_src.path(), "dst": to_create_dst.path() },
                    {
                        "src": nothing_src.path(),
                        "dst": nothing_dst.path()
                    },
                ]
            } };
            top.child( make_random_str!() )
                .tap( |it| it.write_str( &j.to_string() ).unwrap() )
        };

        let mut cmd_process = app
            .arg( "--new-blueprint" ).arg( new_bp.path() )
            .arg( "--old-blueprint" ).arg( old_bp.path() )
            .spawn().unwrap();

        let ret = cmd_process.wait().unwrap();

        assert!( ret.success() );

        assert!( std::fs::read_to_string( norm_file ).unwrap()
            == norm_file_content );

        assert!( !to_remove_dst.try_exists_no_traverse().unwrap() );
        assert!( !old_subdir.try_exists_no_traverse().unwrap() );

        assert!( to_replace_dst.is_symlink()
            && to_replace_dst.read_link().unwrap() == to_replace_new_src.path()
        );

        assert!( new_subdir.try_exists_no_traverse().unwrap()
            && new_subdir.symlink_metadata().unwrap().is_dir()
        );
        assert!( to_create_dst.is_symlink()
            && to_create_dst.read_link().unwrap() == to_create_src.path()
        );

        assert!( nothing_dst.is_symlink()
            && nothing_dst.read_link().unwrap() == nothing_src.path()
        );
    }

}

#[ test ]
fn abs_path() {
    let mut app = make_app!();
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
