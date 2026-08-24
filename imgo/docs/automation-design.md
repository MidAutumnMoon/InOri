# imgo automation workflow

For parameter-selection lessons and measurement practice, see
[tuning-experience.md](tuning-experience.md).

## Goal

`imgo` minimizes downloaded manga, doujinshi, and illustrations without applying one destructive filter to every page.

For still images, the useful term is **rate-distortion optimization**: reduce encoded bytes (or bits per pixel) while keeping the distortion acceptable at the intended viewing scale. Preprocessing can improve that tradeoff by removing information the encoder would otherwise spend bytes preserving. It can also erase intentional screentone, pencil texture, gradients, or fine line art. No scalar metric can decide that artistic boundary reliably.

The workflow therefore automates the repetitive work and keeps destructive decisions reviewable:

```text
analyze and group -> encode representative candidates -> review at 1:1
-> edit one recipe choice per group -> run the mixed batch once
```

## Commands

The workflow has two CLI phases and one human boundary:

```sh
# Phase 1: classify in parallel, encode representative candidates, write the
# plan, then stop.
i plan <dir>

# Human boundary: inspect the actual AVIF/JXL candidates and choose one recipe
# per group by editing selected_recipe.
$EDITOR <dir>/imgo-plan.json

# Phase 2: apply each selected recipe to its group.
i run <dir>/imgo-plan.json
```

The command names communicate ownership:

- `plan` produces every fact and artifact needed for a decision;
- the user owns only `selected_recipe`;
- `run` consumes that reviewed decision.

There is no separate prepare or preview phase. `plan` already parallelizes image
analysis (`-J` overrides the default of at most four workers) and performs the
unavoidably expensive representative candidate encodes. `run` reuses the exact
reviewed selected bytes for each representative and transcodes the remaining
files. If a review artifact is missing or stale, `run` performs that image’s
normal recipe instead.

Direct expert transcoder commands remain separate tools, not workflow phases:

```sh
i avif --quality 65 --chroma auto <files...>
i jxl <files...>
i denoise --mode artifact <files...>
i clean-scan --threshold 55 <files...>
```

### Review layout

The managed review directory contains the exact encoded candidates:

```text
.imgo-review/
└── 001--degraded-scan-high-resolution--001/
    ├── 00-original--001.png
    ├── 01--clean-scan-jxl.jxl
    ├── 02--avif-grayscale-compact-high-resolution.avif
    ├── 03--avif-safe-high-resolution.avif
    └── review.txt
```

No `.preview.png` derivatives are generated. The ordinal, content group, and
source stem provide the mapping to the original. `review.txt` records which
recipe was initially selected and the size of every candidate.

The hidden root manifest maps each source and recipe to the reviewed artifact.
It records source metadata, the exact serialized recipe, artifact path, size,
and mtime. `run` validates that manifest before reusing bytes. Changing a
source, recipe, or reviewed artifact turns reuse into a normal transcode.

Review generation is fully staged before the previous managed directory is
replaced. An old or unrelated `.imgo-review` requires `plan --force`.
`--force` also disables artifact reuse and performs fresh candidate encodes,
which is the correct choice after upgrading ImageMagick, avifenc, or cjxl.

## Classification model

The analyzer measures decoded content before assigning one valid content
category:

- color occupancy;
- exact line art versus general grayscale;
- grayscale entropy and near-black/white occupancy;
- high-frequency microtexture;
- microtexture directional coherence;
- low-amplitude fine variation globally and in local tiles;
- locally smooth midtone occupancy;
- canvas resolution.

The category representation excludes combinations that cannot exist, such as
colored line art marked as grayscale texture. User-facing group names describe
content and resolution: `textured-color-low-resolution`,
`manga-sand-tone-heavy-standard-resolution`, or
`grayscale-high-resolution`. The plan stores relative file paths, source size
and nanosecond-mtime guards, group metrics, one representative, a selected
recipe, and an explicit candidate catalog.

This deliberately does **not** treat JPEG extension or 8-pixel block-boundary energy as proof of JPEG damage. On the reference corpus, the strongest 8-pixel signal came from a clean grayscale/halftone image, while the clean JPEG had a much weaker signal. Deliberate panel geometry and screentone make blockiness heuristics unreliable.

### Automatic choices

- Decoded black/white line art selects mathematically lossless JPEG XL directly.
- The `degraded-scan` category selects `clean-scan-jxl`.
- `manga-sand-tone-{light,medium,heavy}` selects matching destructive flattening followed by AVIF speed 2.
- Other grayscale images select direct monochrome AVIF.
- Color images select direct AVIF with automatic chroma sampling.
- Higher-resolution canvases use a more compact default quality because review is modeled around an approximately 2k-pixel viewing scale.

### Destructive routing

`clean-scan-jxl` is selected only when each page independently has:

- at least 65% near-black/white pixels;
- mean error to its thresholded binary value at most 18/255;
- at most 5% of pixels in the threshold-sensitive 45–65% luminance band;
- at most 1% locally smooth midtone pixels;
- a longest edge of at least 1800 pixels.

This separates degraded-scan references from clean pages
with meaningful smooth grayscale. The strong class gets its own group, so one
page cannot make an averaged mixed group destructive. It still has
`review_required = true`, and direct AVIF remains a candidate.

Manga sand-tone routing is also automatic because the storage policy
explicitly discards that styling. Two evidence paths share page-level guards
of at least 25% near-black/white occupancy, mean binary error at least
19.5/255, at most 42% smooth midtones, and directional coherence at most
0.15:

- global evidence requires entropy at least 4.2, at least 20% sampled
  microtexture, and at least 20% soft noise in the noisiest 8×8 tile;
