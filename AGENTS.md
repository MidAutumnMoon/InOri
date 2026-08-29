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
- Use `$CARGO_HOME` instead of `~/.cargo`.

## No `| tail` / `| head`

Do not pipe any command output through `head` or `tail`, tools will properly handle large output natively.

## Workspace Layout

### Binary Crates

| Crate | Description |
|---|---|
| `derputils` | Miscellaneous utilities |
| `imgo` | Image batch processing and transcoding |
| `lny` | Symlink manager driven by JSON blueprints with templates |
| `nh` (`noah/`) | Nix CLI helper/wrapper |
| `rpgdemake` | Batch decryption of RPG Maker MV/MZ encrypted assets |

### Shared Library Crates (`crates/`)

| Crate | Description |
|---|---|
| `ino_color` | Terminal coloring with type-level color/style selection |
| `ino_iter` | Iterator extension traits |
| `ino_path` | Path utilities (executable detection, etc.) |
| `ino_shell` | Shell-style command execution for scripting (macros in `ino_shell-macros`) |
| `ino_tracing` | Opinionated `tracing-subscriber` initialization |

## Fitting New Code

- Recover the domain truth before choosing a shape: valid states, natural owner, boundaries, and relevant failure or resource constraints.
- Give each fact one authoritative home. Prefer representations and APIs that enforce invariants over comments or caller discipline.
- Read the surrounding code first. Extend an abstraction only when the new behavior shares its policy and ownership; otherwise reshape proportionally instead of adding special cases, parallel paths, or copied logic.
- Keep dependencies and data flow explicit. Do not recover required context from ambient state or reconstruct information discarded earlier.
- Design for real pressures, not hypothetical reuse. Preserve existing behavior unless a change is requested; consolidate experiments before landing.

## Coding Conventions

### Linting

- Strict Clippy lints are configured in the workspace `Cargo.toml`. Run `cargo clippy --all-features` and address all warnings before committing.

### Error Handling

- Use [`rootcause`](https://docs.rs/rootcause/) for application, and [`thiserror`](https://docs.rs/thiserror/) for library. For past experiences and gotchas, see [`.ai/rootcause.md`](./.ai/rootcause.md).
- Prefer richer return types instead of convention.
- Avoid `unwrap()` and `panic` in production code (Clippy will warn).

### Logging

- Use the `tracing` crate for all logging.
- Initialize the subscriber via `ino_tracing` at the start of `main()`.

### CLI Structure

- New apps should use `bpaf` CLI parser, `clap` is legacy and is gradually being replaced.
- For experience and gotchas on using bpaf, see [`.ai/bpaf.md`](./.ai/bpaf.md).

### Dependency Management

- Workspace-level dependencies are declared in the root `Cargo.toml` under `[workspace.dependencies]`.
- Crate-level `Cargo.toml` files reference them with `foo.workspace = true`.
- Renovate bot is configured for automated dependency updates.

## Verify Changes

After making changes (scope to the relevant app/crate):

1. **Lint**: `cargo clippy --all-features --all-targets --package <crate>`
2. **Test**: `cargo test --all-features --package <crate>`
3. Ensure the above all pass before considering the change complete.

Do not pipe command output through `head` or `tail`, especially `cargo test`.
