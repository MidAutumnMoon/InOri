#![expect(clippy::unwrap_used, reason = "Tests")]
#![expect(clippy::panic, reason = "Tests")]
#![expect(clippy::missing_assert_message, reason = "Tests")]

use std::path::PathBuf;

use assert_fs::prelude::*;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixture")
        .join(name)
}

fn fixture_bytes(name: &str) -> Vec<u8> {
    std::fs::read(fixture(name)).unwrap()
}

enum Version {
    MV,
    MZ,
}

struct Layout {
    version: Version,
    dir: assert_fs::TempDir,
}

impl Layout {
    fn new(version: Version) -> Self {
        let dir = assert_fs::TempDir::new().unwrap();
        Self { version, dir }
    }

    fn path(&self) -> &std::path::Path {
        self.dir.path()
    }

    fn base_dir(&self) -> PathBuf {
        match self.version {
            Version::MV => self.dir.path().join("www"),
            Version::MZ => self.dir.path().to_owned(),
        }
    }

    fn setup_system_json(&self) {
        let dir = self.base_dir();
        let path = dir.join("data/System.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, include_str!("./fixture/System.json"))
            .unwrap();
    }

    fn setup_layout(&self) {
        self.dir.child("locales").touch().unwrap();

        let dir = self.base_dir();
        let mapping: [(&str, &str); 2] = match self.version {
            Version::MV => [
                ("img/pictures/Clouds.rpgmvp", "Clouds.rpgmvp"),
                ("audio/bgm/Castle1.rpgmvo", "Castle1.rpgmvo"),
            ],
            Version::MZ => [
                ("img/pictures/Clouds.png_", "Clouds.rpgmvp"),
                ("audio/bgm/Castle1.ogg_", "Castle1.rpgmvo"),
            ],
        };

        for (child, src_name) in mapping {
            let path = dir.join(child);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, fixture_bytes(src_name)).unwrap();
        }

        // Non-RPG-Maker file — tool must ignore it
        std::fs::write(dir.join("junk-to-be-ignored"), b"").unwrap();
    }

    fn decrypted_png_path(&self) -> PathBuf {
        self.base_dir().join("img/pictures/Clouds.png")
    }

    fn decrypted_ogg_path(&self) -> PathBuf {
        self.base_dir().join("audio/bgm/Castle1.ogg")
    }

    fn encrypted_audio_path(&self) -> PathBuf {
        let name = match self.version {
            Version::MV => "audio/bgm/Castle1.rpgmvo",
            Version::MZ => "audio/bgm/Castle1.ogg_",
        };
        self.base_dir().join(name)
    }

    fn junk_path(&self) -> PathBuf {
        self.base_dir().join("junk-to-be-ignored")
    }
}

fn assert_file_matches(path: &std::path::Path, expected: &[u8]) {
    let actual = std::fs::read(path).unwrap_or_else(|e| {
        panic!("expected file {} to exist: {e}", path.display())
    });
    assert_eq!(
        actual,
        expected,
        "content mismatch for {}",
        path.display()
    );
}

fn assert_file_not_exists(path: &std::path::Path) {
    assert!(
        !path.try_exists().unwrap(),
        "file should not exist: {}",
        path.display()
    );
}

fn run_main_program(dir: &std::path::Path, mode: &str) {
    let exe_path = std::env!("CARGO_BIN_EXE_rpgdemake");

    let status = std::process::Command::new(exe_path)
        .arg(dir)
        .arg("--mode")
        .arg(mode)
        .spawn()
        .unwrap()
        .wait()
        .unwrap();

    assert!(status.success());
}

fn check_full_decrypt(version: Version) {
    let layout = Layout::new(version);

    layout.setup_system_json();
    layout.setup_layout();

    run_main_program(layout.path(), "full");

    // Both PNG and OGG should be decrypted correctly
    assert_file_matches(
        &layout.decrypted_png_path(),
        &fixture_bytes("Clouds.png"),
    );
    assert_file_matches(
        &layout.decrypted_ogg_path(),
        &fixture_bytes("Castle1.ogg"),
    );
    // Junk file should still exist
    assert!(layout.junk_path().try_exists().unwrap());
}

fn check_light_decrypt(version: Version) {
    let layout = Layout::new(version);

    layout.setup_layout();

    run_main_program(layout.path(), "light");

    // Only PNG should be decrypted
    assert_file_matches(
        &layout.decrypted_png_path(),
        &fixture_bytes("Clouds.png"),
    );
    // Audio should NOT be decrypted —
    // encrypted file unchanged, no .ogg output
    assert_file_matches(
        &layout.encrypted_audio_path(),
        &fixture_bytes("Castle1.rpgmvo"),
    );
    assert_file_not_exists(&layout.decrypted_ogg_path());
    // Junk file should still exist
    assert!(layout.junk_path().try_exists().unwrap());
}

#[test]
fn mv_full() {
    check_full_decrypt(Version::MV);
}

#[test]
fn mz_full() {
    check_full_decrypt(Version::MZ);
}

#[test]
fn mv_light() {
    check_light_decrypt(Version::MV);
}

#[test]
fn mz_light() {
    check_light_decrypt(Version::MZ);
}
