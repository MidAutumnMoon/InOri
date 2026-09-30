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
    run_with(search, shim, args, &[])
}

fn run_with(
    search: &str,
    shim: &Path,
    args: &[&str],
    env: &[(&str, &str)],
) -> Output {
    let mut command = Command::new(shim);
    command.args(args).env("PATH", search);
    for (key, value) in env {
        command.env(key, value);
    }
    command.output().expect("spawning the shim")
}

#[cfg(test)]
mod test {
    use std::fs;
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::Path;
    use std::path::PathBuf;
    use std::process::Command;
    use std::process::Output;

    use super::BIN;
    use super::Scratch;
    use super::run;
    use super::run_with;

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
    fn skips_other_agentcept_installs_in_path() {
        // Two agentcept installs in `$PATH` (an old build during a
        // rollout, say) must not exec each other in a loop; both are
        // skipped when looking for the real tool. Without the skip this
        // test hangs, each install exec'ing the other.
        let scratch = Scratch::new("two-installs");

        let old = scratch.root().join("old");
        fs::create_dir_all(&old).expect("creating the old install dir");
        fs::copy(BIN, old.join("agentcept"))
            .expect("installing the old build");
        std::os::unix::fs::symlink("agentcept", old.join("grep"))
            .expect("symlinking the old grep");

        let shim = scratch.shim_path("grep");
        std::os::unix::fs::symlink(BIN, &shim)
            .expect("symlinking the shim");
        scratch.executable(
            "real",
            "grep",
            "#!/bin/sh\necho REAL-GREP \"$@\"\n",
        );

        let output = run(
            &scratch.search(&["shim", "old", "real"]),
            &shim,
            &["-R", "hello", "."],
        );

        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "REAL-GREP -R hello .\n"
        );
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

    /// Marker reporting which engine served the call. A shebang script
    /// cannot observe `argv[0]` (the kernel drops it when dispatching to
    /// the interpreter), so the `arg0` wiring is covered by the real-ugrep
    /// canary below instead.
    const PINNED: &str = "#!/bin/sh\necho \"PINNED $0 $@\"\n";
    const REAL_GREP: &str = "#!/bin/sh\necho \"REAL-GREP $0 $@\"\n";

    fn pinned_grep(tag: &str) -> (Scratch, PathBuf, PathBuf) {
        let scratch = Scratch::new(tag);
        let pin = scratch.executable("tools", "ugrep", PINNED);
        let shim = scratch.shim_path("grep");
        std::os::unix::fs::symlink(BIN, &shim)
            .expect("symlinking the shim");
        scratch.executable("real", "grep", REAL_GREP);
        (scratch, pin, shim)
    }

    fn pinned_run(
        scratch: &Scratch,
        shim: &Path,
        pin: &Path,
        args: &[&str],
        env: &[(&str, &str)],
    ) -> Output {
        let pin = pin.to_string_lossy().into_owned();
        let mut env = env.to_vec();
        env.push(("AGENTCEPT_UGREP_PATH", &pin));
        run_with(&scratch.search(&["shim", "real"]), shim, args, &env)
    }

