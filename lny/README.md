# Lny

Manage your symlinks.

## Usage

```
Usage: lny [-n=PATH] [-s=PATH]

Available options:
    -n, --new-blueprint=PATH  Blueprint for symlinks to be created
    -s, --state=PATH          State file recording the last applied blueprint, superseded on success
    -h, --help                Prints help information
    -V, --version             Prints version information
```

- No `--new-blueprint`: warn "nothing to do" and exit 0. A lone
  `--state` behaves the same and never touches the state file.
- `--new-blueprint` alone: apply against an empty old generation, so
  nothing is removed and every declared link is ensured.
- `--new-blueprint` + `--state`: stateful apply. Symlinks recorded in
  the state but absent from the new blueprint are removed; the rest of
  the recorded generation drives replace/collapse/expand transitions.

### State Semantics

The state file is itself a blueprint (see below) whose symlink paths
are the **literal rendered paths** of the last applied run, not
template text. When the environment changes (e.g. XDG variables), the
next run migrates the links to their new locations instead of
orphaning the old ones.

- A **missing** state file means first run: the old generation is
  empty.
- An **unreadable or invalid** state file (unparseable, unsupported
  version, not a regular file) is a hard error raised before any
  filesystem mutation. A damaged state is never treated as a first
  run.
- The state is rewritten atomically, only after every step succeeded.
- An interrupted run leaves the previous state in place. Re-running
  converges: already-applied steps (including replacements) are no-ops,
  and the state is recorded once a retry fully succeeds.
- Manual recovery from a lost state file: `cp` a known-good old
  blueprint to the state path. A symlinked state is rejected as not a
  regular file.

## Blueprint Shape

```json
{
  "version": 1,
  "symlinks": [
    { "src": "/src", "dst": "{{ home }}/dst" } 
  ]
}
```

1. Current `version` is **1**
2. Both symlink paths may contain [Minijinja](https://docs.rs/minijinja/latest/minijinja/) template markers
3. Both symlink paths must be absolute and must not contain `..` components.
4. Destinations must not overlap: each must be unique, and no destination
   may be an ancestor of another destination in the same blueprint.

## Builtin Template Constants

- `{{ home }}`: user's home directory, e.g. `/home/tincan`
- `{{ config }}`: $XDG_CONFIG_HOME
- `{{ data }}`: $XDG_DATA_HOME
- `{{ cache }}`: $XDG_CACHE_HOME
- `{{ state }}`: $XDG_STATE_HOME

Guaranteed to be absolute if the app started successfully.
