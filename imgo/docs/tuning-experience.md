# imgo compression tuning field guide

This document is about changing parameters safely. For workflow and execution architecture, see [automation-design.md](automation-design.md).

## The optimization target

For a still image, “bitrate” is better expressed as **rate**:

```text
bits_per_pixel = final_bytes * 8 / (width * height)
```

The real target is not minimum rate. It is the best **rate-distortion tradeoff at the intended viewing condition**.

Three decisions must remain separate:

1. **Preprocessing:** what source information may be removed or normalized?
2. **Encoding:** how much distortion may the codec add to that chosen source?
3. **Viewing:** is quality judged at 1:1, fit-to-screen, or a fixed display bound such as 2000 pixels?

An encoder metric can help with decision 2. It cannot decide whether pencil grain, fuzzy screentone, scan dirt, or a photographic background is artistically disposable. That is decision 1 and still needs a representative visual review.

## Rules learned the expensive way

- Change one axis at a time. A result comparing quality, chroma, bit depth, grain, and preprocessing simultaneously explains nothing.
- Compare decoded pixels, not container files or screenshots from different viewers.
- Keep tool versions and the complete command beside every result. Advanced codec options can change meaning across versions.
- Measure a group, not one lucky image. Use a typical page plus at least one high-detail or noisy outlier.
- Review both 1:1 and the intended viewing scale. Downscaling can hide harmless high-resolution artifacts, while 1:1 exposes damaged lines and screentone.
- Reject a candidate that is both larger and worse. Keep non-dominated candidates as a Pareto frontier.
- A preprocessing win on one sample becomes an alternative, not a default. A universal default must be neutral or better across every relevant reference family.
- Lossless strategies are different: after exact decoded-pixel verification, the smallest output wins automatically.
- `plan` records encoded representatives directly in `.imgo-review` using source metadata, the exact recipe, and artifact metadata. Recipe/source edits force a new encode; unchanged reviewed bytes are intentionally reused by `run`. Use `plan --force` after upgrading an encoder to measure fresh candidates.

The measurements quoted below used libavif 1.4.2 with libaom 3.12.1,
libjxl/cjxl 0.12.0, ImageMagick 7.1.2-29 Q16-HDRI, and SSIMULACRA2
2.1. Re-measure after a tool upgrade.

## A repeatable experiment

### 1. Freeze the corpus

Keep untouched originals and record:

- source path and size;
- decoded dimensions, channel count, bit depth, alpha, and color profile;
- source format;
- tool versions;
- intended display bound.

Do not mix full pages, explanatory zoom crops, already-processed outputs, and source pages in one classifier experiment. They have different statistics and will create misleading groups. Put crops and expected outputs in a separate directory.

For a book-like cohort, test:

- the page nearest the group average;
- the noisiest/detail-heaviest page;
- a page with thin text or hair lines;
- a page with flat gradients or large gray fills;
- any known exception such as a color insert.

### 2. Establish a control

Encode the original with the current selected recipe. Decode it. This is the control against which one parameter changes.

Record at least:

| Field | Meaning |
|---|---|
| final bytes | NAS cost |
| bits/pixel | rate independent of canvas area |
| source-to-final ratio | useful operationally, but source-format dependent |
| encode time | use the median of repeated runs under similar load |
| native metric | codec damage visible at 1:1 |
| view-scale metric | damage after both source and result use the same downscale |
| visual notes | lines, text, tones, gradients, color edges, alpha |

A compact log row is enough:

```text
sample, tool-version, recipe, changed-axis, bytes, bpp,
native-score, view-score, seconds, accepted, notes
```

### 3. Sweep narrowly

Start around an accepted value. Examples:

- AVIF quality: `55, 59, 62, 65, 68`;
- threshold: `48%, 50%, ... 62%`;
- adaptive-blur sigma: `0.3, 0.5, 0.8` with radius fixed;
- bilateral diameter: `3x3, 5x5`, but specify sigmas if the intent is to change only diameter.

