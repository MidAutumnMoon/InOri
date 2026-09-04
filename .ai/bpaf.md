# bpaf usage notes

Gotchas and non-obvious behaviors from using [bpaf](https://crates.io/crates/bpaf) (v0.9.x) in this workspace. Not a tutorial.

## Shell completion

- No public generation API. The hidden `--bpaf-complete-style-bash|zsh|fish|elvish` flag emits the script; the installed script calls back via `--bpaf-complete-rev=<n>` to produce candidates. Different flags, so scripts never regenerate themselves — regenerate only on bpaf major bumps.
- To wrap behind a subcommand (`derputils completion bash`), re-exec `current_exe()` with the flag and print the captured stdout (maintainer's recommendation, discussion #263). The flag is handled inside the parser before `run()`/tracing init, so the child writes only the script — `Command::output()` is safe.
- Multicall dispatchers need no extra wiring: each applet calls `cli().run_inner(Args::from(args).set_name(NAME))`; `set_name` also sets the program name in the generated script.

## `construct!`

- "Exactly one of" → array form `construct!([a, b, c])`; bpaf rejects both or neither. Struct/variant form takes variable shorthand only: `construct!(T { field })`, not `field: expr` (macro error: "no rules expected `:`"). See `qr.rs` / `completion.rs`.
- Array alternatives must all be `Parser<T>` for one `T`; map command outputs into the dispatch enum or build the variant directly (`construct!(CliOpts::Avif { transcoder, shared })`).
- Each branch runs on a cloned state (full backtracking). The more-consuming / left-most success wins; ties go to the first alternative. Because a failed alternative falls through to the next, a required argument inside a subcommand alternative can end up matching a later positional alternative — enforce such rules in a `.parse()` *after* the alternatives.
- Parser output values must be `Debug + Clone + 'static` throughout.

### `positional` must be last in `construct!`

Order decides who consumes what, not just how usage renders. A positional declared before a `--flag value` parser steals that pair's value. A named item after a positional- or command-bearing field compiles and parses fine but panics when usage is rendered: `bpaf usage BUG: all positional and command items must be placed in the right most position`. Call `.check_invariants(false)` in parser tests to catch it without rendering `--help`.

## Global flags (clap's `global = true`)

A named parser declared **before** a command in `construct!` evaluates first and scans the whole current scope, including past the command word. One declaration therefore accepts the flag before the subcommand, after it, and after nested subcommands — no per-subcommand copies.

- Never declare the same flag name at two levels: the outer parser silently steals the inner flag ("outer flag wins", documented in bpaf's `command.md`).
- A command word only matches at the head of the remaining items — `app hello packages` does not trigger a `packages` command.

## `env` + `switch` is presence-only

`long("ask").env("NH_ASK").switch()` treats any variable value (even `false`, `0`, empty) as "flag present" — it never parses the value. Real boolean env fallbacks need a `pure(()).parse(...)` step merged with the CLI switch (`cli || env`); noah used to do this for `NH_ASK` & co. and dropped it on purpose — boolean switches are CLI-only now, so don't reintroduce the pattern. On `argument`, `.env()` parses via `FromStr` as expected.

## Aliases

No alias API; chaining works: `long("no-gc").long("nogc")` — first name visible, the rest hidden. Same for `.short()`; multiple `.env()` fall back in order. Hidden aliases don't appear in help, completions, or error messages.

## `--` and passthrough

- Everything after `--` becomes strict positional words; flag/argument parsers never match them, so passthrough content can't be consumed as flags.
- `positional("EXTRA").strict().many()` ≈ clap's `last = true`. A positional that must not steal from the passthrough zone gets `.non_strict()`.

## Multi-value flags (`--flag NAME VALUE`)

`req_flag(())` anchor + `positional` items in a `construct!(a, b, c).adjacent()` group, repeated with `.many()`. An incomplete group fails with "expected `VALUE` ...".

## Subcommands

- The free `command(name, subparser)` fn is deprecated (0.9.27); use `subparser.command(name)`.
- The command's help line inherits the inner parser's `.descr()` — set it there once.

## `run_inner` vs `run`

- `run()` exits directly on help/error. Fine for simple single-command CLIs.
- `run_inner(args) -> Result<T, ParseFailure>` for tests, multicall dispatch, or custom error handling.

```rust
pub enum ParseFailure {
    Stdout(Doc, bool),   // --help / --version → stdout, exit 0
    Stderr(Doc),         // parse error → stderr, exit 1
    Completion(String),  // completion output → stdout, exit 0
}
```

`failure.print_message(max_width)` prints to the right stream; `failure.exit_code()` gives the exit code.

## Testing parsers

Use `run_inner` (never `run`, which exits). `options.check_invariants(false)` panics on ordering violations. `ParseFailure::unwrap_stderr()` returns the rendered error text for assertions. Unwraps in tests need `#[allow(clippy::unwrap_used)]` — workspace clippy flags them.

## Migrating from clap

- Doc comments become explicit calls: field docs → `.help("...")`; struct/enum doc → `.descr("...")` on `to_options()`. Short flags are written out; `--version` needs `.version(env!("CARGO_PKG_VERSION"))` (top-level only ≈ `propagate_version = false`).
- `#[arg(long, short, value_name = "PATH")]` → `long("name").short('n').argument::<T>("PATH").help("...").optional()`.
- `req_flag()` returns a plain `impl Parser<T>` — `.help()` and other `NamedArg` calls must come before it. `.switch()`/`.argument()` keep their own `.help()`.
- `ValueEnum` → `FromStr` with `type Err = String`, plus `Display` if you use `display_fallback` (which `PathBuf` isn't, so path fallbacks can't show their default).
- `last = true` → `positional().strict().many()`; `number_of_values = 2` → adjacent groups; `global = true` → single declaration before the command (all above).
- Parse failures exit 1 (clap used 2).
