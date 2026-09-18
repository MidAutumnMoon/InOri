//! Integration tests for PATH dispatch and self-shim detection.

#![expect(clippy::expect_used, reason = "in tests")]

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;

const BIN: &str = env!("CARGO_BIN_EXE_agentcept");

/// Test directory with `shim` and `real` PATH entries.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "agentcept-exec-test-{tag}-{}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("shim")).expect("creating shim dir");
        fs::create_dir_all(root.join("real")).expect("creating real dir");
        Self(root)
    }

    fn root(&self) -> &Path {
        &self.0
    }

    fn shim_path(&self, name: &str) -> PathBuf {
        self.0.join("shim").join(name)
    }

    fn pair(&self, name: &str, script: &str) -> PathBuf {
        let shim = self.shim_path(name);
        drop(fs::remove_file(&shim));
        std::os::unix::fs::symlink(BIN, &shim)
            .expect("symlinking the shim");
        self.executable("real", name, script);
        shim
    }

    fn executable(&self, dir: &str, name: &str, script: &str) -> PathBuf {
        let dir = self.0.join(dir);
        fs::create_dir_all(&dir).expect("creating the tool dir");
        let path = dir.join(name);
        fs::write(&path, script).expect("writing the marker script");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
            .expect("chmod the marker script");
        path
    }

    fn search(&self, dirs: &[&str]) -> String {
        dirs.iter()
            .map(|dir| self.0.join(dir).to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(":")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        drop(fs::remove_dir_all(&self.0));
    }
}

fn run(search: &str, shim: &Path, args: &[&str]) -> Output {
    Command::new(shim)
        .args(args)
        .env("PATH", search)
        .output()
        .expect("spawning the shim")
}

#[cfg(test)]
mod test {
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    use super::BIN;
    use super::Scratch;
    use super::run;

    #[test]
    fn skips_its_own_shim_and_execs_the_real_tool() {
        type Install = fn(&Path, &Path) -> std::io::Result<()>;

        let scratch = Scratch::new("self-skip");
        scratch.executable(
            "real",
            "find",
            "#!/bin/sh\necho REAL-FIND \"$@\"\n",
        );
        let shim = scratch.shim_path("find");

        // Hard links cannot cross filesystems, so install every variant from
        // a copy in the scratch directory.
        let myself = scratch.root().join("agentcept");
        fs::copy(BIN, &myself)
            .expect("copying the binary into the scratch");

        let installs: [(&str, Install); 3] = [
            ("symlink", |src, dst| std::os::unix::fs::symlink(src, dst)),
            ("hardlink", |src, dst| fs::hard_link(src, dst)),
            ("copy", |src, dst| fs::copy(src, dst).map(drop)),
        ];
        for (kind, install) in installs {
            drop(fs::remove_file(&shim));
            install(&myself, &shim).expect("installing the shim");

            let output = run(
                &scratch.search(&["shim", "real"]),
                &shim,
                &[".", "-name", "x"],
            );
            assert!(output.status.success(), "{kind}: {output:?}");
            assert_eq!(
                String::from_utf8_lossy(&output.stdout),
                "REAL-FIND . -name x\n",
                "{kind}"
            );
        }
    }