A coarse sweep locates the useful region. A second narrow sweep chooses the final value. Do not spend time searching values already dominated by both size and quality.

### 4. Decode and measure

For encoder-only tests, compare the decoded result with the exact input handed to the encoder.

For preprocessing tests, keep two comparisons:

- preprocessed image versus original: a damage guard, not an objective truth;
- decoded final image versus preprocessed image: codec distortion.

SSIMULACRA2 is useful for encoder tuning and for catching catastrophic blur or line damage. Do not use its score to approve denoising or synthesized grain: removing or replacing stochastic texture lowers a pixel-reference score even when a human prefers the result. Establish metric targets from already accepted images instead of treating one universal score as law.

For view-scale measurement, resize both reference and decoded output with the same filter and dimensions before scoring. Never compare a resized output with an unresized reference.

### 5. Review the failure surfaces

Toggle candidates without rescaling or interpolation changes. Inspect:

- small Japanese text and punctuation;
- single-pixel or antialiased line art;
- hair, eyelashes, and speed lines;
- sharp screentone dots;
- fuzzy or resampled screentone;
- large flat gray and color regions;
- smooth color gradients;
- high-contrast colored edges, where 4:2:0 bleeding appears;
- corners and backgrounds, where localized noise is easy to miss;
- transparency boundaries.

## AVIF parameters

### Use one quality control

`imgo` uses libavif’s `--qcolor` scale: `0` is most lossy and `100` requests lossless color quantization. Chroma conversion such as 4:2:0 can still make the complete encode lossy. `imgo` deliberately does not combine `--qcolor` with a separate libaom `cq-level`; two quality controls make experiments ambiguous and can interact with quantizer bounds.

Approximate migration from the old AOM CQ scale is:

```text
libavif_quality ≈ 100 - cq_level * 100 / 63
```

Useful anchors are therefore roughly:

| Old CQ | New quality |
|---:|---:|
| 22 | 65 |
| 24 | 62 |
| 26 | 59 |
| 28 | 56 |
| 33 | 48 |

Current starting points are quality 68 for small canvases, 65 for medium canvases, and 55 for large canvases. These are viewing-scale priors, not claims that resolution alone determines quality. Each generated plan also offers a value seven points lower as a compact alternative.

Generated recipe IDs are semantic labels such as `avif-safe-large` and
`avif-color-compact-small`; parameter values live only in the recipe options.
Do not encode a copied recipe's current quality value into its ID, because the
label becomes false as soon as the experiment changes that value.

When changing these defaults, compare at equal viewing quality, not merely equal `--qcolor`. Content complexity means the same quality number does not imply the same file size or metric score.

### Chroma is a separate quality axis

With `chroma = auto`, `imgo` omits `--yuv`:

- grayscale PNG can remain monochrome 4:0:0;
- color PNG defaults to 4:4:4;
- JPEG sampling can be retained when possible.

This is the safe control for colored line art. The compact color alternative uses 4:2:0 plus SharpYUV.

Measure 4:2:0 separately from quality. It often saves substantial space, but metrics and visual inspection should focus on saturated text, outlines, eyes, and narrow colored shapes. Do not compensate for bad chroma edges by raising quality and then attribute the result to quality alone.

### Bit depth is not “higher always wins”

Ten-bit is the current default. When testing 8, 10, or 12 bits, record:

- bytes at comparable visual quality;
- encode and decode time;
- decoder/viewer compatibility;
- gradient banding;
- whether the source actually contains more than 8-bit samples.

A higher encoded depth can improve coding efficiency even for an 8-bit source, but the result is content-dependent. In the reference experiments, 12-bit helped one clean color image and lost on a sharp grayscale image. It is therefore not a universal preset change.

### Speed and concurrency

Speed 5 is the current starting point. Lower values may improve density, but compare the size gain with wall time across the corpus. A tiny saving that multiplies an overnight batch by several times is usually a bad default; it may still be a deliberate archival preset.

