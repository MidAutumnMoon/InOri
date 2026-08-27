# Lny

Manage your symlinks.

## Usage

```
Usage: lny [OPTIONS]

Options:
  -n, --new-blueprint <PATH>  Blueprint for symlinks to be created
  -o, --old-blueprint <PATH>  Previous generation of blueprint, symlinks in it will be removed
  -h, --help                  Print help
```

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
3. Both symlink paths must be absolute.
4. Destinations must not overlap: each must be unique, and no destination
   may be an ancestor of another destination in the same blueprint.

## Builtin Template Constants

- `{{ home }}`: user's home directory, e.g. `/home/tincan`
- `{{ config }}`: $XDG_CONFIG_HOME
- `{{ data }}`: $XDG_DATA_HOME
- `{{ cache }}`: $XDG_CACHE_HOME
- `{{ state }}`: $XDG_STATE_HOME

Guaranteed to be absolute if the app started successfully.
