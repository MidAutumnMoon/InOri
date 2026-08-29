# bpaf usage notes

Gotchas and non-obvious behaviors from using [bpaf](https://crates.io/crates/bpaf) (v0.9.x) in this workspace. Not a tutorial.

## Shell completion

### No public generation API — it's dynamic only

There's no `generate(shell)` like `clap_complete::aot::generate`. The generation functions are private; the only way to get a completion script is to run your own binary with a hidden flag:

```
your_program --bpaf-complete-style-bash   # or zsh, fish, elvish
```

The script is a thin stub that calls back into the binary at completion time to produce candidates. Content is stable across option changes, so you only regenerate on bpaf major-version bumps.

To wrap this behind a subcommand (e.g. `derputils completion bash`), re-exec `current_exe()` with `--bpaf-complete-style-<shell>` and print the captured stdout. This is the maintainer's recommended approach (discussion #263).

### Two hidden flags, no recursion

- `--bpaf-complete-style-<shell>` emits the completion script (one-time install).
- `--bpaf-complete-rev=<n>` is what the installed script calls at completion time to produce candidates.

Different flags, so generated scripts never regenerate themselves. An applet can generate its own completion without infinite recursion.

### The child process is clean

`--bpaf-complete-style-*` is handled inside the parser and returns the script before the applet's `run()` or tracing init runs. When you re-exec to capture the script, the child writes only the script to stdout and nothing to stderr. `Command::output()` is safe.

### Multicall dispatchers need no extra wiring

Each applet calls `cli().run_inner(Args::from(args).set_name(NAME))`. `set_name(NAME)` makes the style flags fire and sets the program name in the generated script. The dispatcher routes args to the applet, which handles completion on its own.

## `construct!` gotchas

For "exactly one of these flags" (e.g. `--clipboard` xor `--stdin`), use the array form `construct!([a, b, c])`. bpaf rejects both or neither on its own. The struct form `construct!(Struct { a, b })` requires field names to match the struct; see `qr.rs` / `completion.rs`.

## `run_inner` vs `run`

- `run()` panics or exits directly. Fine for simple single-command CLIs.
- `run_inner(args) -> Result<T, ParseFailure>` returns the parse result so you can handle errors yourself. The multicall dispatcher pattern needs this — `main` renders `ParseFailure` instead of bpaf exiting.

## `ParseFailure` variants

```rust
pub enum ParseFailure {
    Stdout(String),       // --help / --version → print to stdout, exit 0
    Stderr(String),       // parse error → print to stderr, exit code from exit_code()
    Completion(String),   // completion script/candidates → print to stdout, exit 0
}
```

`failure.print_message(max_width)` prints to the right stream; `failure.exit_code()` gives the exit code. The dispatcher's `main` calls both.

## Testing parsers

Use `run_inner` (not `run`, which exits). Match on `ParseFailure::Stderr` for parse errors. If you `.unwrap()` the success path, add `#[allow(clippy::unwrap_used)]` to the test module — the workspace's strict clippy flags it otherwise.

## Migrating from clap

- No derive. Doc comments become explicit calls: field docs → `.help("...")`; the struct/enum doc → `.descr("...")` on the `to_options()` builder.
- Short flags are not derived from field names — write `.short('n')` explicitly.
- `--version` is not automatic — add `.version(env!("CARGO_PKG_VERSION"))` on the `to_options()` builder.
- Clap's `#[arg(long, short, value_name = "PATH")]` maps to
  `long("name").short('n').argument::<T>("PATH").help("...").optional()`;
  attach `.help()` after `.argument()`.