`avifenc` uses all internal jobs, while batch image parallelism defaults to one. Increasing both creates oversubscription and invalid timing data. Tune one concurrency layer at a time and watch peak memory as well as elapsed time.

### Grain synthesis is a content switch

`grain = true` passes a positive AOM `denoise-noise-level`. In all-intra still-image mode, treat this as an automatic denoise/noise-synthesis switch, not a stable numeric “grain strength” knob.

It can be excellent on genuinely grainy color art. It can be disastrous on crisp screentone. In the reference sharp-screentone sample, the old global grain setting was larger, slower, and dropped SSIMULACRA2 from about 90 to 43. Grain must remain an unselected alternative until reviewed.

The current textured-color reference confirms the useful case: at low-resolution
quality 68, direct 4:4:4 AVIF used 295,362 bytes, compact 4:2:0 used 189,911,
and 4:2:0 with AOM noise estimation/grain synthesis used 154,280. The decoded
grain candidate remained visually acceptable. This does not generalize to
grayscale manga sand tone: on the heavy sand sample the same switch changed
817,735 bytes to 818,186 while reducing the metric score.

### Color profiles and alpha

Alpha is encoded losslessly. Exif and XMP are stripped, but ICC profiles are retained. Never discard ICC and then assert sRGB metadata unless the pixels were actually converted to sRGB first. Otherwise a small metadata saving can produce a real color shift.

### Advanced libaom options

Add an advanced option only when it wins across a documented corpus and encoder version. First prove that stable libavif controls cannot express the same choice. Autotiling, for example, is already the avifenc default; spelling it again adds no tuning information.

## JPEG XL parameters

For PNG-like input, `cjxl --distance 0` is the command-line lossless control; effort changes time and density, not decoded quality. The current standard candidate is effort 9 with automatic threading.

The previous expert modular constants are retained as a second candidate:

```text
allow_expert_options
distance=0
effort=8
modular=1
lossless_jpeg=1
iterations=100
modular_nb_prev_channels=6
modular_group_size=2
modular_predictor=15
num_threads=-1
```

They are not “better settings.” One reference bilevel page favored them; another favored standard effort 9. `imgo` therefore runs both and keeps the smaller output.

When adding another lossless strategy:

1. Decode every candidate to a canonical pixel representation.
2. Compare dimensions, channel values, alpha, and bit depth exactly.
3. Only then choose by bytes and time.
4. Keep a new strategy only if its wins repay its extra encode cost across a meaningful corpus.

Do not confuse pixel-lossless PNG conversion with JPEG reconstruction. A JPEG-reconstruction JXL can reproduce the original JPEG bitstream; a pixel-lossless re-encode only promises the same decoded pixels. Test the property the preset claims.

If a future implementation uses the libjxl C API instead of `cjxl`, call the explicit frame-lossless API. Do not assume setting distance to zero alone configures every required lossless option in the API.

## ImageMagick constants

All current cleanup constants are starting points derived from the reference images. Their units differ; a value named “strength” is not comparable across operators.

### Bilateral: `3x3`

The default artifact mode is:

```text
-bilateral-blur 3x3
```

Width and height set a pixel neighborhood. Bilateral filtering also has intensity-space and coordinate-space sigmas. If omitted, ImageMagick derives both from the diameter. Consequently, comparing `3x3` with `5x5` changes the neighborhood and both implicit sigmas at once.

`3x3` was selected as a conservative edge-preserving alternative: it caused little metric loss on clean sharp tone, but its size savings were also small. It is not a JPEG-deblocking silver bullet. To study the operator itself, specify all parameters and sweep one sigma at a time.

### Adaptive blur: `1x0.5` versus `2x0.8`

ImageMagick syntax is `radius x sigma`. Sigma determines the blur amount; radius mainly bounds the Gaussian kernel storage.

- The manual legacy mode defaults to `2x0.8`.
- Automation offers the lighter `1x0.5` alternative.

