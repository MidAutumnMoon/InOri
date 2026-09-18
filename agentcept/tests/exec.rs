//! End-to-end guards for the exec shim: resolving the real tool through
//! `$PATH` while skipping our own shims, passing the exit status through,
//! refusing before any exec, and failing loudly when nothing usable
//! remains.

#![expect(clippy::expect_used, reason = "in tests")]

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;

/// The built agentcept binary.
const BIN: &str = env!("CARGO_BIN_EXE_agentcept");

/// A scratch directory removed on drop, holding a `shim` dir with our
/// symlinks (early in `$PATH`, as in real use) and a `real` dir with
/// the marker binaries resolution must land on.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir()
            .join(format!("agentcept-exec-test-{tag}-{}", std::process::id()));
        fs::create_dir_all(root.join("shim")).expect("creating shim dir");
        fs::create_dir_all(root.join("real")).expect("creating real dir");
        Self(root)
    }

    /// Symlink `shim/<name>` to agentcept and write an executable
    /// `real/<name>` running `script`; returns the shim path.
    fn pair(&self, name: &str, script: &str) -> PathBuf {
        let shim = self.0.join("shim").join(name);
        drop(fs::remove_file(&shim));
        std::os::unix::fs::symlink(BIN, &shim).expect("symlinking the shim");

        let real = self.0.join("real").join(name);
        fs::write(&real, script).expect("writing the marker script");
        fs::set_permissions(&real, fs::Permissions::from_mode(0o755))
            .expect("chmod the marker script");
        shim
    }

    /// `$PATH` with our shim dir shadowing the marker dir.
    fn search(&self) -> String {
        format!(
            "{}:{}",
            self.0.join("shim").to_string_lossy(),
            self.0.join("real").to_string_lossy()
        )
    }

    /// `$PATH` with only our own shim in it.
    fn search_only_shim(&self) -> String {
        self.0.join("shim").to_string_lossy().into_owned()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        drop(fs::remove_dir_all(&self.0));
    }
}

/// Run the shim with `$PATH` set to `search`.
fn run(search: &str, shim: &Path, args: &[&str]) -> Output {
    Command::new(shim)
        .args(args)
        .env("PATH", search)
        .output()
        .expect("spawning the shim")
}

#[cfg(test)]
mod test {
    use std::process::Command;

    use super::Scratch;
    use super::run;

    #[test]
    fn skips_its_own_shim_and_execs_the_real_tool() {
        let scratch = Scratch::new("self-skip");
        let shim = scratch.pair("find", "#!/bin/sh\necho REAL-FIND \"$@\"\n");

        // The `find` in $PATH position one is agentcept itself; the shim
        // must skip it and exec the marker in position two.
        let output = run(&scratch.search(), &shim, &[".", "-name", "x"]);

        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "REAL-FIND . -name x\n"
        );
    }

    #[test]
    fn passes_the_real_tools_exit_status_through() {
        let scratch = Scratch::new("status");
        let shim = scratch.pair("grep", "#!/bin/sh\nexit 42\n");

        let output = run(&scratch.search(), &shim, &["-R", "hello", "."]);

        assert_eq!(output.status.code(), Some(42), "{output:?}");
    }

    #[test]
    fn passes_terminal_help_through_from_the_root_directory() {
        let scratch = Scratch::new("terminal");
        let shim = scratch.pair("find", "#!/bin/sh\necho REAL-FIND \"$@\"\n");

        // `--help` from `/`: the real find prints and exits; the cwd
        // guard must not fire, because no search happens.
        let output = Command::new(&shim)
            .arg("--help")
            .current_dir("/")
            .env("PATH", scratch.search())
            .output()
            .expect("spawning the shim");

        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "REAL-FIND --help\n"
        );
    }
    #[test]
    fn refuses_a_root_search_before_any_exec() {
        let scratch = Scratch::new("refusal");
        let shim = scratch.pair("find", "#!/bin/sh\necho REAL-FIND \"$@\"\n");

        let output = run(&scratch.search(), &shim, &["/", "-name", "x"]);

        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stdout).is_empty());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("refusing"), "{stderr}");
    }

    #[test]
    fn fails_loudly_when_only_itself_is_in_path() {
        let scratch = Scratch::new("exhausted");
        let shim = scratch.pair("find", "#!/bin/sh\necho REAL-FIND \"$@\"\n");

        let output = run(&scratch.search_only_shim(), &shim, &[".", "-name", "x"]);

        assert_eq!(output.status.code(), Some(127), "{output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("no usable"), "{stderr}");
    }
}
