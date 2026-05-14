# AGENTS.md

## Project Overview

InOri is a Rust workspace containing CLI tools and shared utility crates, authored by MidAutumnMoon and licensed under GPL-3.0-or-later.

- **Repository**: <https://github.com/MidAutumnMoon/InOri>

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
| `coruma` | Comma replacement and symlink reverse-tracing |
| `derputils` | Miscellaneous utilities (QR code, clipboard, UUIDv7) |
| `imgo` | Image batch processing and transcoding |
| `lny` | Symlink manager driven by JSON blueprints with templates |
| `rpgdemake` | Batch decryption of RPG Maker MV/MZ encrypted assets |

### Shared Library Crates (`crates/`)

| Crate | Description |
|---|---|
| `ino_color` | Terminal coloring with type-level color/style selection |
| `ino_iter` | Iterator extension traits |
| `ino_path` | Path utilities (executable detection, etc.) |
| `ino_tap` | `tap` extension traits with `tracing` integration |
| `ino_tracing` | Opinionated `tracing-subscriber` initialization |

## Build & Test

```sh
cargo build --all-features
cargo test --all-features
cargo check --all-features
```

CI runs `cargo test --all-features` on every push and PR (see `.github/workflows/ci-tests.yml`).

## Coding Conventions

### Style & Formatting

- `rustfmt` and `.editorconfig` are configured — run `cargo fmt` before committing.

### Linting

- Strict Clippy lints are configured in the workspace `Cargo.toml`. Run `cargo clippy --all-features` and address all warnings before committing.

### Error Handling

- Use `anyhow` for application crates; `thiserror` for library crates that define custom error types.
- Prefer `context()` / `with_context()` to add meaningful error messages.
- Avoid `unwrap()` and `panic` in production code (Clippy will warn).

### Logging

- Use the `tracing` crate for all logging.
- Initialize the subscriber via `ino_tracing` at the start of `main()`.

### CLI Structure

- All CLI tools use `clap` with derive macros.

### Dependency Management

- Workspace-level dependencies are declared in the root `Cargo.toml` under `[workspace.dependencies]`.
- Crate-level `Cargo.toml` files reference them with `foo.workspace = true`.
- Renovate bot is configured for automated dependency updates.
- When looking for the Cargo registry directory, read from `$CARGO_HOME` (defaults to `~/.cargo` but may differ). Never hardcode `~/.cargo`.

## Verify Changes

After making changes:

1. **Format**: `cargo fmt --all`
2. **Lint**: `cargo clippy --all-features -- -D warnings`
3. **Test**: `cargo test --all-features`
4. Ensure the above all pass before considering the change complete.