Keep radius fixed while measuring sigma. The strong value can reduce noisy-page size, but on the sharp-screentone reference it increased AVIF from about 99 KB to 296 KB and collapsed the metric score from about 90 to 14. Blur can make screen dots fuzzy and harder—not easier—for a codec to represent.

### Median plus contrast stretch: `3x3`, `5%x0%`

The fake-pencil mode performs:

```text
-statistic median 3x3
-contrast-stretch 5%x0%
```

`3x3` is a pixel neighborhood. Median filtering removes structures smaller than that neighborhood; it does not have a continuous blur strength.

`5%x0%` is histogram population, not intensity. It permits up to 5% of pixels to be clipped to black and 0% to white while stretching the remaining range. Its effect therefore changes with page composition. A mostly white manga page and a dark illustration do not receive equivalent treatment.

Under the current AVIF recipe, this mode did not improve the stylized-noise sample: final size stayed approximately flat while the metric dropped sharply. The automation instead offers `-despeckle` for the detected pencil-like group. Keep fake-pencil manual until a broader corpus proves a stable target class.

### Despeckle

`-despeckle` has no numeric parameter. `imgo` rejects a `strength` value in this mode rather than silently ignoring it. Repeating the operation would create a stronger, different preset, so repetition count must be treated as a parameter. On the stylized-noise reference it saved about 8% but visibly altered texture and reduced the reference metric. It remains review-only.

### Manga sand tone: adaptive flattening

