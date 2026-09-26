# B1-6 outputs, codecs and stills

Scope: every bullet of B1-6 in `docs/schema-1.1-batch1-proposal.md` §4, which
also completes the "multiple `output`" part of the B1-0 root section. Base:
`main` at `4106b84`. Lane branch `b1-6-outputs`.

## 1. Goals and non-goals

In scope:

- several `output` elements with `id`, grouped into render passes;
- per-output `start`/`end`, `width`/`height`/`fps`;
- codecs `prores` (prores_ks + `proresProfile`), `vp9` (libvpx-vp9), `av1`
  (libsvtav1), `png-sequence`, `tiff-sequence`, `exr-sequence`, `gif`,
  `apng`;
- `container`, `keyframeInterval`, `bFrames`, `faststart`, `loopCount`;
- `poster` and `thumbnail` stills in PNG and JPEG;
- the CLI rules for selecting outputs and the interaction with `--output`,
  `--frame-range`, `--preview-frame`, `--hash` and `--resume`.

Stay unsupported (capability rows remain `false`, so a use fails with
"unsupported in this build"): still `format="webp|avif"` (as the proposal
says), still `@marker` (markers are B1-5; flipped when both land), output
`layout`, `variant`, `destination`, `alpha`, `twoPass`, `maxFileSize`,
`profile`, `level`, `maxBitrate`, `bufferSize`, `transfer`, HDR metadata,
`captions`, `audioLanguages`, `representation`; codecs `dnxhr`, `webp`,
`jpeg-sequence`, `audio-only`; containers `mxf`, `wav`, `m4a`, `mp3`.

## 2. Data model

`SrScene.output` keeps its meaning: the first `<output>` (or the built-in
default when the document has none). A second and later `<output>` go into a
new owned array `SrScene.extra_outputs` (document order). `src/outputs.c`
gives uniform access (`sr_scene_output_count`, `sr_scene_output_at`) so no
other code needs to know about the split. This keeps every existing reader of
`scene->output` (1.0 behaviour) untouched.

`SrOutput` gains (all owned by the scene, freed by `sr_output_free`):

| Field | Meaning |
|---|---|
| `id` | optional string; required when the document has more than one output |
| `container` | `SR_CONTAINER_AUTO` (derive, the 1.0 rule) or mp4/mov/mkv/webm |
| `width`, `height`, `fps_num`, `fps_den` | 0 = inherit the project (after CLI overrides) |
| `start`, `end`, `has_end` | seconds of composition time |
| `keyframe_interval` | seconds, 0 = not authored |
| `b_frames` | -1 = not authored |
| `faststart` | default true (the 1.0 behaviour) |
| `loop_count` | plays, 0 = infinite |
| `prores_profile` | enum, `SR_PRORES_AUTO` = not authored |
| `pixel_format_authored`, `audio_codec_authored` | for per-codec defaults |
| `stills`, `still_count` | owned `SrStill` array |

`SrStill` = `{kind (poster|thumbnail), time, path, format (png|jpeg),
width (0 = output width), quality, source_line}`.

`SrCodec` is extended by appending values, so the integers already written
to resume manifests do not change.

## 3. Loader

New module `src/xml_outputs.c` holds the `output` handler (moved out of
`xml_elements.c` unchanged for the 1.0 attributes) and the `poster` /
`thumbnail` handler. `xml.c` changes only its dispatch rows: `output` is no
longer a once-only section, and `poster`/`thumbnail` are children of
`E_OUTPUT` with minimum version 1.1. `xml_resolve.c` calls one new function,
`sr_xml_resolve_outputs`, which performs every cross-field check:

- A second `output` needs `version="1.1"` ("requires version=\"1.1\""), and
  the count is bounded by `SR_MAX_OUTPUTS` (16).
