# InOri

InOri is a Rust workspace monorepo containing CLI tools and shared utility crates, authored by MidAutumnMoon and licensed under GPL-3.0-or-later.

- Repository: <https://github.com/MidAutumnMoon/InOri>

## Make changes

- Make systematic changes. Don't be scoped to the spot you are looking at.
- Make breaking changes. Trace, find, redesign, from top to bottom.
- Challenge existing designs.

## Workspace Layout

### Binary Crates

| Crate          | Description                                              |
| -------------- | -------------------------------------------------------- |
| `derputils`    | Miscellaneous utilities                                  |
| `imgo`         | Image batch processing and transcoding                   |
| `lny`          | Symlink manager driven by JSON blueprints with templates |
| `rpgdemake`    | Batch decryption of RPG Maker MV/MZ encrypted assets     |

### Shared Library Crates (`crates/`)

| Crate         | Description                                                                |
| ------------- | -------------------------------------------------------------------------- |
| `ino_color`   | Terminal coloring with type-level color/style selection                    |
| `ino_iter`    | Iterator extension traits                                                  |
| `ino_path`    | Path utilities (executable detection, etc.)                                |
| `ino_tracing` | Opinionated `tracing-subscriber` initialization                            |

## Repo conventions

- This is a Rust cargo workspace.
- Use [`rootcause`](https://docs.rs/rootcause/) for application, and [`thiserror`](https://docs.rs/thiserror/) for library. For past experiences and gotchas, see [`.ai/rootcause.md`](./.ai/rootcause.md). Avoid `unwrap()` and `panic` outside tests.
- Use [`indoc`](https://docs.rs/indoc/) for multi-line strings.
- Use `tracing` for logging.
- Use `bpaf` to handle CLI parsing. For experience and gotchas on using bpaf, see [`.ai/bpaf.md`](./.ai/bpaf.md).

## VCS

Use `jj` vcs exclusively. For example, `jj status`, `jj new`.

External pager is configured. Use `jj diff --git` and `jj show --git` for traditional git style diff output.

Don't commit on behalf of the user. Don't write commit message

## Verify Changes

After making changes (scope to the relevant app/crate):

1. **Lint**: `cargo clippy --all-features --all-targets --package <crate>`. MUST NOT suppress warning without permission. Skip and report warnings that might signal architectural problems. Fix other warnings properly instead of taking clippy suggestions as-is.
2. **Test**: `cargo test --all-features --package <crate>`

Do not pipe command output through `head` or `tail`, especially `cargo test`.