- regional evidence divides the sampled page into 16×16 tiles and requires at
  least one qualifying tile (approximately 0.35% page coverage) plus at least
  45% local microtexture. Each qualifying tile independently satisfies the
  occupancy, binary-error, smoothness, and coherence guards.

This catches full pages whose dense tone panels are diluted by large clean
areas. Existing globally detected pages keep the 25%/45% light-medium-heavy
boundaries. Region-only pages use medium strength below 50% local
microtexture and heavy strength otherwise.

The selected recipe computes a dense high-frequency mask, removes small
connected detections, feathers its boundaries, and applies low-pass filtering
and 12/8/6-level quantization only inside the surviving regions. Smooth
gradients and text outside those regions remain original. All three strengths
and unprocessed direct AVIF remain candidates.

Other destructive operations remain unselected candidates:

- every textured grayscale group gets a masked light sand-tone candidate even
  without confident classification;
- non-scan grayscale groups with weak regional evidence get a matching masked
  light/medium candidate;
- bilateral denoise remains available for general grayscale texture;
- despeckle remains available for broad pencil-like grayscale texture;
- AOM denoise/grain synthesis remains available for densely textured color;
- lower-quality and, for color, 4:2:0 compact AVIF variants remain available.

Near-black/white percentage or histogram shape alone is insufficient: sharp
clean screentone and degraded scans overlap on both.

## Encoder policy

### AVIF

The AVIF recipe uses libavif's documented quality control rather than combining `--qcolor` with a separate libaom `cq-level`:

- color quality is explicit in `0..=100`;
- alpha is lossless;
- 10-bit output is the default;
- `--yuv` is omitted for `auto`, allowing grayscale PNG to become 4:0:0 and color PNG to remain 4:4:4;
- 4:2:0 plus SharpYUV is an explicit compact color alternative;
- Exif/XMP are stripped, but ICC profiles are retained;
- AOM grain synthesis is opt-in per recipe, never global.
- speed 5 remains the general default; sand-tone flattening uses speed 2
  because the storage-first route already accepts substantial visual change
  and slower speeds below 2 had sharply diminishing returns;

The old global grain switch was the largest proven defect. On the sharp-screentone reference, the old settings produced 102,274 bytes with SSIMULACRA2 42.7; disabling grain produced 99,600 bytes with score 90.2. It was simultaneously larger, slower, and much more distorted.

Forced 4:2:0 is also not a safe universal default for colored line art. On the clean-color reference at quality 65, automatic 4:4:4 produced 292,626 bytes and score 85.4; 4:2:0 produced 241,608 bytes and score 80.8. Both are available through review instead of being conflated with quality.

For the 4118x3096 clean grayscale reference, quality 55 scored 81.8 at native resolution and 89.0 after both source and result were downscaled to a 2000-pixel viewing bound. Quality 65 scored 85.0 native and 90.6 at that viewing bound. This supports scale-aware defaults while keeping a higher-quality alternative editable in the plan.

### JPEG XL

One set of expert modular parameters was not consistently smallest. For the two one-bit references:

- the expert strategy won 91,406 versus 93,634 bytes on one page;
- standard effort 9 won 45,120 versus 46,995 bytes on the other.

Lossless JXL recipes therefore run both strategies and retain the smaller result. Both commands request mathematically lossless coding; reference roundtrips, including alpha and invisible RGB, were byte-exact after canonical RGBA decoding.

## How to judge alternatives

Use two separate checks:

1. **Encoding distortion:** compare an encoded result with the input to that encoder. SSIMULACRA2 at native and intended viewing scale is useful for choosing AVIF quality and detecting accidental line damage.
2. **Preprocessing intent:** compare the preprocessed representative with the original at 1:1. A metric treats intended denoising as an error and can penalize synthesized grain even when it looks acceptable. This decision remains visual.

Discard a preprocessing recipe if it is both larger and visibly worse. The reference sharp-screentone page demonstrates why: adaptive blur increased the quality-65 AVIF from 98,511 to 295,580 bytes while reducing SSIMULACRA2 from 90.5 to 13.7.

## Execution invariants

Before mutating a source, `i run` validates the complete plan:

- schema version and unknown fields;
- relative paths and duplicate membership;
- source size and nanosecond-mtime identity guards;
- every step's parameters and format transition;
- executables required by images without a reusable review artifact;
- deterministic destination collisions.

Each image is then handled independently:

1. Reuse the exact reviewed selected candidate when its manifest entry remains valid; otherwise run the recipe through temporary files.
2. Verify that the final output is non-empty and flush it.
3. Move the original to `.backup`, preserving the relative tree.
4. Atomically persist the final output beside the source.

One bad page does not cancel unrelated pages. Successful results remain committed; failures are aggregated and returned with a non-zero exit status. If commit fails after backup, the next run can re-encode from the verified backup. Existing unrelated outputs are never silently overwritten or renamed with a numeric suffix.

`--no-backup` is available, but it intentionally gives up reliable resume detection and is not the default for automated runs.

## Current boundary

The implemented automation intentionally stops at reviewed image conversion:

- planning discovers PNG and JPEG sources;
- each content group gets one medoid representative, so exceptional pages still
  require attention;
- source identity uses size and modification time, not a cryptographic content
  hash;
- `.imgo-review` is derived state; old `.imgo-cache` directories from the
  abandoned intermediate design are unused and may be removed;
- archiving, completion notifications, backup purging, multi-book scheduling,
  and NAS transfer are not implemented by `imgo`.

These are current limits, not implicit promises hidden behind plan flags.

## External tools

The processing surface requires:

- ImageMagick 7 for preprocessing;
- `avifenc` from libavif for AVIF;
- `cjxl` from libjxl for JPEG XL.

The implementation probes tools required by work without a reusable review artifact. A reviewed one-image run can commit without invoking its encoder again.
