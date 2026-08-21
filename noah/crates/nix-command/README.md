# nix-command

`nix-command` builds the Nix commands used by `nh`. It owns argv policy:
global flags precede the subcommand, `nix build` prints build logs by default,
and `nix repl` inherits interactive stdio. The `subprocess` crate owns process
execution, capture, streaming, pipelines, timeouts, and cleanup.

## Features

- Typed `CommandKind` values for the Nix subcommands used by `nh`.
- Builder methods for arguments, global arguments, environment variables,
  build-log policy, and timeouts.
- Deadlock-free concurrent stdout/stderr forwarding via `run_with_logs()`.
- Captured stdout/stderr via `output()`.
- Conversion to `subprocess::Exec` via `into_exec()` for pipelines and custom
  redirection.

## Quick start

```rust
use nix_command::{CommandKind, NixCommand};

let command = NixCommand::new(CommandKind::Build)
    .arg("--impure")
    .arg("nixpkgs#hello");

assert_eq!(command.argv(), [
    "nix",
    "build",
    "--print-build-logs",
    "--impure",
    "nixpkgs#hello",
]);

let status = command.run_with_logs()?;
assert!(status.success());

let output = NixCommand::new(CommandKind::Eval)
    .args(["--raw", "nixpkgs#hello.name"])
    .output()?;
assert!(output.success());
```

## Supported commands

| Command     | `--print-build-logs` | Interactive |
| ----------- | -------------------- | ----------- |
| `build`     | yes                  | no          |
| `copy`      | no                   | no          |
| `eval`      | no                   | no          |
| `flake`     | no                   | no          |
| `path-info` | no                   | no          |
| `repl`      | no                   | yes         |
