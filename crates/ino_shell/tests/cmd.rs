#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::tests_outside_test_module,
    reason = "test code"
)]

mod common;

use std::ffi::OsStr;

use ino_shell::cmd;

use common::setup;

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
    {
        let output = cmd!(sh, "xecho {hello}").read().unwrap();
        assert_eq!(output, "hello");
    }

    // Whitespace inside braces is tolerated by the macro.
    {
        let output = cmd!(sh, "xecho { hello }").read().unwrap();
        assert_eq!(output, "hello");
    }
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
    let output = cmd!(sh, "xec{ho} hello").read().unwrap();
    assert_eq!(output, "hello");
}

#[test]
fn interpolation_move() {
    let sh = setup();

    let hello = "hello".to_owned();
    let output1 = cmd!(sh, "xecho {hello}").read().unwrap();
    let output2 = cmd!(sh, "xecho {hello}").read().unwrap();
    assert_eq!(output1, output2);
}

#[test]
fn interpolation_splat() {
    let sh = setup();

    // Splat of slice, empty slice, and owned strings.
    {
        let words = &["hello", "world"];
        let empty_words: &[&OsStr] = &[];
        let bang_words = &["!".to_owned()];
        let output =
            cmd!(sh, "xecho {words...} {empty_words...} {bang_words...}")
                .read()
                .unwrap();
        assert_eq!(output, "hello world !");
    }

    // Splat of Option.
    {
        let present = Some("hello");
        let absent: Option<&OsStr> = None;
        let output =
            cmd!(sh, "xecho {absent...} {present...}").read().unwrap();
        assert_eq!(output, "hello");
    }

    // Conditional splat idiom (Rust-side, but exercises the macro path).
    let check = if true { &["--", "--check"][..] } else { &[] };
    let dry_run = true.then_some("--dry-run");
    assert_eq!(
        cmd!(sh, "cargo fmt {check...}").to_string(),
        "cargo fmt -- --check",
    );
    assert_eq!(
        cmd!(sh, "cargo publish {dry_run...}").to_string(),
        "cargo publish --dry-run",
    );

    // Whitespace inside braces is tolerated.
    {
        let args = ["hello", "world"];
        let output = cmd!(sh, "xecho { args... }").read().unwrap();
        assert_eq!(output, "hello world");
    }
}

#[test]
fn exit_status() {
    let sh = setup();

    let err = cmd!(sh, "xecho -f").read().unwrap_err();
    assert_eq!(
        err.to_string(),
        "command exited with non-zero code `xecho -f`: 1
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
        "command was terminated by a signal `xecho -s`: 9
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
fn unknown_command() {
    let sh = setup();

    {
        let err = cmd!(sh, "nope no way").read().unwrap_err();
        assert_eq!(err.to_string(), "command not found: `nope`");
    }

    // `ignore_status` does not suppress command-not-found.
    {
        let err = cmd!(sh, "xecho-f").ignore_status().read().unwrap_err();
        assert_eq!(err.to_string(), "command not found: `xecho-f`");
    }
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
#[expect(
    clippy::non_ascii_literal,
    reason = "the command lexer must preserve UTF-8 token boundaries"
)]
fn unicode_words() {
    let sh = setup();
    let suffix = "世界";

    let output = cmd!(sh, "xecho café 'naïve' {suffix}").read().unwrap();

    assert_eq!(output, "café naïve 世界");
}

#[test]
fn escape() {
    let sh = setup();

    // Backslash and quote handling in the tokenizer.
    let output = cmd!(sh, "xecho \\hello\\ '\\world\\'").read().unwrap();
    assert_eq!(output, r"\hello\ \world\");

    // String-literal escapes preserved by `to_string()`.
    assert_eq!(cmd!(sh, "\"hello\"").to_string(), "\"hello\"");
    assert_eq!(cmd!(sh, "\"\"\"asdf\"\"\"").to_string(), r#""""asdf""""#);
    assert_eq!(cmd!(sh, "\\\\").to_string(), r"\\");
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
