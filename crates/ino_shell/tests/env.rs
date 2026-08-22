#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::needless_pass_by_value,
    reason = "test code"
)]

mod common;

use std::collections::BTreeMap;

use ino_shell::cmd;

use common::setup;

const VAR: &str = "SPICA";

#[test]
fn subshells_env() {
    let sh = setup();

    let e1 = sh.var_os(VAR);
    {
        let mut sh = sh.clone();
        sh.set_var(VAR, "1");
        let e2 = sh.var_os(VAR);
        assert_eq!(e2.as_deref(), Some("1".as_ref()));
        {
            let mut sh = sh.clone();
            sh.set_var(VAR, "2");
            let e3 = sh.var_os(VAR);
            assert_eq!(e3.as_deref(), Some("2".as_ref()));
        }
        let e4 = sh.var_os(VAR);
        assert_eq!(e4, e2);
    }
    let e5 = sh.var_os(VAR);
    assert_eq!(e5, e1);
}

#[test]
fn env() {
    let mut sh = setup();

    let v1 = "xshell_test_123";
    let v2 = "xshell_test_456";

    let cloned_sh = sh.clone();

    for sh in [&sh, &cloned_sh] {
        assert_env(
            cmd!(sh, "xecho -$ {v1}").env(v1, "123"),
            &[(v1, Some("123"))],
        );

        assert_env(
            cmd!(sh, "xecho -$ {v1} {v2}")
                .envs([(v1, "123"), (v2, "456")].iter().copied()),
            &[(v1, Some("123")), (v2, Some("456"))],
        );
        assert_env(
            cmd!(sh, "xecho -$ {v1} {v2}")
                .envs([(v1, "123"), (v2, "456")].iter().copied())
                .env_remove(v2),
            &[(v1, Some("123")), (v2, None)],
        );
        assert_env(
            cmd!(sh, "xecho -$ {v1} {v2}")
                .envs([(v1, "123"), (v2, "456")].iter().copied())
                .env_remove("nothing"),
            &[(v1, Some("123")), (v2, Some("456"))],
        );
    }

    sh.set_var(v1, "foobar");
    sh.set_var(v2, "quark");

    assert_env(
        cmd!(sh, "xecho -$ {v1} {v2}"),
        &[(v1, Some("foobar")), (v2, Some("quark"))],
    );
    assert_env(
        cmd!(cloned_sh, "xecho -$ {v1} {v2}"),
        &[(v1, None), (v2, None)],
    );

    assert_env(
        cmd!(sh, "xecho -$ {v1} {v2}").env(v1, "wombo"),
        &[(v1, Some("wombo")), (v2, Some("quark"))],
    );

    assert_env(
        cmd!(sh, "xecho -$ {v1} {v2}").env_remove(v1),
        &[(v1, None), (v2, Some("quark"))],
    );
    assert_env(
        cmd!(sh, "xecho -$ {v1} {v2}").env_remove(v1).env(v1, "baz"),
        &[(v1, Some("baz")), (v2, Some("quark"))],
    );
    assert_env(
        cmd!(sh, "xecho -$ {v1} {v2}").env(v1, "baz").env_remove(v1),
        &[(v1, None), (v2, Some("quark"))],
    );
}

#[test]
fn env_clear() {
    let mut sh = setup();

    let v1 = "xshell_test_123";
    let v2 = "xshell_test_456";

    // `env_clear()` wipes the ambient environment including `PATH`, so
    // `xecho` can't be resolved via `$PATH`. Invoke it by absolute path.
    let xecho = std::env::var("CARGO_BIN_EXE_xecho")
        .expect("CARGO_BIN_EXE_xecho must be set by the test harness");

    assert_env(
        cmd!(sh, "{xecho} -$ {v1} {v2}")
            .envs([(v1, "123"), (v2, "456")].iter().copied())
            .env_clear(),
        &[(v1, None), (v2, None)],
    );
    assert_env(
        cmd!(sh, "{xecho} -$ {v1} {v2}")
            .envs([(v1, "123"), (v2, "456")].iter().copied())
            .env_clear()
            .env(v1, "789"),
        &[(v1, Some("789")), (v2, None)],
    );

    sh.set_var(v1, "foobar");
    sh.set_var(v2, "quark");

    assert_env(
        cmd!(sh, "{xecho} -$ {v1} {v2}").env_clear(),
        &[(v1, None), (v2, None)],
    );
    assert_env(
        cmd!(sh, "{xecho} -$ {v1} {v2}").env_clear().env(v1, "baz"),
        &[(v1, Some("baz")), (v2, None)],
    );
    assert_env(
        cmd!(sh, "{xecho} -$ {v1} {v2}").env(v1, "baz").env_clear(),
        &[(v1, None), (v2, None)],
    );
}

#[track_caller]
fn assert_env(
    xecho_env_cmd: ino_shell::Cmd,
    want_env: &[(&str, Option<&str>)],
) {
    let output = xecho_env_cmd.output().unwrap();
    let env = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let (key, val) = line.split_once('=').unwrap_or_else(|| {
                panic!(
                    "failed to parse line from `xecho -$` output: {line:?}"
                )
            });
            (key.to_owned(), val.to_owned())
        })
        .collect::<BTreeMap<_, _>>();
    check_env(&env, want_env);
}

#[track_caller]
fn check_env(
    env: &BTreeMap<String, String>,
    wanted_env: &[(&str, Option<&str>)],
) {
    let mut failed = false;
    let mut seen = env.clone();
    for &(k, val) in wanted_env {
        match (seen.remove(k), val) {
            (Some(env_v), Some(want_v)) if env_v == want_v => {}
            (None, None) => {}
            (have, want) => {
                eprintln!(
                    "mismatch on env var {k:?}: have `{have:?}`, want `{want:?}` "
                );
                failed = true;
            }
        }
    }
    for (k, v) in seen {
        eprintln!("Unexpected env key {k:?} (value: {v:?})");
        failed = true;
    }
    assert!(
        !failed,
        "env didn't match (see stderr for cleaner output):\nsaw: {env:?}\n\nwanted: {wanted_env:?}",
    );
}
