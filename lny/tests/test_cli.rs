#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::tests_outside_test_module,
    reason = "Tests"
)]

use assert_fs::TempDir;
use assert_fs::fixture::ChildPath;
use assert_fs::prelude::*;
use ino_path::PathExt as _;
use rand::RngExt as _;
use std::path::Path;
use tap::Tap as _;

const VERSION: usize = 1;

macro_rules! make_app {
    () => {{
        let exe = std::env!("CARGO_BIN_EXE_lny");
        let cmd = std::process::Command::new(exe);
        cmd
    }};
}

macro_rules! make_tempdir {
    () => {{ TempDir::new().expect("Failed to setup tempdir") }};
}

macro_rules! make_random_str {
    () => {{
        use rand::distr::Alphanumeric;
        rand::rng()
            .sample_iter(&Alphanumeric)
            .take(8)
            .map(char::from)
            .collect::<String>()
    }};
}

fn write_blueprint(
    top: &TempDir,
    name: &str,
    symlinks: &[(&Path, &Path)],
) -> ChildPath {
    let symlinks = symlinks
        .iter()
        .map(|(src, dst)| serde_json::json!({ "src": src, "dst": dst }))
        .collect::<Vec<_>>();
    let blueprint = serde_json::json!({
        "version": VERSION,
        "symlinks": symlinks,
    });
    top.child(name)
        .tap(|it| it.write_str(&blueprint.to_string()).unwrap())
}