Clip Studio calls this intentional manga texture `砂目ノイズトーン`
(noise/sand tone) and exposes it as tone type `ノイズ`. Local dot density
represents gray, so AVIF sees expensive high-frequency detail instead of a
smooth tone. See [Clip Studio's tone guide](https://tips.clip-studio.com/en-us/articles/9181).

The project policy values storage and main-content legibility over preserving
this styling. Mean-shift was too conservative for that policy. Its color
distance rejects high-contrast black/white dots, so larger spatial windows do
not necessarily merge them. On the new 837×3390 heavy-tone sample at quality
55 and AVIF speed 2:

| Preprocessing | Bytes | Saving |
|---|---:|---:|
| none | 817,735 | — |
| mean-shift `5x5+15%` | 803,416 | 1.8% |
| mean-shift `7x7+50%` | 660,741 | 19.2% |
| adaptive heavy flattening | 148,377 | 81.9% |

The selected pipeline uses ordinary ImageMagick operations, but in a deliberate
multi-stage composition:

1. composite transparency onto white and convert to grayscale;
2. blur a base image with a resolution-scaled sigma;
3. disable dithering and posterize the base;
4. threshold the original at 20%, invert it, morphologically open it, and
   invert it again to retain solid dark line work while dropping small dots;
5. darken-compose that line layer over the flattened base.

The base sigma is
`clamp(longest_edge / 1400, 0.8, 2.8) × strength_multiplier`.
The presets are:

| Strength | Measured microtexture burden | Multiplier | Gray levels |
|---|---:|---:|---:|
| light | below 25% | 0.6 | 12 |
| medium | 25% to below 45% | 0.9 | 8 |
| heavy | 45% or above | 1.1 | 6 |

Line-layer opening uses `Disk:1` below 1800 pixels, `Disk:1.5` below 3500,
and `Disk:2` otherwise. This is not semantic segmentation: long or thick
decorative marks can survive with line art. It does preserve speech text,
panel borders, and character outlines on the measured samples without
reintroducing most fine stipple.

Across five representative target samples, both sides use AVIF speed 2 and
each resolution bucket's normal quality:

| Sample | Direct bytes | Flattened bytes | Saving |
|---|---:|---:|---:|
| heavy crosshatched sand tone | 817,735 | 148,377 | 81.9% |
| mixed sharp screentone and sand patches | 309,872 | 157,570 | 49.1% |
| very fine low-resolution sand tone | 127,009 | 27,227 | 78.6% |
| smooth main subject with a large tone field | 544,631 | 180,462 | 66.9% |
| sparse tone on a 4441×4503 page | 895,682 | 501,738 | 44.0% |

Reference metrics become very low because removal of the reference texture is
the intended result. Candidate inspection therefore checks main-content
legibility and encoded size, not fidelity to disposable stipple. Direct AVIF
and all three flattening strengths remain in the review bundle.

Complex edge masks were not automatically better. Canny detected the tone
itself as edges even after coarse smoothing; median, morphology-only,
Kuwahara, bilateral, and stronger mean-shift retained expensive patterns or
damaged lines for less storage gain. Research-grade manga separation combines
Laplacian-of-Gaussian and flow-based difference-of-Gaussian masks
([Ito et al., 2015](https://diglib7.eg.org/items/79cb54a0-109d-492e-ae83-0827a4eaccae)).
That is a specialized line-extraction system, not a useful amount of
ImageMagick command complexity for this storage-first route.

AV1 grain synthesis is also the wrong representation. AOM's model first
denoises the image and estimates zero-mean autoregressive grain from selected
flat regions
([AOMedia film-grain report](https://aomedia.org/docs/CWG-C051o_TR_AOMedia_film_grain_synthesis_technology_v2.pdf)).
Authored high-contrast manga tone is not stationary film grain. On the heavy
sample, enabling libaom noise estimation changed 817,735 bytes to 818,186
bytes (+0.06%) and reduced SSIMULACRA2 from 84.83 to 82.44.

Automatic routing requires multiple independent signals. Directional
coherence is computed from the sampled microtexture gradient structure tensor:
isotropic sand tone measured 0.01–0.10 in the new target set, while the
explicit non-sand vertical texture measured 0.28 and the strip-pattern sample
measured 0.37. The destructive class still requires entropy, occupancy,
binary-error, local-noise, smooth-region, and microtexture guards in addition
to coherence.

### Clean scan: unsharp plus threshold

The current one-bit conversion is:

```text
-background white -alpha remove -alpha off
-colorspace Gray -strip
-unsharp 0x2+1+0.4
-threshold 55%
-depth 1 -colors 2
```

For `0x2+1+0.4`:

- radius `0`: let ImageMagick choose the kernel radius;
- sigma `2`: edge scale in pixels;
- gain `1`: add the full original-minus-blurred difference;
- threshold `0.4`: apply only differences above 40% of QuantumRange.

The high threshold makes sharpening selective, but it still changes which pixels cross the later binary threshold. Operator order is part of the preset.

`-threshold 55%` maps values above 55% to white and the rest to black. The value came from the degraded-scan references; it is not a generic manga constant. Sweep threshold while watching thin lines, gray fills, closed screentone dots, and background dirt.

`-auto-threshold OTSU` chooses a cluster-based threshold from each image histogram. It can adapt to a shifted scan, but page-to-page variation can also create inconsistent line weights. Compare fixed and Otsu on a whole cohort, not one page. ImageMagick can expose the chosen Otsu value through its `auto-threshold:verbose` property; record it when diagnosing outliers.

`--otsu` and a customized fixed `--threshold` are mutually exclusive in the
recipe contract. This prevents a plan from carrying a threshold value that the
selected mode silently ignores.

The intermediate PNG uses compression level 1 for speed because it is consumed immediately by AVIF/JXL. Do not use intermediate PNG bytes to judge a preprocessing preset; measure the final encoded result.

## Classifier constants

Classifier thresholds route review work; they do not prove semantic image types.

Current 8-bit feature rules include:

- a pixel is chromatic when `max(R,G,B) - min(R,G,B) > 8`;
- color occupancy and the grayscale histogram scan every decoded pixel;
- an image is color when at least 1% of pixels are chromatic;
- exact bilevel means no chromatic pixels and only grayscale values 0 and 255;
- sampled “detail” uses Laplacian magnitude above 24;
- sampled “soft noise” uses Laplacian magnitude 4–20 with local gradient at most 24;
- microtexture is the non-overlapping sum of detail and soft-noise occupancy;
- sampled smooth midtones use luminance 33–222, gradient at most 6, and Laplacian at most 12;
- microtexture directional coherence is the normalized eigenvalue difference of its sampled gradient structure tensor;
- color texture triggers at 20% global soft noise or 35% in the worst tile;
- grayscale texture triggers at 8% global or 20% in the worst tile;
- manga sand tone additionally requires entropy at least 4.2, microtexture at
  least 20%, near-black/white occupancy at least 25%, worst-tile soft noise at
  least 20%, mean binary error at least 19.5/255, smooth midtones at most 42%,
  and directional coherence at most 0.15;
- sand-tone burden is light below 25% microtexture, medium below 45%, and heavy
  otherwise;
- resolution buckets use longest edges below 1800, below 3000, and 3000 or above;
- ambiguous near-bilevel grayscale gets a review-only clean-scan candidate at 68%;
- the `degraded-scan` category is selected for standard/high-resolution pages at 65% near-black/white when mean binary error is at most 18/255, threshold-sensitive band is at most 5%, and smooth midtones are at most 1%.

The second degraded-scan reference measured 65.5%, 17.8/255, 4.2%, and
0.38% on those four axes. Clean grayscale guards still fail the independent
threshold-band or smooth-midtone checks; this is why the rule was widened on
two axes rather than reduced to a single lower near-black/white cutoff.

Palette/color and threshold-band facts scan the full image. Local
edge/texture analysis samples at most two million coordinates and divides each
image into an 8×8 tile grid. Each image receives one content category before
group metrics are averaged, preventing a degraded scan or sand-tone page from
routing clean grayscale neighbors destructively. Tiny localized defects can
still be diluted.

When tuning routing thresholds, measure two costs:

- false positive on a review-only candidate: extra review work;
- false negative: a useful candidate is not offered;
- false positive on an automatically selected destructive recipe: damaged output.

Review-only routing may favor recall slightly. Automatic destructive routing
must favor precision and require independent signals. Never promote from
blockiness, entropy, extension, or near-black/white percentage alone: clean
sharp screentone and damaged scans overlap on all of them.

## Checklist for a new or changed preset

1. State the target content and an explicit anti-target.
2. Decide whether the change is encoder-only, preprocessing, or both.
3. Add it as an unselected candidate first; promote it only after independent signals separate the target from its anti-target.
4. Freeze representative and worst-case inputs.
5. Sweep one parameter around a control.
6. Record exact commands, versions, bytes, bpp, time, native metric, view metric, and visual notes.
7. Decode and verify any lossless claim exactly.
8. Check color profiles, alpha, bit depth, and chroma—not only RGB appearance on white.
9. Remove dominated candidates.
10. Change a default only after every relevant reference family remains acceptable.
11. Update the behavioral test or reference measurement that explains the constant.

A constant without its measurement and anti-target is folklore. Keep it as an alternative until the evidence is broader.

## Primary references

- [libavif `avifenc` manual](https://github.com/AOMediaCodec/libavif/blob/main/doc/avifenc.1.md)
- [libavif discussion of all-intra denoise/grain behavior](https://github.com/AOMediaCodec/libavif/issues/1137)
- [libjxl `cjxl` manual](https://github.com/libjxl/libjxl/blob/main/doc/man/cjxl.txt)
- [libjxl lossless API contract](https://github.com/libjxl/libjxl/blob/main/lib/include/jxl/encode.h)
- [ImageMagick command-line option reference](https://imagemagick.org/command-line-options/)
- [AOMedia film-grain synthesis technical report](https://aomedia.org/docs/CWG-C051o_TR_AOMedia_film_grain_synthesis_technology_v2.pdf)
- [Ito et al., “Separation of Manga Line Drawings and Screentones”](https://diglib7.eg.org/items/79cb54a0-109d-492e-ae83-0827a4eaccae)
- [Clip Studio tone guide](https://tips.clip-studio.com/en-us/articles/9181)
