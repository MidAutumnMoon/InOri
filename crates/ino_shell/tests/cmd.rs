#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test code"
)]

mod common;

use std::ffi::OsStr;

use ino_shell::cmd;

use common::setup;

#[test]
fn smoke() {
    let sh = setup();

    let pwd = "lol";
    let cmd = cmd!(sh, "xecho 'hello '{pwd}");
    println!("{cmd}");
}

#[test]
fn into_command() {
    let sh = setup();
    let cmd = cmd!(sh, "git branch");
    let _ = cmd.to_command();
    let _: std::process::Command = cmd.into();
}

#[test]
fn multiline() {
    let sh = setup();

    let output = cmd!(
        sh,
        "
        xecho hello
        "
    )
    .read()
    .unwrap();
    assert_eq!(output, "hello");
}

#[test]
fn interpolation() {
    let sh = setup();

    let hello = "hello";
    let output = cmd!(sh, "xecho {hello}").read().unwrap();
    assert_eq!(output, "hello");
}

#[test]
fn program_interpolation() {
    let sh = setup();

    let echo = "xecho";
    let output = cmd!(sh, "{echo} hello").read().unwrap();
    assert_eq!(output, "hello");
}

#[test]
fn interpolation_concatenation() {
    let sh = setup();

    let hello = "hello";
    let world = "world";
    let output = cmd!(sh, "xecho {hello}-{world}").read().unwrap();
    assert_eq!(output, "hello-world");
}

#[test]
fn program_concatenation() {
    let sh = setup();

    let ho = "ho";
    let output = dbg!(cmd!(sh, "xec{ho} hello")).read().unwrap();
    assert_eq!(output, "hello");
}

#[test]
fn interpolation_move() {
    let sh = setup();

    let hello = "hello".to_string();
    let output1 = cmd!(sh, "xecho {hello}").read().unwrap();
    let output2 = cmd!(sh, "xecho {hello}").read().unwrap();
    assert_eq!(output1, output2);
}

#[test]
fn interpolation_spat() {
    let sh = setup();

    let a = &["hello", "world"];
    let b: &[&OsStr] = &[];
    let c = &["!".to_string()];
    let output = cmd!(sh, "xecho {a...} {b...} {c...}").read().unwrap();
    assert_eq!(output, "hello world !");
}

#[test]
fn splat_option() {
    let sh = setup();

    let a: Option<&OsStr> = None;
    let b = Some("hello");
    let output = cmd!(sh, "xecho {a...} {b...}").read().unwrap();
    assert_eq!(output, "hello");
}

#[test]
fn splat_idiom() {
    let sh = setup();

    let check = if true { &["--", "--check"][..] } else { &[] };
    let cmd = cmd!(sh, "cargo fmt {check...}");
    assert_eq!(cmd.to_string(), "cargo fmt -- --check");

    let dry_run = if true { Some("--dry-run") } else { None };
    let cmd = cmd!(sh, "cargo publish {dry_run...}");
    assert_eq!(cmd.to_string(), "cargo publish --dry-run");
}

#[test]
fn interpolation_tolerates_whitespace() {
    let sh = setup();

    let hello = "hello";
    let output = cmd!(sh, "xecho { hello }").read().unwrap();
    assert_eq!(output, "hello");
}

#[test]
fn splat_tolerates_whitespace() {
    let sh = setup();

    let args = ["hello", "world"];
    let output = cmd!(sh, "xecho { args... }").read().unwrap();
    assert_eq!(output, "hello world");
}

#[test]
fn exit_status() {
    let sh = setup();

    let err = cmd!(sh, "xecho -f").read().unwrap_err();
    assert_eq!(
        err.to_string(),
        r"command exited with non-zero code `xecho -f`: 1
stdout suffix:


stderr suffix:
other error

"
    );
}

#[test]
#[cfg_attr(not(unix), ignore)]
fn exit_status_signal() {
    let sh = setup();

    let err = cmd!(sh, "xecho -s").read().unwrap_err();
    assert_eq!(
        err.to_string(),
        r"command was terminated by a signal `xecho -s`: 9
stdout suffix:


"
    );
}

#[test]
fn ignore_status() {
    let sh = setup();

    let output = cmd!(sh, "xecho -f").ignore_status().read().unwrap();
    assert_eq!(output, "");
}

#[test]
fn ignore_status_no_such_command() {
    let sh = setup();

    let err = cmd!(sh, "xecho-f").ignore_status().read().unwrap_err();
    assert_eq!(err.to_string(), "command not found: `xecho-f`");
}

#[test]
#[cfg_attr(not(unix), ignore)]
fn ignore_status_signal() {
    let sh = setup();

    let output = cmd!(sh, "xecho -s dead").ignore_status().read().unwrap();
    assert_eq!(output, "dead");
}

#[test]
fn read_stderr() {
    let sh = setup();

    let output = cmd!(sh, "xecho -f -e snafu")
        .ignore_status()
        .read_stderr()
        .unwrap();
    assert!(output.contains("snafu"));
}

#[test]
fn unknown_command() {
    let sh = setup();

    let err = cmd!(sh, "nope no way").read().unwrap_err();
    assert_eq!(err.to_string(), "command not found: `nope`");
}

#[test]
fn args_with_spaces() {
    let sh = setup();

    let hello_world = "hello world";
    let cmd = cmd!(sh, "xecho {hello_world} 'hello world' hello world");
    assert_eq!(
        cmd.to_string(),
        r#"xecho "hello world" "hello world" hello world"#
    );
}

#[test]
fn escape() {
    let sh = setup();

    let output = cmd!(sh, "xecho \\hello\\ '\\world\\'").read().unwrap();
    assert_eq!(output, r"\hello\ \world\");
}

#[test]
fn stdin_redirection() {
    let sh = setup();

    let lines = "\
foo
baz
bar
";
    let output = cmd!(sh, "xecho -i")
        .stdin(lines)
        .read()
        .unwrap()
        .replace("\r\n", "\n");
    assert_eq!(
        output,
        "\
foo
baz
bar
"
    );
}

#[test]
fn no_deadlock() {
    let sh = setup();

    let mut data = "All the work and now paly made Jack a dull boy.\n"
        .repeat(1 << 20);
    data.pop();
    let res = cmd!(sh, "xecho -i").stdin(&data).read().unwrap();
    assert_eq!(data, res);
}

#[test]
fn string_escapes() {
    let sh = setup();

    assert_eq!(cmd!(sh, "\"hello\"").to_string(), "\"hello\"");
    assert_eq!(cmd!(sh, "\"\"\"asdf\"\"\"").to_string(), r#""""asdf""""#);
    assert_eq!(cmd!(sh, "\\\\").to_string(), r"\\");
}

#[test]
fn nonexistent_current_directory() {
    let mut sh = setup();
    sh.set_current_dir("nonexistent");
    let err = cmd!(sh, "ls").run().unwrap_err();
    let message = err.to_string();
    if cfg!(unix) {
        assert!(message.contains("nonexistent"), "{message}");
        assert!(message.starts_with("failed to get current directory"));
        assert!(
            message.ends_with("No such file or directory (os error 2)")
        );
    } else {
        assert_eq!(
            message,
            "io error when running command `ls`: The directory name is invalid. (os error 267)"
        );
    }
}
