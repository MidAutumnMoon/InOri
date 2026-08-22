#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::tests_outside_test_module,
    reason = "test code"
)]

mod common;

use std::path::Path;

use common::setup;

#[test]
fn push_dir() {
    let sh = setup();

    let d1 = sh.current_dir();
    {
        let sh = sh.with_current_dir("ino_shell-macros");
        let d2 = sh.current_dir();
        assert_eq!(d2, d1.join("ino_shell-macros"));
        {
            let sh = sh.with_current_dir("src");
            let d3 = sh.current_dir();
            assert_eq!(d3, d1.join("ino_shell-macros/src"));
        }
        let d4 = sh.current_dir();
        assert_eq!(d4, d1.join("ino_shell-macros"));
    }
    let d5 = sh.current_dir();
    assert_eq!(d5, d1);
}

#[test]
fn copy_file() {
    let sh = setup();

    let path;
    {
        let tempdir = sh.create_temp_dir().unwrap();
        path = tempdir.path().to_path_buf();
        let foo = tempdir.path().join("foo.txt");
        let bar = tempdir.path().join("bar.txt");
        let dir = tempdir.path().join("dir");
        sh.write_file(&foo, "hello world").unwrap();
        sh.create_dir(&dir).unwrap();

        sh.copy_file(&foo, &bar).unwrap();
        assert_eq!(sh.read_file(&bar).unwrap(), "hello world");

        sh.copy_file_to_dir(&foo, &dir).unwrap();
        assert_eq!(
            sh.read_file(dir.join("foo.txt")).unwrap(),
            "hello world"
        );
        assert!(path.exists());
    }
    assert!(!path.exists());
}

#[test]
fn exists() {
    let mut sh = setup();
    let tmp = sh.create_temp_dir().unwrap();
    sh.set_current_dir(tmp.path());
    assert!(!sh.path_exists("foo.txt"));
    sh.write_file("foo.txt", "foo").unwrap();
    assert!(sh.path_exists("foo.txt"));
    assert!(!sh.path_exists("bar"));
    sh.create_dir("bar").unwrap();
    assert!(sh.path_exists("bar"));
    sh.set_current_dir("bar");
    assert!(!sh.path_exists("quz.rs"));
    sh.write_file("quz.rs", "fn main () {}").unwrap();
    assert!(sh.path_exists("quz.rs"));
    sh.remove_path("quz.rs").unwrap();
    assert!(!sh.path_exists("quz.rs"));
}

#[test]
fn write_makes_directory() {
    let sh = setup();

    let tempdir = sh.create_temp_dir().unwrap();
    let folder = tempdir.path().join("some/nested/folder/structure");
    sh.write_file(folder.join(".gitinclude"), "").unwrap();
    assert!(folder.exists());
}

#[test]
fn remove_path() {
    let mut sh = setup();

    let tempdir = sh.create_temp_dir().unwrap();
    sh.set_current_dir(tempdir.path());
    sh.write_file(Path::new("a/b/c.rs"), "fn main() {}")
        .unwrap();
    assert!(tempdir.path().join("a/b/c.rs").exists());
    sh.remove_path("./a").unwrap();
    assert!(!tempdir.path().join("a/b/c.rs").exists());
    sh.remove_path("./a").unwrap();
}

#[test]
fn recovers_from_panics() {
    let sh = setup();

    let tempdir = sh.create_temp_dir().unwrap();
    let tempdir = tempdir.path().canonicalize().unwrap();

    let orig = sh.current_dir();

    std::panic::catch_unwind(|| {
        let sh = sh.with_current_dir(&tempdir);
        assert_eq!(sh.current_dir(), tempdir);
        std::panic::resume_unwind(Box::new(()));
    })
    .unwrap_err();

    assert_eq!(sh.current_dir(), orig);
    {
        let sh = sh.with_current_dir(&tempdir);
        assert_eq!(sh.current_dir(), tempdir);
    }
}