    #[test]
    fn prefers_the_pinned_ugrep_under_the_intercepted_name() {
        let (scratch, pin, shim) = pinned_grep("pin-name");
        let output =
            pinned_run(&scratch, &shim, &pin, &["-R", "hello", "."], &[]);

        assert!(output.status.success(), "{output:?}");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.starts_with("PINNED "), "{output:?}");
        assert!(stdout.contains(" -R hello ."), "{output:?}");
    }

    #[test]
    fn egrep_and_fgrep_run_the_pin_under_their_own_names() {
        for name in ["egrep", "fgrep"] {
            let scratch = Scratch::new("pin-dialects");
            let pin = scratch.executable("tools", "ugrep", PINNED);
            let shim = scratch.shim_path(name);
            std::os::unix::fs::symlink(BIN, &shim)
                .expect("symlinking the shim");
            scratch.executable("real", name, REAL_GREP);

            let output =
                pinned_run(&scratch, &shim, &pin, &["hello", "file"], &[]);

            assert!(output.status.success(), "{name}: {output:?}");
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(stdout.starts_with("PINNED "), "{name}: {output:?}");
            assert!(stdout.contains(" hello file"), "{name}: {output:?}");
        }
    }

    #[test]
    fn a_bare_pin_name_resolves_through_path() {
        let scratch = Scratch::new("pin-bare-name");
        scratch.executable("real", "ugrep", PINNED);
        let shim = scratch.shim_path("grep");
        std::os::unix::fs::symlink(BIN, &shim)
            .expect("symlinking the shim");
        scratch.executable("real", "grep", REAL_GREP);

        let output = run_with(
            &scratch.search(&["shim", "real", "tools"]),
            &shim,
            &["hello", "file"],
            &[("AGENTCEPT_UGREP_PATH", "ugrep")],
        );

        assert!(output.status.success(), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stdout).starts_with("PINNED "),
            "{output:?}"
        );
    }

    /// Pins a real ugrep when one is installed, to check the `argv[0]`
    /// wiring: only a ugrep running as `grep` serves the BRE dialect.
    fn real_ugrep() -> Option<PathBuf> {
        let search = std::env::var_os("PATH")?;
        search
            .to_string_lossy()
            .split(':')
            .map(|dir| PathBuf::from(dir).join("ugrep"))
            .find(|candidate| {
                candidate.is_file()
                    && fs::metadata(candidate).is_ok_and(|meta| {
                        meta.permissions().mode() & 0o111 != 0
                    })
            })
    }

    #[test]
    fn the_pin_runs_ugrep_under_the_grep_name() {
        let Some(pin) = real_ugrep() else {
            // No ugrep on this machine: the marker tests above still
            // cover the routing.
            return;
        };
        let scratch = Scratch::new("pin-real-ugrep");
        let fixture = scratch.root().join("bre.txt");
        fs::write(&fixture, "aaa\nb\n").expect("writing the fixture");
        let fixture = fixture.to_string_lossy().into_owned();
        let shim = scratch.shim_path("grep");
        std::os::unix::fs::symlink(BIN, &shim)
            .expect("symlinking the shim");

        // BRE reads `a+` as literal text (no match); ugrep's native ERE
        // would match "aaa". Only a ugrep running as `grep` serves BRE.
        let output =
            pinned_run(&scratch, &shim, &pin, &["a+", &fixture], &[]);

        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "",
            "{output:?}"
        );
    }

    #[test]
    fn falls_back_when_the_pin_is_missing_or_not_executable() {
        let (scratch, _pin, shim) = pinned_grep("pin-unusable");
        let missing =
            scratch.root().join("tools").join("definitely-absent");
        let non_executable = scratch.root().join("tools").join("data");
        fs::write(&non_executable, "#!/bin/sh\n").expect("writing data");

        for unusable in [&missing, &non_executable] {
            let output = pinned_run(
                &scratch,
                &shim,
                unusable,
                &["-R", "hello", "."],
                &[],
            );

            assert!(output.status.success(), "{unusable:?}: {output:?}");
            assert!(
                String::from_utf8_lossy(&output.stdout)
                    .starts_with("REAL-GREP"),
                "{unusable:?}"
            );
        }
    }

    #[test]
    fn falls_back_when_exec_of_the_pin_fails() {
        let (scratch, pin, shim) = pinned_grep("pin-exec-failure");
        // Usable (file + executable) but its interpreter does not exist.
        fs::write(&pin, "#!/nonexistent/shebang\n")
            .expect("rewriting the pin");

        let output =
            pinned_run(&scratch, &shim, &pin, &["-R", "hello", "."], &[]);

        assert!(output.status.success(), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stdout)
                .starts_with("REAL-GREP"),
            "{output:?}"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("using the real"), "{output:?}");
    }

    #[test]
    fn skips_itself_when_pinned_as_the_replacement() {
        let (scratch, _pin, shim) = pinned_grep("pin-is-self");
        let output = pinned_run(
            &scratch,
            &shim,
            Path::new(BIN),
            &["-R", "hello", "."],
            &[],
        );

        assert!(output.status.success(), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stdout)
                .starts_with("REAL-GREP"),
            "{output:?}"
        );
    }

    #[test]
    fn tools_gnu_disables_the_replacement() {
        let (scratch, pin, shim) = pinned_grep("pin-opted-out");
        let output = pinned_run(
            &scratch,
            &shim,
            &pin,
            &["-R", "hello", "."],
            &[("AGENTCEPT_TOOLS", "gnu")],
        );

        assert!(output.status.success(), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stdout)
                .starts_with("REAL-GREP"),
            "{output:?}"
        );
    }

    #[test]
    fn tools_values_other_than_gnu_keep_the_replacement() {
        let (scratch, pin, shim) = pinned_grep("pin-opted-in");
        let output = pinned_run(
            &scratch,
            &shim,
            &pin,
            &["-R", "hello", "."],
            &[("AGENTCEPT_TOOLS", "auto")],
        );

        assert!(output.status.success(), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stdout).starts_with("PINNED "),
            "{output:?}"
        );
    }

    #[test]
    fn argv_gated_calls_run_the_real_tool() {
        let (scratch, pin, shim) = pinned_grep("pin-gated");
        let dir = scratch.root().join("subdir");
        fs::create_dir_all(&dir).expect("creating a directory operand");
        let dir = dir.to_string_lossy().into_owned();

        let gated: [&[&str]; 4] = [
            &["--version"],
            &["pattern", &dir],
            &["-y", "pattern", "file"],
            // GNU reads stdin alone; ugrep would also scan the tree.
            &["-r", "pattern", "-"],
        ];
        for args in gated {
            let output = pinned_run(&scratch, &shim, &pin, args, &[]);

            assert!(output.status.success(), "{args:?}: {output:?}");
            assert!(
                String::from_utf8_lossy(&output.stdout)
                    .starts_with("REAL-GREP"),
                "{args:?}"
            );
        }

        // Recursive searches of directories stay on the fast path.
        let fast = pinned_run(
            &scratch,
            &shim,
            &pin,
            &["-r", "pattern", &dir],
            &[],
        );
        assert!(
            String::from_utf8_lossy(&fast.stdout).starts_with("PINNED "),
            "{fast:?}"
        );
    }

    #[test]
    fn refusals_win_over_the_replacement() {
        let (scratch, pin, shim) = pinned_grep("pin-refused");
        let output =
            pinned_run(&scratch, &shim, &pin, &["-R", "hello", "/"], &[]);

        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stdout).is_empty());
        assert!(
            String::from_utf8_lossy(&output.stderr).starts_with("grep:"),
            "{output:?}"
        );
    }

    #[test]
    fn find_is_never_replaced() {
        let scratch = Scratch::new("pin-find-unaffected");
        let pin = scratch.executable("tools", "ugrep", PINNED);
        let shim = scratch.shim_path("find");
        std::os::unix::fs::symlink(BIN, &shim)
            .expect("symlinking the shim");
        scratch.executable(
            "real",
            "find",
            "#!/bin/sh\necho REAL-FIND \"$@\"\n",
        );

        let output =
            pinned_run(&scratch, &shim, &pin, &[".", "-name", "x"], &[]);

        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "REAL-FIND . -name x\n"
        );
    }
}