    #[test]
    fn skips_only_itself_not_the_first_candidate() {
        let scratch = Scratch::new("identity");
        scratch.executable(
            "early",
            "find",
            "#!/bin/sh\necho EARLY-FIND \"$@\"\n",
        );
        scratch.executable(
            "real",
            "find",
            "#!/bin/sh\necho REAL-FIND \"$@\"\n",
        );
        let shim = scratch.shim_path("find");
        std::os::unix::fs::symlink(BIN, &shim)
            .expect("symlinking the shim");

        let output = run(
            &scratch.search(&["early", "shim", "real"]),
            &shim,
            &[".", "-name", "x"],
        );

        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "EARLY-FIND . -name x\n"
        );
    }

    #[test]
    fn passes_the_real_tools_exit_status_through() {
        let scratch = Scratch::new("status");
        let shim = scratch.pair("grep", "#!/bin/sh\nexit 42\n");

        let output = run(
            &scratch.search(&["shim", "real"]),
            &shim,
            &["-R", "hello", "."],
        );

        assert_eq!(output.status.code(), Some(42), "{output:?}");
    }

    #[test]
    fn passes_terminal_help_through_from_the_root_directory() {
        let scratch = Scratch::new("terminal");
        let shim =
            scratch.pair("find", "#!/bin/sh\necho REAL-FIND \"$@\"\n");

        let output = Command::new(&shim)
            .arg("--help")
            .current_dir("/")
            .env("PATH", scratch.search(&["shim", "real"]))
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
        let shim =
            scratch.pair("find", "#!/bin/sh\necho REAL-FIND \"$@\"\n");

        let output = run(
            &scratch.search(&["shim", "real"]),
            &shim,
            &["/", "-name", "x"],
        );

        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stdout).is_empty());
        assert!(
            String::from_utf8_lossy(&output.stderr).starts_with("find:"),
            "{output:?}"
        );
    }

    #[test]
    fn refuses_a_recursive_grep_rooted_at_root() {
        let scratch = Scratch::new("grep-refusal");
        let shim =
            scratch.pair("grep", "#!/bin/sh\necho REAL-GREP \"$@\"\n");

        let output = run(
            &scratch.search(&["shim", "real"]),
            &shim,
            &["-R", "hello", "/"],
        );
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stdout).is_empty());
        assert!(
            String::from_utf8_lossy(&output.stderr).starts_with("grep:"),
            "{output:?}"
        );

        // A lone `/` is parsed as the pattern but usually means the pattern
        // was omitted from a root scan.
        let refusal =
            run(&scratch.search(&["shim", "real"]), &shim, &["-R", "/"]);
        assert_eq!(refusal.status.code(), Some(1), "{refusal:?}");
        assert!(String::from_utf8_lossy(&refusal.stdout).is_empty());
        assert!(
            String::from_utf8_lossy(&refusal.stderr).starts_with("grep:"),
            "{refusal:?}"
        );
    }

    #[test]
    fn fails_loudly_when_only_itself_is_in_path() {
        let scratch = Scratch::new("exhausted");
        let shim =
            scratch.pair("find", "#!/bin/sh\necho REAL-FIND \"$@\"\n");

        let output =
            run(&scratch.search(&["shim"]), &shim, &[".", "-name", "x"]);

        assert_eq!(output.status.code(), Some(127), "{output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("no usable"), "{stderr}");
    }

    #[test]
    fn dispatches_on_the_first_argument() {
        let scratch = Scratch::new("dispatcher");
        scratch.pair("find", "#!/bin/sh\necho REAL-FIND \"$@\"\n");

        let output = run(
            &scratch.search(&["shim", "real"]),
            Path::new(BIN),
            &["find", ".", "-name", "x"],
        );

        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "REAL-FIND . -name x\n"
        );
    }

    #[test]
    fn dispatcher_execs_explicit_paths_but_never_itself() {
        let scratch = Scratch::new("explicit-path");
        let marker = scratch.executable(
            "real",
            "find",
            "#!/bin/sh\necho REAL-FIND \"$@\"\n",
        );
        let shim = scratch.shim_path("find");
        std::os::unix::fs::symlink(BIN, &shim)
            .expect("symlinking the shim");

        // Explicit tool paths bypass `$PATH` lookup.
        let marker = marker.to_string_lossy().into_owned();
        let output = run(
            &scratch.search(&["shim"]),
            Path::new(BIN),
            &[&marker, ".", "-name", "x"],
        );
        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "REAL-FIND . -name x\n"
        );

        // Explicit paths must still reject agentcept itself.
        let shim = shim.to_string_lossy().into_owned();
        let refusal = run(
            &scratch.search(&["shim"]),
            Path::new(BIN),
            &[&shim, ".", "-name", "x"],
        );
        assert_eq!(refusal.status.code(), Some(127), "{refusal:?}");
        let stderr = String::from_utf8_lossy(&refusal.stderr);
        assert!(stderr.contains("agentcept itself"), "{stderr}");
    }

    #[test]
    fn prints_usage_and_fails_on_unknown_tools() {
        let scratch = Scratch::new("dispatcher-usage");
        let bin = Path::new(BIN);

        // No arguments: usage on stderr and failure.
        let output = run(&scratch.search(&["shim", "real"]), bin, &[]);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("Usage"), "{stderr}");

        // Help flags: usage on stdout and success.
        for flag in ["--help", "-h", "help"] {
            let flag_output =
                run(&scratch.search(&["shim", "real"]), bin, &[flag]);
            assert!(
                flag_output.status.success(),
                "{flag}: {flag_output:?}"
            );
            let stdout = String::from_utf8_lossy(&flag_output.stdout);
            assert!(stdout.contains("Usage"), "{flag}: {stdout}");
        }

        // Unknown tools must not fall through to `$PATH`.
        let unknown_output = run(
            &scratch.search(&["shim", "real"]),
            bin,
            &["frobnicate", "."],
        );
        assert_eq!(
            unknown_output.status.code(),
            Some(1),
            "{unknown_output:?}"
        );
        let unknown_stderr =
            String::from_utf8_lossy(&unknown_output.stderr);
        assert!(
            unknown_stderr.contains("unknown tool"),
            "{unknown_stderr}"
        );
    }

    #[test]
    fn wrong_multicall_names_error_loudly() {
        let scratch = Scratch::new("wrong-name");
        let shim = scratch.pair("ls", "#!/bin/sh\necho REAL-LS \"$@\"\n");

        // Do not fall through to the real `ls`.
        let output =
            run(&scratch.search(&["shim", "real"]), &shim, &["-la"]);

        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stdout).is_empty());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("not an intercepted tool"), "{stderr}");
    }
}
