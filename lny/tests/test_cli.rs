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

fn read_json(path: &Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn assert_success(output: &std::process::Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_migration(
    new_blueprint: &Path,
    state: &Path,
) -> std::process::Output {
    make_app!()
        .arg("--new-blueprint")
        .arg(new_blueprint)
        .arg("--state")
        .arg(state)
        .output()
        .unwrap()
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
            .arg("--state")
            .arg(old_bp.path())
            .spawn()
            .unwrap();

        let ret = cmd_process.wait().unwrap();

        assert!(ret.success());

        assert_eq!(read_json(old_bp.path()), read_json(new_bp.path()));

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
fn unchanged_declaration_repairs_missing_symlink() {
    let top = make_tempdir!();
    let source = top.child("source").tap(|it| it.touch().unwrap());
    let destination = top.child("destination");
    let symlinks = [(source.path(), destination.path())];
    let state = write_blueprint(&top, "state.json", &symlinks);
    let new_blueprint = write_blueprint(&top, "new.json", &symlinks);

    let output = run_migration(new_blueprint.path(), state.path());

    assert_success(&output);
    assert_eq!(destination.read_link().unwrap(), source.path());
    assert_eq!(read_json(state.path()), read_json(new_blueprint.path()));
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

    let state = write_blueprint(
        &top,
        "state.json",
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

    let output = run_migration(new_blueprint.path(), state.path());

    assert_success(&output);
    assert!(destination_dir.is_symlink());
    assert_eq!(destination_dir.read_link().unwrap(), source_dir.path());
    assert_eq!(read_json(state.path()), read_json(new_blueprint.path()));
}

#[test]
fn collapse_missing_tree_and_completed_retry() {
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
    let old_destination = destination_dir.child("__moonstep.fish");

    let state = write_blueprint(
        &top,
        "state.json",
        &[(source_file.path(), old_destination.path())],
    );
    let new_blueprint = write_blueprint(
        &top,
        "new.json",
        &[(source_dir.path(), destination_dir.path())],
    );

    let first_run = run_migration(new_blueprint.path(), state.path());
    assert_success(&first_run);
    assert_eq!(destination_dir.read_link().unwrap(), source_dir.path());

    let retry = run_migration(new_blueprint.path(), state.path());
    assert_success(&retry);
    assert_eq!(destination_dir.read_link().unwrap(), source_dir.path());
    assert_eq!(std::fs::read_to_string(source_file).unwrap(), "moonstep");
}

#[test]
fn collapse_recovers_missing_nested_descendant() {
    let top = make_tempdir!();
    let source_dir =
        top.child("source").tap(|it| it.create_dir_all().unwrap());
    let nested_source_dir = source_dir
        .child("nested")
        .tap(|it| it.create_dir_all().unwrap());
    let source_file = nested_source_dir
        .child("file")
        .tap(|it| it.write_str("content").unwrap());

    let destination_dir = top
        .child("destination")
        .tap(|it| it.create_dir_all().unwrap());
    let nested_destination_dir = destination_dir
        .child("nested")
        .tap(|it| it.create_dir_all().unwrap());
    let missing_destination = nested_destination_dir.child("file");

    let state = write_blueprint(
        &top,
        "state.json",
        &[(source_file.path(), missing_destination.path())],
    );
    let new_blueprint = write_blueprint(
        &top,
        "new.json",
        &[(source_dir.path(), destination_dir.path())],
    );

    let output = run_migration(new_blueprint.path(), state.path());

    assert_success(&output);
    assert_eq!(destination_dir.read_link().unwrap(), source_dir.path());
    assert_eq!(
        std::fs::read_to_string(missing_destination).unwrap(),
        "content"
    );
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

    let state = write_blueprint(
        &top,
        "state.json",
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

    let output = run_migration(new_blueprint.path(), state.path());

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

    // Retry from a partially expanded real directory.
    std::fs::remove_file(second_destination.path()).unwrap();
    let unmanaged = destination_dir
        .child("unmanaged")
        .tap(|it| it.write_str("keep").unwrap());
    let retry = run_migration(new_blueprint.path(), state.path());

    assert_success(&retry);
    assert_eq!(
        first_destination.read_link().unwrap(),
        first_source.path()
    );
    assert_eq!(
        second_destination.read_link().unwrap(),
        second_source.path()
    );
    assert_eq!(std::fs::read_to_string(unmanaged).unwrap(), "keep");
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

    let state = write_blueprint(
        &top,
        "state.json",
        &[(source_file.path(), managed_destination.path())],
    );
    let seeded_state = std::fs::read_to_string(state.path()).unwrap();
    let new_blueprint = write_blueprint(
        &top,
        "new.json",
        &[(source_dir.path(), destination_dir.path())],
    );

    let output = run_migration(new_blueprint.path(), state.path());

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
    assert_eq!(
        std::fs::read_to_string(state.path()).unwrap(),
        seeded_state
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

    let state = write_blueprint(
        &top,
        "state.json",
        &[(expected_source.path(), destination.path())],
    );
    let seeded_state = std::fs::read_to_string(state.path()).unwrap();
    let new_blueprint = write_blueprint(
        &top,
        "new.json",
        &[(source_dir.path(), destination_dir.path())],
    );

    let output = run_migration(new_blueprint.path(), state.path());

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("not controlled by us")
    );
    assert_eq!(destination.read_link().unwrap(), foreign_source.path());
    assert!(destination_dir.symlink_metadata().unwrap().is_dir());
    assert_eq!(
        std::fs::read_to_string(state.path()).unwrap(),
        seeded_state
    );
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

#[test]
fn state_without_new_blueprint_does_nothing() {
    let top = make_tempdir!();
    let state = top.child("state.json");
    state.write_str("sentinel").unwrap();

    let output = make_app!()
        .arg("--state")
        .arg(state.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    assert_eq!(std::fs::read_to_string(state.path()).unwrap(), "sentinel");
}

#[test]
fn first_run_writes_state() {
    let top = make_tempdir!();
    let src = top.child("src").tap(|it| it.touch().unwrap());
    let dst = top.child("dst");

    let new_blueprint =
        write_blueprint(&top, "new.json", &[(src.path(), dst.path())]);
    let state = top.child("state.json");

    let output = run_migration(new_blueprint.path(), state.path());

    assert_success(&output);
    assert!(dst.is_symlink());
    assert_eq!(dst.read_link().unwrap(), src.path());

    assert!(state.is_file());
    assert_eq!(read_json(state.path()), read_json(new_blueprint.path()));
    assert!(
        std::fs::read_to_string(state.path())
            .unwrap()
            .contains(src.path().to_str().unwrap())
    );
}

#[test]
fn rerun_same_blueprint_is_noop_with_stable_state() {
    let top = make_tempdir!();
    let src = top.child("src").tap(|it| it.touch().unwrap());
    let dst = top.child("dst");

    let new_blueprint =
        write_blueprint(&top, "new.json", &[(src.path(), dst.path())]);
    let state = top.child("state.json");

    assert_success(&run_migration(new_blueprint.path(), state.path()));
    let first_state = std::fs::read_to_string(state.path()).unwrap();

    assert_success(&run_migration(new_blueprint.path(), state.path()));

    assert_eq!(
        std::fs::read_to_string(state.path()).unwrap(),
        first_state
    );
    assert_eq!(dst.read_link().unwrap(), src.path());
}

#[test]
fn migration_removes_old_and_records_new_state() {
    let top = make_tempdir!();
    let gone_src = top.child("gone_src").tap(|it| it.touch().unwrap());
    let old_src = top.child("old_src").tap(|it| it.touch().unwrap());
    let new_src = top.child("new_src").tap(|it| it.touch().unwrap());

    let gone_dst = top
        .child("gone")
        .tap(|it| it.symlink_to_file(&gone_src).unwrap());
    let replaced_dst = top
        .child("replaced")
        .tap(|it| it.symlink_to_file(&old_src).unwrap());
    let fresh_dst = top.child("fresh");

    let state = write_blueprint(
        &top,
        "state.json",
        &[
            (gone_src.path(), gone_dst.path()),
            (old_src.path(), replaced_dst.path()),
        ],
    );
    let new_blueprint = write_blueprint(
        &top,
        "new.json",
        &[
            (new_src.path(), replaced_dst.path()),
            (new_src.path(), fresh_dst.path()),
        ],
    );

    let output = run_migration(new_blueprint.path(), state.path());

    assert_success(&output);
    assert!(!gone_dst.try_exists_no_traverse().unwrap());
    assert_eq!(replaced_dst.read_link().unwrap(), new_src.path());
    assert_eq!(fresh_dst.read_link().unwrap(), new_src.path());
    assert_eq!(read_json(state.path()), read_json(new_blueprint.path()));
}

#[test]
fn corrupt_state_fails_without_mutation() {
    let top = make_tempdir!();
    let src = top.child("src").tap(|it| it.touch().unwrap());
    let dst = top.child("dst");

    let new_blueprint =
        write_blueprint(&top, "new.json", &[(src.path(), dst.path())]);
    let state = top.child("state.json");
    state.write_str("{ definitely not json").unwrap();

    let output = run_migration(new_blueprint.path(), state.path());

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("not a valid blueprint")
    );
    assert!(!dst.try_exists_no_traverse().unwrap());
    assert_eq!(
        std::fs::read_to_string(state.path()).unwrap(),
        "{ definitely not json"
    );
}

#[test]
fn state_directory_or_wrong_version_is_hard_error() {
    let top = make_tempdir!();
    let src = top.child("src").tap(|it| it.touch().unwrap());

    // state path is a directory
    {
        let dst = top.child("dst_a");
        let new_blueprint = write_blueprint(
            &top,
            "new_a.json",
            &[(src.path(), dst.path())],
        );
        let state =
            top.child("state_a").tap(|it| it.create_dir_all().unwrap());

        let output = run_migration(new_blueprint.path(), state.path());

        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains("not a regular file")
        );
        assert!(!dst.try_exists_no_traverse().unwrap());
        assert!(state.try_exists_no_traverse().unwrap());
    }

    // state records an unsupported version
    {
        let dst = top.child("dst_b");
        let new_blueprint = write_blueprint(
            &top,
            "new_b.json",
            &[(src.path(), dst.path())],
        );
        let state = top.child("state_b.json");
        state
            .write_str(
                &serde_json::json!({
                    "version": VERSION + 1,
                    "symlinks": [],
                })
                .to_string(),
            )
            .unwrap();
        let seeded = std::fs::read_to_string(state.path()).unwrap();

        let output = run_migration(new_blueprint.path(), state.path());

        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains("version mismatch")
        );
        assert!(!dst.try_exists_no_traverse().unwrap());
        assert_eq!(std::fs::read_to_string(state.path()).unwrap(), seeded);
    }
}

#[test]
fn interrupted_run_converges_on_retry() {
    use std::os::unix::fs::PermissionsExt as _;

    let top = make_tempdir!();
    let old1_src = top.child("old1_src").tap(|it| it.touch().unwrap());
    let old2_src = top.child("old2_src").tap(|it| it.touch().unwrap());
    let new_src = top.child("new_src").tap(|it| it.touch().unwrap());

    let keep = top.child("keep").tap(|it| it.create_dir_all().unwrap());
    let old1_dst = keep
        .child("old1")
        .tap(|it| it.symlink_to_file(&old1_src).unwrap());
    let new_dst = keep.child("new");

    let frozen =
        top.child("frozen").tap(|it| it.create_dir_all().unwrap());
    let old2_dst = frozen
        .child("old2")
        .tap(|it| it.symlink_to_file(&old2_src).unwrap());

    let state = write_blueprint(
        &top,
        "state.json",
        &[
            (old1_src.path(), old1_dst.path()),
            (old2_src.path(), old2_dst.path()),
        ],
    );
    let seeded_state = std::fs::read_to_string(state.path()).unwrap();
    let new_blueprint = write_blueprint(
        &top,
        "new.json",
        &[(new_src.path(), new_dst.path())],
    );

    // The read-only directory only fails at execute time: the new link
    // is ensured and the first removal applied before the run dies.
    std::fs::set_permissions(
        frozen.path(),
        std::fs::Permissions::from_mode(0o555),
    )
    .unwrap();

    let failed = run_migration(new_blueprint.path(), state.path());
    assert!(!failed.status.success());

    std::fs::set_permissions(
        frozen.path(),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();

    assert!(new_dst.is_symlink());
    assert_eq!(new_dst.read_link().unwrap(), new_src.path());
    assert_eq!(old2_dst.read_link().unwrap(), old2_src.path());
    assert_eq!(
        std::fs::read_to_string(state.path()).unwrap(),
        seeded_state
    );

    let retry = run_migration(new_blueprint.path(), state.path());
    assert_success(&retry);

    assert!(!old1_dst.try_exists_no_traverse().unwrap());
    assert!(!frozen.try_exists_no_traverse().unwrap());
    assert_eq!(new_dst.read_link().unwrap(), new_src.path());
    assert_eq!(read_json(state.path()), read_json(new_blueprint.path()));
}

#[test]
fn replace_recovers_after_interrupted_run() {
    let top = make_tempdir!();
    let old_src = top.child("old_src").tap(|it| it.touch().unwrap());
    let new_src = top.child("new_src").tap(|it| it.touch().unwrap());
    let dst = top
        .child("dst")
        .tap(|it| it.symlink_to_file(&new_src).unwrap());

    // The previous run swapped the link but died before recording it.
    let state = write_blueprint(
        &top,
        "state.json",
        &[(old_src.path(), dst.path())],
    );
    let new_blueprint =
        write_blueprint(&top, "new.json", &[(new_src.path(), dst.path())]);

    let output = run_migration(new_blueprint.path(), state.path());

    assert_success(&output);
    assert_eq!(dst.read_link().unwrap(), new_src.path());
    assert_eq!(read_json(state.path()), read_json(new_blueprint.path()));

    // A plain re-run from the recorded state stays a no-op.
    let rerun = run_migration(new_blueprint.path(), state.path());
    assert_success(&rerun);
    assert_eq!(dst.read_link().unwrap(), new_src.path());
}

#[test]
fn state_records_rendered_paths_not_templates() {
    use std::os::unix::fs::symlink;

    let top = make_tempdir!();
    let old_src = top.child("old-src").tap(|it| it.touch().unwrap());
    let new_src = top.child("new-src").tap(|it| it.touch().unwrap());

    let old_dst = top.child("old-dst");
    symlink(old_src.path(), old_dst.path()).unwrap();

    let seeded = serde_json::json!({
        "version": VERSION,
        "symlinks": [
            { "src": "{{ home }}/old-src", "dst": "{{ home }}/old-dst" }
        ]
    });
    let state = top.child("state.json");
    state.write_str(&seeded.to_string()).unwrap();

    let new_blueprint = top.child("new.json");
    new_blueprint
        .write_str(
            &serde_json::json!({
                "version": VERSION,
                "symlinks": [
                    {
                        "src": "{{ home }}/new-src",
                        "dst": "{{ home }}/new-dst"
                    }
                ]
            })
            .to_string(),
        )
        .unwrap();

    // Pin the XDG environment so `{{ home }}` renders deterministically.
    let mut app = make_app!();
    let output = app
        .env("HOME", top.path())
        .env("XDG_CONFIG_HOME", top.child(".config").path())
        .env("XDG_DATA_HOME", top.child(".local/share").path())
        .env("XDG_CACHE_HOME", top.child(".cache").path())
        .env("XDG_STATE_HOME", top.child(".local/state").path())
        .arg("--new-blueprint")
        .arg(new_blueprint.path())
        .arg("--state")
        .arg(state.path())
        .output()
        .unwrap();

    assert_success(&output);

    assert!(!old_dst.try_exists_no_traverse().unwrap());
    let new_dst = top.child("new-dst");
    assert_eq!(new_dst.read_link().unwrap(), new_src.path());

    let raw = std::fs::read_to_string(state.path()).unwrap();
    assert!(
        !raw.contains("{{"),
        "state must not contain template text: {raw}"
    );
    assert_eq!(
        read_json(state.path()),
        serde_json::json!({
            "version": VERSION,
            "symlinks": [
                { "src": new_src.path(), "dst": new_dst.path() }
            ]
        })
    );
}

// A symlinked state is rejected as not a regular file: a stale link
// must not silently read through to whatever it points at.
#[test]
fn state_symlink_to_blueprint_is_hard_error() {
    use std::os::unix::fs::symlink;

    let top = make_tempdir!();
    let src = top.child("src").tap(|it| it.touch().unwrap());
    let dst = top.child("dst");

    let new_blueprint =
        write_blueprint(&top, "new.json", &[(src.path(), dst.path())]);
    let state = top.child("state.json");
    symlink(new_blueprint.path(), state.path()).unwrap();

    let output = run_migration(new_blueprint.path(), state.path());

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("not a regular file")
    );
    assert!(!dst.try_exists_no_traverse().unwrap());
    assert_eq!(state.read_link().unwrap(), new_blueprint.path());
}