- When there is more than one output every output needs a unique `id`.
- `start >= 0`, `start < end <= project duration`, finite values.
- `width`, `height` ≤ `SR_MAX_OUTPUT_DIMENSION` (16384); a per-output size
  that differs from the project is rejected in `mode="equirectangular"`
  (the canvas is the 360 panorama itself).
- Codec/container compatibility (table in §5), and attribute applicability:
  an attribute that has no meaning for the codec (`crf` on ProRes,
  `loopCount` on H.264, `bFrames` on VP9, `faststart` on Matroska, …) is a
  load error rather than being silently ignored. 1.0 codecs keep their 1.0
  acceptance exactly (`crf`/`preset` on FFV1 remain accepted and unused).
- Sequence paths hold exactly one integer conversion (`%d` or `%0Nd`,
  N ≤ 9), `%%` escapes, nothing else.
- No two writers can produce the same file: plain paths are compared
  exactly; a sequence pattern collides with another pattern when their
  literal prefix and suffix are equal (so `f-%d.png` and `f-%04d.png`
  collide), and with a plain path the pattern can generate. The check runs
  at load and again at plan time on the effective paths (after `--output`).
- Still count per output ≤ `SR_MAX_OUTPUT_STILLS` (16); the still time must
  lie inside its output's `[start, end)`; still `width` ≤ 16384 and the
  derived height is in `[1, 16384]`.
- Per-output `fps` is between 1 and 1000 frames per second (bounds the
  per-frame audio block and the frame count).

The resolve pass fills per-codec defaults when not authored: pixel format
(ProRes `yuv422p10le` or `yuv444p10le` for 4444/4444xq, VP9/AV1 `yuv420p`,
GIF `pal8`, APNG/PNG/TIFF `rgb24`, EXR `gbrpf32le`), audio codec `libopus`
for WebM, container.

The capability gate keeps doing the per-construct work: the occurrence limit
`scene/output = 1` is removed from `schema/capabilities.json` and the rows
listed in §9 are flipped when rendering, tests and docs are complete.

## 4. Passes and the CLI

### Selection

- Default: every output in document order.
- `--output-id ID[,ID...]`: the listed outputs, in document order; an
  unknown id is an argument error (exit 2) that names the id.
- `--output FILE` overrides the path of the single selected output; with
  more than one selected output it is an argument error. The container of
  a 1.0 codec, or of a new codec without `container`, is derived from the
  effective path (so `--output x.mkv` still switches to Matroska).
- `--resolution` and `--fps` change the project; outputs without their own
  `width`/`height`/`fps` inherit them, authored per-output values win.
- `--quality` applies to every selected output whose codec has rate
  control (h264, h265, vp9, av1), as it does to the single 1.0 output.
- `--preview-frame N` and `--hash` operate on exactly one output: the one
  given by `--output-id` (which must then name one output) or the first
  output. They render at that output's width, height, fps and colour space.
  A 1.0 scene has one output, so both behave exactly as before.

### Output paths

Each output writes `path` resolved against the scene directory (the 1.0
rule). A sequence path is a pattern; frame files are numbered by absolute
composition frame index at the output's frame rate, so `--frame-range`
slices of a sequence need no renaming. Parent directories are created.
Still paths resolve the same way.

### Pass planning (`src/outputs.c`)

A pass is `(width, height, fps_num, fps_den)` after CLI overrides plus the
list of outputs ("sinks") that render at that key. Outputs with an equal key
share one pass: every frame is composited once and every sink converts and
encodes the same float frame. Outputs with a different size or rate get
their own pass; nothing is rescaled (rescaling would break relative lengths
and viewport units, which are resolved against the output frame). Passes are
ordered by their first output in document order; sinks in document order.

Per pass the renderer loads assets and prepares physics exactly as the
single-output path does today (physics depends on the frame size, so it is
re-prepared for every pass). The project fields are set to the pass key
before the pass and restored after it; nothing is written during frame
evaluation.

### Frame ranges

