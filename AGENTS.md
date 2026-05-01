# AGENTS.md

## Project Overview

InOri is a Rust workspace containing CLI tools and shared utility crates, authored by MidAutumnMoon and licensed under GPL-3.0-or-later.

- **Repository**: <https://github.com/MidAutumnMoon/InOri>
- **Rust edition**: 2024
- **Minimum Rust version**: 1.90.0
- **Workspace resolver**: 2

## Communication

- Be short. Say the thing, stop.
- Don't repeat what I already said or what's already in context.
- Don't pad with disclaimers, summaries, or "hope that helps" type closings.
- If something is wrong, say what's wrong and how to fix it. Don't hedge.

## Look Things Up

- When unsure about a library, tool, or API, use web search or Context7 before guessing.
- Prefer Context7 for library docs — it pulls real examples and up-to-date signatures.
- Don't hallucinate option names, function signatures, or CLI flags. Look it up.

## Complex Tasks

- Break large tasks into sub-tasks. Tackle them in parallel with sub-agents when they don't depend on each other.
- Give each sub-agent full context — it won't see your conversation history.
- Keep sub-tasks scoped to one concern. If two sub-agents might edit the same file, don't run them in parallel.

## Workspace Layout

### Binary Crates

| Crate | Description |
|---|---|
| `coruma` | Comma replacement and symlink reverse-tracing (`coruma-reverse`) |
| `derputils` | Miscellaneous utilities (QR code generation, clipboard, UUIDv7) |
| `imgo` | Image batch processing and transcoding (AVIF, JXL, ImageMagick) |
| `lny` | Symlink manager driven by JSON blueprints with Minijinja templates |
| `rpgdemake` | Batch decryption of RPG Maker MV/MZ encrypted assets |

### Shared Library Crates (`crates/`)

| Crate | Description |
|---|---|
| `ino_color` | Terminal coloring with type-level color/style selection (16-color ANSI) |
| `ino_iter` | Iterator extension traits |
| `ino_path` | Path utilities (executable detection, etc.) built on `rustix` |
| `ino_tap` | `tap` extension traits with `tracing` integration |
| `ino_tracing` | Opinionated `tracing-subscriber` initialization |

## Build & Test

```sh
# Build the entire workspace
cargo build --all-features

# Run all tests
cargo test --all-features

# Check without building
cargo check --all-features
```

CI runs `cargo test --all-features` on every push and PR (see `.github/workflows/ci-tests.yml`).

## Coding Conventions

### Style & Formatting

- `rustfmt` is configured in `rustfmt.toml`: max width 75, no derive merging.
- `.editorconfig`: 4-space indent, trailing whitespace trimmed.
- Run `cargo fmt` before committing.

### Linting

The workspace enforces strict Clippy lints (pedantic + nursery), with the following allowances:

- `cognitive_complexity`, `literal_string_with_formatting_args`, `missing_const_for_fn`, `too_many_lines` — allowed
- `unwrap_used`, `panic`, `indexing_slicing`, `unreachable`, `undocumented_unsafe_blocks`, `unwrap_in_result` — warned

Run `cargo clippy --all-features` and address all warnings before committing.

### Error Handling

- Use `anyhow` for application crates; `thiserror` for library crates that define custom error types.
- Prefer `context()` / `with_context()` to add meaningful error messages.
- Avoid `unwrap()` and `panic` in production code (Clippy will warn).

### Logging

- Use the `tracing` crate for all logging.
- Initialize the subscriber via `ino_tracing::init_tracing_subscriber()` at the start of `main()`.
- Use `#[tracing::instrument]` on significant functions.

### CLI Structure

- All CLI tools use `clap` with derive macros.
- Define a `CliOpts` struct deriving `clap::Parser` with `#[arg(...)]` annotations.

### Dependency Management

- Workspace-level dependencies are declared in the root `Cargo.toml` under `[workspace.dependencies]`.
- Crate-level `Cargo.toml` files reference them with `foo.workspace = true`.
- Renovate bot is configured for automated dependency updates with auto-merge for minor/patch/digest changes.
- When looking for the Cargo registry directory, read from `$CARGO_HOME` (defaults to `~/.cargo` but may differ). Never hardcode `~/.cargo`.

## Verify Changes

After making changes:

1. **Format**: `cargo fmt --all`
2. **Lint**: `cargo clippy --all-features -- -D warnings`
3. **Test**: `cargo test --all-features`
4. Ensure the above all pass before considering the change complete.
