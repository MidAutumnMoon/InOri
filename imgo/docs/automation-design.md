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

```sh
# Analyze PNG/JPEG files and write <dir>/imgo-plan.json.
i plan <dir>

# Encode one representative with every candidate recipe.
# Outputs go to <dir>/.imgo-review by default.
i preview <dir>/imgo-plan.json

# Inspect *.preview.png at 1:1, then change selected_recipe for any group.
$EDITOR <dir>/imgo-plan.json

# Execute every selected recipe, with one shared progress run and backup tree.
i run <dir>/imgo-plan.json
```

Each group’s `candidate_recipes` is the reviewed catalog.
`selected_recipe` must name one candidate; choosing another candidate changes
only `selected_recipe`, not the candidate list.

`i run` is resumable with the default backup policy. Re-running a completed plan reports the completed files instead of creating numbered duplicates.

Direct expert commands remain available:

```sh
i avif --quality 65 --chroma auto <files...>
i jxl <files...>
i denoise --mode artifact <files...>
i clean-scan --threshold 55 <files...>
```

## Classification model

A flat label such as `screentone` is not enough. The analyzer measures orthogonal properties from decoded pixels:

- color occupancy;
- exact bilevel versus general grayscale;
- grayscale entropy and near-black/white occupancy;
- edge/detail energy;
- low-amplitude fine variation globally and in local tiles;
- canvas scale.

It groups images by measured properties such as `color-textured-small` or `gray-quiet-large`. The plan stores relative file paths, source size and nanosecond-mtime guards, group metrics, one representative, a conservative selected recipe, and an explicit candidate catalog.

This deliberately does **not** treat JPEG extension or 8-pixel block-boundary energy as proof of JPEG damage. On the reference corpus, the strongest 8-pixel signal came from a clean grayscale/halftone image, while the clean JPEG had a much weaker signal. Deliberate panel geometry and screentone make blockiness heuristics unreliable.

### Automatic choices

- An image containing only decoded black/white grayscale values selects mathematically lossless JPEG XL directly.
- Other grayscale images select direct monochrome AVIF.
- Color images select direct AVIF with automatic chroma sampling.
- Larger canvases use a more compact default quality because review is modeled around an approximately 2k-pixel viewing scale.

### Review-only alternatives

These are generated but never selected automatically:

- fixed-threshold one-bit conversion followed by lossless JXL;
- bilateral or light adaptive denoise followed by AVIF;
- despeckle for broad pencil-like grayscale texture;
- AOM denoise/grain synthesis for densely textured color images;
- lower-quality and, for color, 4:2:0 compact AVIF variants.

Sharp clean screentone and degraded near-bilevel scans can have nearly identical histograms. Thresholding either one automatically would be unsafe. The representative preview makes that ambiguity visible without requiring inspection of every page.

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
- all required executables;
- deterministic destination collisions.

Each image is then handled independently:

1. Run every recipe step through temporary files.
2. Verify that the final output is non-empty and flush it.
3. Move the original to `.backup`, preserving the relative tree.
4. Atomically persist the final output beside the source.

One bad page does not cancel unrelated pages. Successful results remain committed; failures are aggregated and returned with a non-zero exit status. If commit fails after backup, the next run can re-encode from the verified backup. Existing unrelated outputs are never silently overwritten or renamed with a numeric suffix.

`--no-backup` is available, but it intentionally gives up reliable resume detection and is not the default for automated runs.

## Current boundary

The implemented automation intentionally stops at reviewed image conversion:

- planning discovers PNG and JPEG sources;
- each feature group gets one medoid representative, so exceptional pages still
  require attention;
- source identity uses size and modification time, not a cryptographic content
  hash;
- derived files in the default `.imgo-review` directory are overwritten when
  preview is rerun;
- archiving, completion notifications, backup purging, multi-book scheduling,
  and NAS transfer are not implemented by `imgo`.

These are current limits, not implicit promises hidden behind plan flags.

## External tools

The processing surface requires:

- ImageMagick 7 for preprocessing and preview decoding;
- `avifenc` from libavif for AVIF;
- `cjxl` from libjxl for JPEG XL.

The implementation probes every tool needed by the selected plan before moving any source file.
