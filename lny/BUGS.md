# lny — bugs, footguns, and safety concerns

Reviewed against `lny/src/{main,blueprint,step,template}.rs`.

## Real bugs

### 1. `replace_symlink` doesn't create parent dirs, and the dry run hides it — FIXED
`lny/src/step.rs:233-313`

The comment at line 262-263 says:
> If dst does not exist, replace essentially becomes create with extra steps

But it doesn't actually copy `create_symlink`'s behavior. If `dst`'s parent dir was also removed (entirely possible — the old symlink is the only thing that kept it alive, or the user manages multiple tools), `symlink(new_src, &tmp_dst)` fails with ENOENT.

Worse, the dry pass returns `Ok(())` at line 269-272 *before* this would happen, so the user gets a false "all clear" and then the real pass fails partway through the queue — leaving the filesystem in a half-applied state.

Fix: call `create_parent_dirs` in the replace path too, and move the dry-run early return to *after* feasibility checks (or make dry run actually check parent existence).

### 2. `dry_execute` only catches collisions, not feasibility — FIXED (Option B)
`lny/src/step.rs:158-160`, `200-210`, `269-272`, `329-332`

The two-pass design in `main.rs` (`dry_execute` all, then `execute` all) is supposed to be the safety net, but every step's dry branch early-returns after the collision check. None of these will be caught by dry run:
- parent dir missing (replace case above)
- permission denied on the actual `symlink(2)` / `rename(2)` / `remove_file`
- read-only filesystem
- dst path component is a file (e.g. `/etc/hosts/foo`)

Either rename `dry_execute` to `check_collisions` to be honest about what it does, or make it actually walk the failure paths.

### 3. `remove_empty_parent_dirs` will prune directories lny didn't create
`lny/src/step.rs:368-389`

The walk uses `path.ancestors()` and stops only at the first non-empty dir. There's no record of "did I create this dir during create_symlink?". Concrete footgun:

1. User runs `mkdir -p ~/.config/foo/bar` manually
2. lny creates a symlink at `~/.config/foo/bar/baz`
3. Later, user removes `baz` from the blueprint
4. lny removes the symlink, then prunes `bar`, `foo`, `.config` because they're now empty

This silently reverts directory structure the user owned. At minimum, lny should only prune directories it created in this run (or some tracked history). Bounding the walk to stop at the XDG base directories / home would also help.

It also walks *all the way to `/`*. In practice it stops at the first non-empty ancestor, but on a clean system (container/chroot with empty `/`) it could attempt `remove_dir("/")` — which fails, but it shouldn't even try.

### 4. `StepQueue::next()` pops LIFO, so execution order is reversed
`lny/src/step.rs:132-137`

`next()` does `self.steps.pop()`, but steps are pushed in insertion order. The queue ends up `[new-stuff..., removes...]`, and popping yields `removes` first (in reverse), then `new-stuff` in reverse. The two `for` loops in `main.rs` are consistent with each other so it doesn't break dry-vs-real, but:

- It's surprising for anyone reading the code
- It changes which step fails first if multiple fail
- The `Iterator` impl implies FIFO semantics that the body violates

If LIFO is intended, swap `push`/`pop` for `VecDeque` and document it. If FIFO is intended, iterate `self.steps` directly or use `drain(..)`.

## Smaller bugs / rough edges

### 5. Lost error context in `replace_symlink` cleanup
`lny/src/step.rs:302-311`

```rust
let rename_ret = rename(&tmp_dst, &dst).with_context(...);
if rename_ret.is_err() {
    remove_file(&tmp_dst).context(...)?;  // <- original error dropped
}
```

If rename fails *and* cleanup fails, the user sees the cleanup error and has no idea why the rename failed. Use `let rename_err = rename_ret.as_ref().err();` and attach it, or return both.

### 6. `Engine` initialization can panic the whole program
`lny/src/template.rs:18-33`

`LazyLock::new` calls `ContextOfTemplate::new().unwrap()`. If that fails (unusual XDG setup, missing HOME, etc.), the first `RenderedPath::deserialize` — i.e. parsing the blueprint — panics rather than returning a nice error. The `#[allow(clippy::unwrap_used)]` documents that this is intentional, but for a CLI that takes user input it's a harsh failure mode. Consider returning the error through the deserialize path.

### 7. `XDG_STATE_HOME is not set` message is misleading
`lny/src/template.rs:82-88`

`xdg.state_dir()` from `etcetera` returns `Some(~/.local/state)` by default on Linux. The bail only triggers on platforms where the strategy returns `None`. The error message blames `XDG_STATE_HOME` even when the user never touched it. Either drop the bail (use the default) or fix the message.

### 8. No validation that `src != dst`
`lny/src/blueprint.rs`

Nothing rejects `{"src":"/foo","dst":"/foo"}`. `create_symlink` will then create a self-referential symlink that fails to resolve with ELOOP. `replace_symlink` similarly. Cheap to check at validation time.

### 9. No validation that `src` exists
Probably intentional (you may want to link before installing), but worth at least a debug-level warning so users notice dangling links they typo'd into the blueprint.

### 10. `Step::Nothing` is queued, cloned, and iterated
`lny/src/step.rs:99-100`, `lny/src/main.rs:80-88`

`Nothing` steps take space, get cloned in the dry pass, and consume an iteration in both passes. Filter them out in `StepQueue::new` (or in a post-process) — they carry no information.

## Safety model — what's good

To end on the positive side, the parts that matter most for not destroying the user's files are correct:

- `DstFact::check` uses `try_exists_no_traverse` (verified: `crates/ino_path/src/lib.rs:32-42` uses `symlink_metadata`), so **broken symlinks at `dst` are correctly treated as occupied**, and the `read_link` comparison recovers the "is this ours" check.
- `is_collision()` correctly distinguishes "ours" (`SymlinkToSrc`) from "someone else's" (`Exist`, `SymlinkNotSrc`).
- The `Exist` / `SymlinkNotSrc` refusal-before-mutation is sound.
- Replace uses the canonical atomic-rename pattern with same-directory tmp (so the rename really is atomic on POSIX), and cleans up the tmp on failure.
- Strict-undefined Minijinja + `deny_unknown_fields` + version check make blueprints fail loudly on typos.

The biggest unaddressed risk isn't a code bug — it's that **the dry pass doesn't actually validate executability** (item 2). The "Check collision" stage gives false confidence. Everything else flows from that.
