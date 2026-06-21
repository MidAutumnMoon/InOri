# TASK: Refactor imgo transcoder trait → `External` + `Pixel`

## Goal

Split the single `Transcoder` trait (hard-wired around shelling out via
`Command`) into two focused traits sharing a common `Meta` super-trait,
so that in-process pixel operations (TomatoScramble) can implement the
same orchestration path without duplicating ~200 lines of control flow.

## Target trait shape

```rust
// imgo/src/transcoder/mod.rs

/// Metadata shared by all transcoder kinds.
pub trait Meta: Send + Sync {
    fn id(&self) -> &'static str;
    fn input_formats(&self) -> &'static [ImageFormat];
    fn output_format(&self) -> ImageFormat;
    fn default_jobs(&self) -> NonZeroU64;
}

/// Shell-out transcoders. Sans-IO: returns a process declaration; the
/// orchestrator spawns it.
pub trait External: Meta {
    fn transcode(&self, input: &Path, output: &Path) -> Command;
}

/// In-process pixel transcoders. Sans-IO: orchestrator decodes/encodes;
/// the impl only mutates pixels.
pub trait Pixel: Meta {
    fn transform(&self, img: &mut RgbaImage) -> anyhow::Result<()>;
}
```

The name `Transcoder` is dropped. The two execution shapes (external
process vs. in-process pixels) are real and permanent; pretending
otherwise leaks the difference into a return-type enum.

## Steps

### 1. `transcoder/mod.rs` — define the new traits (Done)

- Replace `trait Transcoder` with `trait Meta` + `trait External: Meta` +
  `trait Pixel: Meta` as shown above.
- Keep `ImageFormat`, `Image`, `RelAbs`, `BaseSeqExt` re-exports as-is.

### 2. Migrate existing shell-out impls (mechanical)

`avif.rs`, `jxl.rs`, `magick.rs`:

- `impl Transcoder for X` → `impl External for X` (the `transcode` method
  signature is unchanged).
- Add `impl Meta for X` (move `id` / `input_formats` / `output_format`
  / `default_jobs` into it). One `impl Meta` + one `impl External` block
  per type.

No behavioral change for these.

### 3. New `transcoder/tomato.rs` — the `Tomato` struct + `Pixel` impl

```rust
#[derive(Debug, clap::Args)]
#[group(id = "TomatoTranscoder")]
pub struct Tomato {
    #[arg(long)]
    pub encrypt: bool,
    #[arg(long)]
    pub decrypt: bool,
    #[arg(long, default_value_t = 1.0)]
    pub key: f64,
}

impl Tomato {
    fn mode(&self) -> anyhow::Result<bool> { /* moved from i.rs */ }
}

impl Meta for Tomato {
    fn id(&self) -> &'static str { "tomato" }
    fn input_formats(&self) -> &'static [ImageFormat] {
        &[ImageFormat::PNG, ImageFormat::JPG, ImageFormat::WEBP, ImageFormat::GIF]
    }
    fn output_format(&self) -> ImageFormat { ImageFormat::PNG }
    fn default_jobs(&self) -> NonZeroU64 { /* available_parallelism */ }
}

impl Pixel for Tomato {
    fn transform(&self, img: &mut RgbaImage) -> anyhow::Result<()> {
        scramble_image(img, self.key, self.mode()?);
        Ok(())
    }
}
```

The algorithm itself (gilbert2d, scramble_rgba, scramble_image, tests)
stays in `imgo/src/tomato.rs` — `transcoder/tomato.rs` only holds the
struct + trait glue, mirroring how `avif.rs` etc. are structured.

### 4. Extract shared orchestration

Add to `imgo/src/transcoder/mod.rs` (or a new `imgo/src/pipeline.rs` —
pick the one that fits the existing layout):

```rust
fn orchestrate(
    images: Vec<Image>,
    shared: &SharedOpts,
    output_format: ImageFormat,
    execute: impl Fn(&Image, &Path /* temp */) -> anyhow::Result<()>
        + Send + Sync,
) -> anyhow::Result<()>;
```

`orchestrate` owns, unchanged from the current transcoder path:
- temp file creation (suffix from `output_format.exts().first()`)
- backup (move source to `.backup/...` unless `no_backup`)
- destination filename + seq-conflict resolution
- progress bar (`indicatif`, `ceprintln!` for status/errors)
- rayon thread pool (`jobs` threads)
- per-image error reporting via `bar.suspend(|| ceprintln!(Red, ...))`

The `execute` closure does the actual work (spawn command, or
decode→transform→encode) and logs via `tracing` for debug-level output.
It must not touch the progress bar directly.

Two thin public wrappers:

```rust
pub fn run_pipeline_external(
    images: Vec<Image>,
    shared: &SharedOpts,
    transcoder: &dyn External,
) -> anyhow::Result<()> {
    let output_format = transcoder.output_format();
    orchestrate(images, shared, output_format, |img, temp| {
        let input_path = img.path.original_path();
        let mut cmd = transcoder.transcode(&input_path, temp);
        let out = cmd.output()
            .with_context(|| format!("spawn {}", transcoder.id()))?;
        if !out.status.success() {
            bail!(
                "{} failed (exit {:?}):\nstderr: {}",
                transcoder.id(),
                out.status.code(),
                String::from_utf8_lossy(&out.stderr)
            );
        }
        Ok(())
    })
}

pub fn run_pipeline_pixel(
    images: Vec<Image>,
    shared: &SharedOpts,
    transcoder: &dyn Pixel,
) -> anyhow::Result<()> {
    let output_format = transcoder.output_format();
    orchestrate(images, shared, output_format, |img, temp| {
        let input_path = img.path.original_path();
        let mut rgba = image::open(&input_path)
            .with_context(|| format!("decode {}", input_path.display()))?
            .to_rgba8();
        transcoder.transform(&mut rgba)?;
        rgba.save(temp)
            .with_context(|| format!("encode {}", temp.display()))?;
        Ok(())
    })
}
```

### 5. Warnings stay as-is

The Tomato-specific warnings (one-time metadata note; per-image >8-bit
downconversion; per-image GIF first-frame) currently use
`bar.suspend(|| ceprintln!(Yellow, ...))` and need progress-bar access.

Per the user's decision, **leave these warnings exactly as they are for
now**. They are out of scope for this refactor — a future task will
decide where they belong (likely some kind of callback hook on the
orchestration, or a `tracing` layer that the progress bar subscribes
to).

Concretely: the warnings stay in `run_pipeline_pixel`'s `execute`
closure for the moment (which has `img`, `bar`, and `input_path` in
scope). This is a temporary wart, not a design decision — it keeps the
refactor mechanical and reversible. The `Pixel::transform` impl itself
remains sans-IO and warning-free.

### 6. Rewrite `main()` dispatch

Each `CliOpts` variant builds its trait object and calls the matching
pipeline:

```rust
match &cliopts {
    CliOpts::GenComplete { shell } => { /* unchanged */ }
    CliOpts::Avif    { transcoder, shared } => run_pipeline_external(images, shared, transcoder),
    CliOpts::Jxl     { transcoder, shared } => run_pipeline_external(images, shared, transcoder),
    CliOpts::Denoise { transcoder, shared } => run_pipeline_external(images, shared, transcoder),
    CliOpts::CleanScan { transcoder, shared } => run_pipeline_external(images, shared, transcoder),
    CliOpts::Tomato  { tomato, shared } => run_pipeline_pixel(images, shared, tomato),
}
```

The image-collection logic (currently duplicated between `main()` and
`run_tomato`) gets pulled into a helper `collect_images_for(formats,
shared)` used by both paths.

### 7. Delete `run_tomato`

Its body is folded into `run_pipeline_pixel` + the Tomato `Pixel` impl.

### 8. Re-exports / `lib.rs`

- Export `Meta`, `External`, `Pixel`, `Tomato`.
- Drop the old `Transcoder` name. (App crate, no external consumers
  expected; this is an internal rearch.)

### 9. Validation

```sh
cargo fmt --all
cargo clippy --all-features -- -D warnings
cargo test --all-features
```

Plus an end-to-end CLI smoke test (scramble → descramble a real PNG,
sha256-compare to the original) to confirm the Tomato path still works
through the new `Pixel` plumbing.

## Decisions baked in

1. **Super-trait named `Meta`** (not `TranscoderMeta`).
2. **Warnings stay as-is** (bar-aware `ceprintln!` inside the
   `run_pipeline_pixel` closure). The `Pixel::transform` impl is sans-IO
   and warning-free. Warning-handling design is deferred to a future
   task.
3. **Two pipeline functions + one private `orchestrate` core**, not a
   `Work` enum. Keeps sans-IO honest: each trait declares its work shape
   (`Command` vs. pixel-mutation fn); the orchestrator executes all fs
   and process IO in both cases.
4. **`Tomato` struct lives in `transcoder/tomato.rs`**; the algorithm
   stays in `tomato.rs`. Matches the `avif.rs` / `jxl.rs` / `magick.rs`
   layout.
5. **Name `Transcoder` is dropped** entirely. No compat shim.

## Files touched

| File | Change |
|---|---|
| `imgo/src/transcoder/mod.rs` | New `Meta`/`External`/`Pixel` traits; `orchestrate`, `run_pipeline_external`, `run_pipeline_pixel` helpers; image-collection helper. |
| `imgo/src/transcoder/avif.rs` | `impl Transcoder` → `impl Meta + impl External`. |
| `imgo/src/transcoder/jxl.rs` | Same. |
| `imgo/src/transcoder/magick.rs` | Same (for `Denoise` and `CleanScan`). |
| `imgo/src/transcoder/tomato.rs` | New: `Tomato` struct + `impl Meta + impl Pixel`. |
| `imgo/src/tomato.rs` | Unchanged (algorithm + tests). |
| `imgo/src/lib.rs` | Re-exports: drop `Transcoder`, add `Meta`/`External`/`Pixel`/`Tomato`; add `transcoder::tomato`. |
| `imgo/src/bin/i.rs` | Rewrite dispatch per step 6; delete `run_tomato`. |

## Out of scope

- Redesigning how transcoders emit progress/warnings to the user
  (tracked as a future task).
- Adding new transcoders.
- Touching the algorithm in `tomato.rs`.