Output k at rate r covers frames `[F(start), F(end))` with
`F(t) = ceil(t * r - 1e-12)` (the formula already used for the project frame
count), `end` defaulting to the project duration. `--frame-range A:B` is
applied to every selected output in that output's own frame numbering and
intersected with its range. A pass renders the union of its sinks' ranges;
a sink receives only its own frames. The encoded file starts at time 0 at
its first frame, exactly like `--frame-range` does today; audio is mixed at
absolute sample times, so it stays aligned. An output whose intersection is
empty is skipped with an info diagnostic; when no selected output has a
frame the render fails as today ("empty frame range").

### `--resume`

Each selected output is rendered in its own pass with its own
`OUTPUT.parts/` directory, so segment reuse per output is independent. Only
container video codecs can resume (h264, h265, ffv1, prores, vp9, av1);
`gif`, `apng` and the sequences are rejected with `--resume` (argument
error) because they have no packet-copy assembly. Segment files are `.mp4`
for H.264/H.265 (unchanged) and `.mkv` for the rest.

### Stills

The still's time must lie inside its output's range (load error
otherwise), so the slice of a sliced render that contains the still's frame
also renders that output. A still shows the frame of its output that is displayed at `time`:
`floor(time * r + 1e-9)` at the output's rate. It is rendered after the
output's frames, inside the same pass when it has the output's width, or in
a stills-only pass at `width x round(width * H / W)` otherwise (render, not
rescale). It is written when the output is rendered without
`--frame-range`, or when the range contains the still's frame, so exactly
one slice of a sliced render writes it. `--hash`, `--preview-frame` and
`--validate` write no stills. Stills are converted to 8 bits in the output's
colour space and written as PNG (`rgb24`) or baseline JPEG (`yuvj420p`,
`qscale = round(2 + (1 - quality) * 29)`), both with bit-exact flags.

## 5. Codecs

