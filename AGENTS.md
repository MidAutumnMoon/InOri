# AGENTS.md

## Project Overview

InOri is a Rust workspace monorepo containing CLI tools and shared utility crates, authored by MidAutumnMoon and licensed under GPL-3.0-or-later.

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

## Workspace Layout

### Binary Crates

| Crate | Description |
|---|---|
| `coruma` | Comma replacement and symlink reverse-tracing |
| `derputils` | Miscellaneous utilities |
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

## Coding Conventions

### Linting

- Strict Clippy lints are configured in the workspace `Cargo.toml`. Run `cargo clippy --all-features` and address all warnings before committing.

### Error Handling

- New apps should use `rootcause` crate instead of `anyhow`. Usage of `anyhow` is legacy and is gradually being replaced.
- Prefer richer return types instead of convention.
- Avoid `unwrap()` and `panic` in production code (Clippy will warn).

### Logging

- Use the `tracing` crate for all logging.
- Initialize the subscriber via `ino_tracing` at the start of `main()`.

### CLI Structure

- New apps should use `bpaf` CLI parser, `clap` is legacy and is gradually being replaced.

### Dependency Management

- Workspace-level dependencies are declared in the root `Cargo.toml` under `[workspace.dependencies]`.
- Crate-level `Cargo.toml` files reference them with `foo.workspace = true`.
- Renovate bot is configured for automated dependency updates.

## Verify Changes

After making changes (scope to the relevant app/crate):

1. **Lint**: `cargo clippy --all-features --all-targets --package <crate>`
2. **Test**: `cargo test --all-features --package <crate>`
3. Ensure the above all pass before considering the change complete.