fn assert_success(output: &std::process::Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn typical_workload() {
    use std::os::unix::fs::symlink;

    // first run

    {
        let top = make_tempdir!();
        let mut app = make_app!();

        let sym_src = top.child("sym_src").tap(|it| it.touch().unwrap());

        let sym_dst = top.child("sym_dst");

        let norm_file =
            top.child("f").tap(|it| it.write_str("f").unwrap());

        let new_bp = {
            let j = serde_json::json! { {
                "version": VERSION,
                "symlinks": [
                    { "src": sym_src.path(), "dst": sym_dst.path() },
                ]
            } };
            top.child("new_blueprint.json")
                .tap(|it| it.write_str(&j.to_string()).unwrap())
        };

        let mut cmd_process = app
            .arg("--new-blueprint")
            .arg(new_bp.path())
            .spawn()
            .unwrap();

        let ret = cmd_process.wait().unwrap();

        assert!(ret.success());
        assert!(
            sym_dst.is_symlink()
                && sym_dst.read_link().unwrap() == sym_src.path()
        );
        assert_eq!(std::fs::read_to_string(norm_file).unwrap(), "f");
    }

    // normal uses

    {
        let top = make_tempdir!();
        let mut app = make_app!();

        let dir = top
            .child(make_random_str!())
            .tap(|it| it.create_dir_all().unwrap());

        let new_subdir = dir
            .child(make_random_str!())
            .tap(|it| it.create_dir_all().unwrap());
        let old_subdir = top
            .child(make_random_str!())
            .tap(|it| it.create_dir_all().unwrap());

        let norm_file_content = make_random_str!();
        let norm_file = dir
            .child(make_random_str!())
            .tap(|it| it.write_str(&norm_file_content).unwrap());

        let to_remove_src =
            top.child(make_random_str!()).tap(|it| it.touch().unwrap());
        let to_remove_dst = old_subdir.child(make_random_str!());
        symlink(&to_remove_src, &to_remove_dst).unwrap();

        let to_replace_dst = top.child(make_random_str!());
        let to_replace_old_src =
            top.child(make_random_str!()).tap(|it| it.touch().unwrap());
        symlink(&to_replace_old_src, &to_replace_dst).unwrap();
        let to_replace_new_src =
            top.child(make_random_str!()).tap(|it| it.touch().unwrap());

        let to_create_src =
            top.child(make_random_str!()).tap(|it| it.touch().unwrap());
        let to_create_dst = new_subdir.child(make_random_str!());

        let nothing_src =
            top.child(make_random_str!()).tap(|it| it.touch().unwrap());
        let nothing_dst = top
            .child(make_random_str!())
            .tap(|it| it.symlink_to_file(&nothing_src).unwrap());

        let old_bp = {
            let j = serde_json::json! { {
                "version": VERSION,
                "symlinks": [
                    { "src": to_remove_src.path(), "dst": to_remove_dst.path() },
                    {
                        "src": to_replace_old_src.path(),
                        "dst": to_replace_dst.path()
                    },
                ]
            } };
            top.child(make_random_str!())
                .tap(|it| it.write_str(&j.to_string()).unwrap())
        };

        let new_bp = {
            let j = serde_json::json! { {
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
            top.child(make_random_str!())
                .tap(|it| it.write_str(&j.to_string()).unwrap())
        };

        let mut cmd_process = app
            .arg("--new-blueprint")
            .arg(new_bp.path())
            .arg("--old-blueprint")
            .arg(old_bp.path())
            .spawn()
            .unwrap();

        let ret = cmd_process.wait().unwrap();

        assert!(ret.success());

        assert_eq!(
            std::fs::read_to_string(norm_file).unwrap(),
            norm_file_content
        );

        assert!(!to_remove_dst.try_exists_no_traverse().unwrap());
        assert!(!old_subdir.try_exists_no_traverse().unwrap());

        assert!(
            to_replace_dst.is_symlink()
                && to_replace_dst.read_link().unwrap()
                    == to_replace_new_src.path()
        );

        assert!(
            new_subdir.try_exists_no_traverse().unwrap()
                && new_subdir.symlink_metadata().unwrap().is_dir()
        );
        assert!(
            to_create_dst.is_symlink()
                && to_create_dst.read_link().unwrap()
                    == to_create_src.path()
        );

        assert!(
            nothing_dst.is_symlink()
                && nothing_dst.read_link().unwrap() == nothing_src.path()
        );
    }
}

#[test]
fn collapse_children_into_parent_symlink() {
    let top = make_tempdir!();
    let source_dir = top
        .child("fish/conf.d")
        .tap(|it| it.create_dir_all().unwrap());
    let moonstep_src = source_dir
        .child("__moonstep.fish")
        .tap(|it| it.write_str("moonstep").unwrap());
    let git_abbr_src = source_dir
        .child("git-abbr.fish")
        .tap(|it| it.write_str("git-abbr").unwrap());

    let destination_dir = top
        .child("config/fish/conf.d")
        .tap(|it| it.create_dir_all().unwrap());
    let moonstep_dst = destination_dir
        .child("__moonstep.fish")
        .tap(|it| it.symlink_to_file(&moonstep_src).unwrap());
    let git_abbr_dst = destination_dir
        .child("git-abbr.fish")
        .tap(|it| it.symlink_to_file(&git_abbr_src).unwrap());

    let old_blueprint = write_blueprint(
        &top,
        "old.json",
        &[
            (moonstep_src.path(), moonstep_dst.path()),
            (git_abbr_src.path(), git_abbr_dst.path()),
        ],
    );
    let new_blueprint = write_blueprint(
        &top,
        "new.json",
        &[(source_dir.path(), destination_dir.path())],
    );

    let output = make_app!()
        .arg("--new-blueprint")
        .arg(new_blueprint.path())
        .arg("--old-blueprint")
        .arg(old_blueprint.path())
        .output()
        .unwrap();

    assert_success(&output);
    assert!(destination_dir.is_symlink());
    assert_eq!(destination_dir.read_link().unwrap(), source_dir.path());
}

#[test]
fn collapse_retry_does_not_traverse_new_parent_symlink() {
    let top = make_tempdir!();
    let source_dir = top
        .child("fish/conf.d")
        .tap(|it| it.create_dir_all().unwrap());
    let source_file = source_dir
        .child("__moonstep.fish")
        .tap(|it| it.write_str("moonstep").unwrap());

    let destination_parent = top
        .child("config/fish")
        .tap(|it| it.create_dir_all().unwrap());
    let destination_dir = destination_parent.child("conf.d");
    destination_dir.symlink_to_dir(&source_dir).unwrap();
    let old_destination = destination_dir.child("__moonstep.fish");

    let old_blueprint = write_blueprint(
        &top,
        "old.json",
        &[(source_file.path(), old_destination.path())],
    );
    let new_blueprint = write_blueprint(
        &top,
        "new.json",
        &[(source_dir.path(), destination_dir.path())],
    );

    let output = make_app!()
        .arg("--new-blueprint")
        .arg(new_blueprint.path())
        .arg("--old-blueprint")
        .arg(old_blueprint.path())
        .output()
        .unwrap();

    assert_success(&output);
    assert_eq!(destination_dir.read_link().unwrap(), source_dir.path());
    assert_eq!(std::fs::read_to_string(source_file).unwrap(), "moonstep");
}

#[test]
fn expand_parent_symlink_into_child_symlinks() {
    let top = make_tempdir!();
    let old_source_dir = top
        .child("old-source")
        .tap(|it| it.create_dir_all().unwrap());
    let new_source_dir = top
        .child("new-source")
        .tap(|it| it.create_dir_all().unwrap());
    let first_source = new_source_dir
        .child("first")
        .tap(|it| it.write_str("first").unwrap());
    let second_source = new_source_dir
        .child("second")
        .tap(|it| it.write_str("second").unwrap());

    let destination_parent = top
        .child("config/fish")
        .tap(|it| it.create_dir_all().unwrap());
    let destination_dir = destination_parent.child("conf.d");
    destination_dir.symlink_to_dir(&old_source_dir).unwrap();
    let first_destination = destination_dir.child("first");
    let second_destination = destination_dir.child("second");

    let old_blueprint = write_blueprint(
        &top,
        "old.json",
        &[(old_source_dir.path(), destination_dir.path())],
    );
    let new_blueprint = write_blueprint(
        &top,
        "new.json",
        &[
            (first_source.path(), first_destination.path()),
            (second_source.path(), second_destination.path()),
        ],
    );

    let output = make_app!()
        .arg("--new-blueprint")
        .arg(new_blueprint.path())
        .arg("--old-blueprint")
        .arg(old_blueprint.path())
        .output()
        .unwrap();

    assert_success(&output);
    assert!(destination_dir.symlink_metadata().unwrap().is_dir());
    assert!(!destination_dir.is_symlink());
    assert_eq!(
        first_destination.read_link().unwrap(),
        first_source.path()
    );
    assert_eq!(
        second_destination.read_link().unwrap(),
        second_source.path()
    );
    assert!(
        !old_source_dir
            .child("first")
            .try_exists_no_traverse()
            .unwrap()
    );
    assert!(
        !old_source_dir
            .child("second")
            .try_exists_no_traverse()
            .unwrap()
    );
}

#[test]
fn collapse_refuses_unmanaged_directory_entries_before_mutating() {
    let top = make_tempdir!();
    let source_dir =
        top.child("source").tap(|it| it.create_dir_all().unwrap());
    let source_file = source_dir
        .child("managed")
        .tap(|it| it.write_str("managed").unwrap());
    let destination_dir = top
        .child("destination")
        .tap(|it| it.create_dir_all().unwrap());
    let managed_destination = destination_dir
        .child("managed")
        .tap(|it| it.symlink_to_file(&source_file).unwrap());
    let unmanaged_destination = destination_dir
        .child("unmanaged")
        .tap(|it| it.write_str("unmanaged").unwrap());

    let old_blueprint = write_blueprint(
        &top,
        "old.json",
        &[(source_file.path(), managed_destination.path())],
    );
    let new_blueprint = write_blueprint(
        &top,
        "new.json",
        &[(source_dir.path(), destination_dir.path())],
    );

    let output = make_app!()
        .arg("--new-blueprint")
        .arg(new_blueprint.path())
        .arg("--old-blueprint")
        .arg(old_blueprint.path())
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("contains paths not controlled by lny")
    );
    assert_eq!(
        managed_destination.read_link().unwrap(),
        source_file.path()
    );
    assert_eq!(
        std::fs::read_to_string(unmanaged_destination).unwrap(),
        "unmanaged"
    );
}

#[test]
fn collapse_refuses_retargeted_child_before_mutating() {
    let top = make_tempdir!();
    let source_dir =
        top.child("source").tap(|it| it.create_dir_all().unwrap());
    let expected_source = source_dir
        .child("managed")
        .tap(|it| it.write_str("managed").unwrap());
    let foreign_source = top
        .child("foreign")
        .tap(|it| it.write_str("foreign").unwrap());
    let destination_dir = top
        .child("destination")
        .tap(|it| it.create_dir_all().unwrap());
    let destination = destination_dir
        .child("managed")
        .tap(|it| it.symlink_to_file(&foreign_source).unwrap());

    let old_blueprint = write_blueprint(
        &top,
        "old.json",
        &[(expected_source.path(), destination.path())],
    );
    let new_blueprint = write_blueprint(
        &top,
        "new.json",
        &[(source_dir.path(), destination_dir.path())],
    );

    let output = make_app!()
        .arg("--new-blueprint")
        .arg(new_blueprint.path())
        .arg("--old-blueprint")
        .arg(old_blueprint.path())
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("not controlled by us")
    );
    assert_eq!(destination.read_link().unwrap(), foreign_source.path());
    assert!(destination_dir.symlink_metadata().unwrap().is_dir());
}

#[test]
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
    } )
    .to_string();

    let new = top.child("new.json");
    new.write_str(&json).unwrap();

    let res = app.arg("--new-blueprint").arg(new.path()).output().unwrap();

    assert!(!res.status.success());
    assert!(
        String::from_utf8_lossy(&res.stderr)
            .contains("Path must be absolute")
    );
}
