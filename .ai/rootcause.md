# rootcause usage notes

Gotchas and non-obvious behaviors from using [rootcause](https://crates.io/crates/rootcause) (v0.13.x) in this workspace. Not a tutorial.

## `Result` alias

- `rootcause::Result<T>` = `Result<T, Report<Dynamic>>`. The second generic is a **context type**, not the error type: `rootcause::Result<T, E>` does not mean "result with error `E`".
- Importing the alias shadows the std one, and trait-impl signatures silently change meaning: `fn from_str(..) -> Result<Self, Self::Err>` is resolved as `rootcause::Result<Self, C = Self::Err>` and fails with confusing type errors. Spell `std::result::Result<Self, Self::Err>` in trait impls (`FromStr`, `Deserialize`, ...).
- Bare `Report` = `Report<Dynamic, Mutable, SendSync>`.

## Concrete vs dynamic reports

- `.context()` / `.context_with()` return `Report<C, Mutable, SendSync>` with the **concrete** context type `C` (`&str`, `String`, `io::Error`, ...).
- `?` is the only implicit coercion from concrete to dynamic. A result that must *be* the function's error type — a tail expression or `return Err(..)` — needs an explicit `.into()`: `return Err(report!(err).context(msg).into())` keeps `err` as the cause. For a bare std error, `Report::from(err)` already produces the dynamic `Report`.
- Chaining `.context()` on a `Result<_, Report>` works.
- Any `std::error::Error + Send + Sync + 'static` converts with plain `?` (`io::Error`, `serde_json::Error`, `minijinja::Error`, etc.). Non-`Send`/`Sync` errors need the `local_*` variants (`local_context`, `local_into_report`).

## Macros

- There is **no `ensure!`**. Write `if !cond { bail!(..) }`.
- `bail!(..)` = `return Err(report!(..).into())`. Accepts literals, format args, or a bare error value (`bail!(io_err)`).
- `report!(err)` wraps a `std::error::Error` value (concrete context); `report!("fmt {}", x)` yields a dynamic report with the `Display` handler.

## Prelude

- The prelude ships `ResultExt`, `IteratorExt`, `report!`, `bail!` — but **not `OptionExt`**. To get `Option::context` (e.g. for etcetera's `state_dir() -> Option<PathBuf>`), import `rootcause::option_ext::OptionExt as _`.

## Output shape

- `fn main() -> rootcause::Result<()>` works out of the box: on error the report is printed to stderr with exit code 1.
- `Display` and `Debug` both print the **whole tree** — every context level and child on its own line, each annotated with the `file:line` where it was created. `Debug` additionally `{:?}`-formats the values.
- `Report` implements `Display`, so it fits wherever a `Display`able error is expected, e.g. serde's `de::Error::custom`.