All new codecs require `version="1.1"`, where codec workers are already
pinned to one (`encoder.c`, 1.1 policy), so container bytes cannot depend on
`--threads`. The scaler (RGB → Y'CbCr) keeps its thread count; its slices
are byte-identical at every count (existing tested property). H.264's
1.0 policy is not touched.

| Codec | Encoder | Containers (default first) | Default pixel format | Rate control | Pinning / options |
|---|---|---|---|---|---|
| prores | prores_ks | mov, mkv | yuv422p10le (4444: yuv444p10le) | `proresProfile` | thread_count 1 |
| vp9 | libvpx-vp9 | webm, mkv, mp4 | yuv420p | `crf` (0–63, `b=0`) or `bitrate` | threads 1, row-mt 0, deadline good, cpu-used from `preset` |
| av1 | libsvtav1 | mp4, mkv, webm | yuv420p | `crf` (1–63; FFmpeg 7.1 ignores 0) or `bitrate` | thread_count 1, `svtav1-params lp=1`, preset from `preset` |
| gif | gif | gif | pal8 | — | per-frame palette (below) |
| apng | apng | apng | rgb24 | — | — |
| png-sequence | png | files | rgb24 | — | one file per frame |
| tiff-sequence | tiff | files | rgb24 | — | packbits (encoder default) |
| exr-sequence | exr | files | gbrpf32le | — | float, zip16 |

`preset` names (x264 vocabulary) map to libvpx `cpu-used`
(ultrafast 8 … veryslow 0) and SVT-AV1 presets (ultrafast 12 … placebo 1);
the table is in `docs/xml-reference.md`.

**Container derivation.** Without `container`: 1.0 codecs use the path
extension (the 1.0 rule, unchanged); prores/vp9/av1 use the extension when
it names an allowed container, otherwise the table default; gif/apng use
their muxers; sequences use no container. With `container`, the muxer is
the attribute's and the extension is free.

**EXR** receives linear-light values: the blend-space frame is
unpremultiplied; when the project is not `linearLight` it is decoded with
an extended-range form of the working-space transfer function (the same
curve, sign-symmetric and not clamped, unlike `sr_color_decode`); linear
projects are not decoded again. It is then converted to the output gamut
and premultiplied again (EXR convention). Values are not clamped. The planes are filled directly
(G, B, R[, A]) instead of going through swscale.

**GIF palette.** Each frame is quantized on its own (frame-order safe): the
distinct colours are sorted; when there are at most 256 the palette is exact
(lossless); otherwise median cut over the sorted colour histogram (split the
box with the largest channel range at the weighted median, ties broken by
channel order and box index), palette entry = count-weighted mean rounded
half up. Pixels map to their box, found by binary search in the sorted
distinct colours. No dithering. Bounded by the frame size.

**APNG/GIF loops.** `loopCount` is the number of plays, 0 = infinite: APNG
`plays = loopCount`; GIF `loop = loopCount == 0 ? 0 : loopCount - 1`, and
`loopCount = 1` writes no loop extension (`loop = -1`).

**keyframeInterval** (seconds) sets `gop_size = max(1, round(interval *
fps))` for h264, h265, vp9 and av1. For h264/h265 the codec default stays
when the attribute is absent (1.1 scenes that exist today keep their bytes);
the schema default of 2 s is applied to vp9 and av1 only. This is a recorded
deviation from the schema default. Intra-only codecs reject the attribute.

**bFrames** sets `max_b_frames` for h264 and h265 only.

**faststart** (mp4/mov) keeps `+faststart` when true (the 1.0 behaviour) and
drops it when false; other containers reject an authored value.

## 6. Renderer changes

`renderer.c` stays the owner of the frame loop and the writer thread; the
changes are:

- `sr_render` builds the plan (`sr_output_plan_build`), validates metadata
  for every sink before any output is opened, runs each pass through the
  former body (now `render_pass`), accumulates metrics and restores the
  project.
- `SrWriteSlot` carries one buffer per sink; the writer encodes each sink
  whose range contains the frame, in sink order, then its audio block. With
  one sink the call sequence is exactly the current one.
- The first sink is converted inside `sr_render_frame` as today; other
  sinks are converted from the same float frame with their own
  `SrColorOutput`. The OpenCL conversion is used only for a pass with one
  sink whose output is a 1.0 output (today's behaviour); every other sink
  uses the CPU conversion, so a sink's bytes never depend on which other
  outputs are selected. The GPU call takes the sink's colour space.
- Audio is opened and written per sink: gif, apng and the sequences never
  get an audio stream (info diagnostic when the scene has audio); the
  others get it when the scene has audio tracks, as today. A 32-bit (EXR) conversion is a new branch
  that calls `sr_output_convert_linear` (new module).
- The encoder API gains `*_output` variants that take the `SrOutput`
  explicitly; the existing functions wrap them with `&scene->output`.
  Resume settings gain the output pointer.

The encoder becomes a small family: container encoders (unchanged path plus
new codec options), and the sequence mode, in which `drain` writes each
packet to the numbered file instead of a muxer (the codec and scaler setup
are shared).

## 7. Limits

| Limit | Value | Diagnostic names |
|---|---|---|
| `SR_MAX_OUTPUTS` | 16 outputs | `output` |
| `SR_MAX_OUTPUT_STILLS` | 16 stills per output | `poster`/`thumbnail` |
| `SR_MAX_OUTPUT_DIMENSION` | 16384 px | `output@width/height`, still `@width` |
| `SR_MAX_PASS_FRAME_BYTES` | 1 GiB of per-frame sink buffers in one shared pass (x3 slots) | `output` |
| sequence frame numbers | < 10^9 | `output@path` |
| output fps | 1 to 1000 per second | `output@fps` |
| still height | derived, 1 to 16384 | still `@width` |
| `loopCount` | ≤ 65535 (APNG `plays` / GIF `loop` bounds) | `output@loopCount` |
| GIF quantizer | frame pixels (already bounded by the frame) | — |

## 8. Determinism

- Frames are a pure function of `(scene, t)`: a pass mutates only
  `scene->project` between passes, never during frame evaluation, and
  restores it.
- Each sink sees the same bytes in the same order whatever the thread count:
  conversion is row-parallel with disjoint rows; writer order is sink order.
- Codec workers are pinned to 1 for every new codec (1.1 policy), SVT-AV1
  `lp=1`. Verified outside the renderer before the design: libvpx-vp9,
  libsvtav1 and prores_ks give identical bytes over repeated runs, and
  SVT-AV1 output is identical for lp 1/2/6.
- The GIF quantizer is a function of one frame, ordered by sorted colour
  values; no hash-map order.
- Stills use the same frame-index time mapping as video frames.

## 9. Resume fingerprint

The manifest already hashes the scene bytes, so XML attributes are covered.
For outputs that use any new attribute or codec, a new `output=` line records
the effective settings, including the pass size and rate from CLI overrides,
the selected id, container, GOP, B-frames, faststart and ProRes profile.
The line also names the libav library versions (`av_version_info()` and the
libavcodec/libavformat/libswscale version integers), since new encoders
live in them. Legacy outputs write no new line, so 1.0 manifests are
byte-identical; fingerprinting library versions for 1.0 outputs would
invalidate existing parts directories and is left to the batch-wide
fingerprint policy (recorded in the evidence file).

## 10. Tests

- Unit (`tests/unit/test_outputs.c`): pass grouping, range arithmetic
  against closed forms, selection and CLI conflicts, sequence pattern
  formatting, GIF quantizer (exact palette for ≤ 256 colours, median cut
  against an independent brute-force reference), EXR linear conversion
  against `sr_color_decode`, codec round trips with 1 vs 4 threads for every
  new codec (identical bytes), stills (PNG exact, JPEG decodes), loader
  rejections for every check in §3.
- Seeded fuzz generator for the output/still parser (valid, mutated,
  truncated attribute sets).
- OOM: plan build, output/still parsing, quantizer, sequence writer, sinks.
- Integration: every codec at 1 and 4 threads (`cmp`), sr-probe for codec,
  size, frames; a multi-output scene with shared and separate passes; hashes
  of the lossless files added to `tests/golden.sha256`.
- Golden: `tests/golden/outputs.xml` (320x180, 24 frames) with visual
  references; one still per lossless codec (png, tiff, exr sequences and
  apng) decoded and compared exactly against the reference.
- Byte oracle for the renderer and encoder refactor (309 previews, 3
  encodes, 2 rejections).

- Frame-order: the golden scene joins `tools/frame-order-check.py`
  (shuffled previews, slices, killed-and-resumed encodes at 1 and 4
  threads); the multi-output unit tests compare shared-pass sinks with the
  same output rendered alone, and sliced/resumed renders with full ones.
- A lossless GIF check: a synthetic frame with at most 256 colours
  round-trips exactly.
- Writer failure injection: a failing second sink stops the render with its
  status and leaks nothing (fault and OOM suites).

## 11. Review record

Codex design review (read-only, 11 findings) was folded in: per-sink CPU
conversion (1), effective-path collision rules (2), CLI precedence (3),
stills inside the output range (4), EXR transfer (5), SVT CRF/`lp` and
ProRes mappings (6), derived-size and fps bounds (7), loop bounds (8),
per-sink audio (9), library versions in the fingerprint (10), extra tests
(11).

## 12. Performance budget

Scenes with one output take the same path (one pass, one sink); budget < 2 %
on `benchmarks/perf-scene.xml`. The cost of the feature: a shared pass adds
one conversion and one encode per extra sink per frame; a separate pass costs
a full render; a still costs one frame render (plus physics preparation for
a stills-only pass).
