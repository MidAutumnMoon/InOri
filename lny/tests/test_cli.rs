#![ allow( clippy::unwrap_used ) ]
#![ allow( clippy::expect_used ) ]

use assert_fs::prelude::*;
use assert_fs::TempDir;
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
        use rand::Rng;
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

        let norm_file_content = make_random_str!();
        let norm_file = top.child( make_random_str!() )
            .tap( |it| it.write_str( &norm_file_content ).unwrap() );

        let old_sym_src = top.child( make_random_str!() )
            .tap( |it| it.touch().unwrap() );
        let old_sym_dst = top.child( make_random_str!() );
        symlink( &old_sym_src, &old_sym_dst ).unwrap();

        let rpl_sym_dst = top.child( make_random_str!() );
        let old_rpl_sym_src = top.child( make_random_str!() )
            .tap( |it| it.touch().unwrap() );
        symlink( &old_rpl_sym_src, &rpl_sym_dst ).unwrap();
        let new_rpl_sym_src = top.child( make_random_str!() )
            .tap( |it| it.touch().unwrap() );

        let new_sym_src = top.child( make_random_str!() )
            .tap( |it| it.touch().unwrap() );
        let new_sym_dst = top.child( make_random_str!() );

        let already_sym_src = top.child( make_random_str!() )
            .tap( |it| it.touch().unwrap() );
        let already_sym_dst = top.child( make_random_str!() );
        symlink( &already_sym_src, &already_sym_dst ).unwrap();

        let old_bp = {
            let j = serde_json::json!{ {
                "version": VERSION,
                "symlinks": [
                    { "src": old_sym_src.path(), "dst": old_sym_dst.path() },
                    {
                        "src": old_rpl_sym_src.path(),
                        "dst": rpl_sym_dst.path()
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
                        "src": new_rpl_sym_src.path(),
                        "dst": rpl_sym_dst.path()
                    },
                    { "src": new_sym_src.path(), "dst": new_sym_dst.path() },
                    {
                        "src": already_sym_src.path(),
                        "dst": already_sym_dst.path()
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

        assert!( !old_sym_dst.try_exists().unwrap() );

        assert!( rpl_sym_dst.is_symlink()
            && rpl_sym_dst.read_link().unwrap() == new_rpl_sym_src.path()
        );

        assert!( new_sym_dst.is_symlink()
            && new_sym_dst.read_link().unwrap() == new_sym_src.path()
        );

        assert!( already_sym_dst.is_symlink()
            && already_sym_dst.read_link().unwrap() == already_sym_src.path()
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
