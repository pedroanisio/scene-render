# Scene format 1.1 — third and final implementation batch (proposal)

Status: accepted in principle, 2026-09-25. Input: `schema/scene-render-1.1.xsd`.
It builds on `docs/schema-1.1-batch1-proposal.md` ("B1") and
`docs/schema-1.1-batch2-proposal.md` ("B2").

## 1. Goal and definition of done

Batch 3 closes the 1.1 contract. It ends when:

1. **The capability table (B1-0) has no "unsupported in this build" entries.**
   Every element, attribute and enumeration value in the final 1.1 XSD is
   implemented. The only exceptions are items this proposal removes from the
   schema (§7), each with its reason.
2. **A schema-coverage test proves it.** A new CTest (`schema.coverage`) walks
   the XSD and lists every element, attribute and enumeration value. For each
   one it checks:
   - it has an "implemented" capability entry;
   - it appears in at least one fixture under `tests/` that is rendered (not
     only validated) by a golden, integration or unit test.

   Anything missing fails the build, so the contract and the engine cannot
   drift apart again.
3. **Every restriction that B1 and B2 placed on combinations is lifted.** That
   means physics bodies and cameras inside symbols and repeats, and included
   `physics`, `audioMix` and `lights`.
4. **`docs/xml-reference.md` documents every construct.** For each one it
   states the semantics, units, limits, and the determinism rule it follows.

## 2. Adopted decisions

The owner accepted every recommendation on 2026-09-25, on the condition that
the result is a complete and better implementation.

**From B1 §6:**
- 1.0 documents keep implicit depth cards; 1.1 documents require `threeD`.
- `%` inside a group without a size resolves against the frame box.
- GLSL `shader` types are removed from 1.1 (§7).
- The 1.1 XSD is committed together with its errata before B1-0 starts.

**From B2 §6:**
- An expression runtime error fails the render. `--expression-errors=hold` is
  available for drafts.
- `@parent` replaces the transform inherited from the XML ancestors, After
  Effects-style.
- A layer override of `text` gives the instance its own copy of the text
  asset.
- Includes are confined to the document's directory tree unless
  `--allow-external-includes` is given.
- Schema errata S6 (scoped override paths) and S9 (two-argument `prop`) go
  in together with the B1 errata.

**New in this batch.** These are recommended and adopted under the same rule.
Each is reversible until its work item starts.

| # | Decision | Why it is the better option |
|---|----------|------------------------------|
| D1 | **Stateful processing runs once before rendering and is cached.** This covers the full audio mix, optical flow, tracking/stabilisation, particles with collisions or force fields, and physics. | Keeps the rule that any frame can be rendered alone (`--preview-frame`, `--range`, resume segments). This is the model physics already uses. |
| D2 | **Temporal visual effects re-evaluate the scene at other times; they keep no history.** This covers motion blur, `echo`, `posterize-time`, `pixel-motion-blur` and `frame-mix`. | Evaluation is already a pure function of time, so correctness does not depend on the order frames are rendered in. Cost is bounded by sample counts that `quality` sets. |
| D3 | **3D shading follows the Khronos glTF Sample Viewer reference.** That reference defines the metallic-roughness model and every ratified `KHR_materials_*` extension the schema lists. A CPU rasteriser replaces today's approximate lighting. | It is a published reference we can test against, rather than an in-house approximation. |
| D4 | **One golden refresh, at the 3D milestone.** 1.0 scenes with lights or materials change once, in a single reviewed commit with before/after images. 2D output of 1.0 scenes stays byte-identical. | Maintaining two shading models would be worse. This is the "single golden refresh" planned since the performance work. |
| D5 | **Colour management is built in, with a native OpenColorIO v2 config reader.** OpenColorIO is not in the Flatpak SDK, and `flatpak-builder` is not installed. The engine implements the enumerated colour spaces and transfer functions natively and reads OCIO configs itself (§4 B3-I). Conformance comes from test vectors generated once with PyOpenColorIO. | No C++ dependency on the colour path, and the same determinism rules apply as for the rest of the engine. |
| D6 | **Dependencies come from the SDK where possible.** Otherwise they are permissively licensed C libraries vendored in `third_party/` with pinned SHA-256 and listed in `NOTICE`. No GPL additions (so no fftw3), and C++ is allowed only for USD, behind a C shim (§5). | Licence hygiene for an Apache-2.0 project; reproducible offline builds. |
| D7 | **External services stay out of the render path.** `generated` assets and caption transcription read verified caches only. Filling those caches (`scene-render resolve`) and uploading outputs (`scene-render deliver`) are separate subcommands. Providers plug in as external executables with a documented JSON protocol. Credentials come from named environment profiles and are never written to the document or to logs. | Renders stay deterministic and offline. Vendor APIs never enter the core. |
| D8 | **Lottie is imported into our scene graph**, not played by an embedded Lottie renderer. | After B1 and B2 the engine has the pieces Lottie needs: shapes, trims, repeaters, masks, mattes, text and precomps. One engine means identical blending, colour and determinism. |

## 3. Prerequisites and constraints

- B1 and B2 are merged. `perf-orders` (2.2–2.6x, byte-identical) is merged
  before B3-F and B3-K, because motion blur and PBR multiply per-frame cost on
  top of today's ~9 s per frame.
- **Build environment.** Everything builds and tests inside
  `org.freedesktop.Sdk//25.08`. SDK libraries this batch uses:
  - lcms2 2.18: ICC profiles;
  - ICU 77: line breaking;
  - HarfBuzz 11.4: `hb-paint` for COLRv1 colour glyphs, `hb-ot-math` for
    formula layout;
  - librsvg 2.62 and cairo: SVG;
  - libwebp, libavif, libjxl, libheif: stills;
  - libcurl and openssl: delivery;
  - libarchive: `.lottie` and `.usdz` containers;
  - libsamplerate and libsndfile: audio I/O;
  - libav: EXR/PSD decode, subtitle encoders (`mov_text`, `webvtt`, `ass`,
    `srt`, `ttml`), libmp3lame, libopus, flac.

  Missing from the SDK: OpenColorIO, rubberband, libebur128, glTF/FBX/USD
  loaders, a barcode library, libyaml and libssh2. SDK curl has no `sftp`.
- **Determinism.** The existing promise (bit-identical per toolchain and
  library versions, `-ffp-contract=off -fno-fast-math`) extends to every new
  dependency. Where a library's output depends on its version (ICU line
  breaking, librsvg, HarfBuzz), that version enters the resume fingerprint and
  the golden manifest.

## 4. Work items

Items are grouped into three milestones. Items within a milestone are
independent and can run as parallel lanes in separate worktrees. Sizes are
relative: M is 2–3 weeks, L 4–6, XL more than that.

### Milestone 3.1 — foundations that the later lanes consume

#### B3-A Templating, layout and safe areas (L)

- **Parameters:**
  - `variant` (named `set` and `override` lists), `bind` (with a `map` value
    table), `data` (inline or `src`; `json`, `csv`, `tsv`; `sha256` checked);
  - `param` types `list` and `asset`;
  - `repeat over`/`var`: iteration over list parameters or data rows. The
    current item is read through `param(var)`, with `item.field` access for
    rows.
  - JSON parsing uses a small in-tree strict parser; it is also used by
    tracking, charts and weights. CSV follows RFC 4180.
- **Batch rendering.**
  - `--variant id`, `--data id --row N|all`.
  - Output `path` accepts `{{param}}`, `{{variant}}` and `{{row}}`
    placeholders.
  - `output/@variant` selects the variant for that output.
  - One render pass per (variant, row, layout).
- **Layouts:**
  - `layouts/layout`: an alternate frame size with `override`s.
  - `reframe`:
    - `reflow`: relative lengths and `align*` resolve against the new frame;
    - `crop` and `fit`: render at the base frame and map with the focus
      point;
    - `fit-blur`: a blurred cover background behind the fitted frame.
  - `output/@layout` selects it.
- **Responsive anchoring:** `alignX`, `alignY`, `alignTo`
  (`parent`, `frame`, `safe-area`) and `margin`.
- **Group layout:**
  - `layout` (`row`, `column`, `stack`, `grid`), `gap`, `padding`, `justify`,
    `alignItems` (including `baseline`, from text metrics), `gridColumns`.
  - Also group `width`, `height`, `clip`, `isolate` and `collapse` (2.5D
    children share the parent's camera space).
  - Layout is recomputed per frame, because children may animate size. It is
    a pure function of the child boxes at time `t`.
- **Safe areas:**
  - The presets carry a dated, versioned inset table in
    `src/safe_area_presets.c`. The preset table version is part of the output
    manifest.
  - `enforce` checks, per frame, the boxes of text, captions and nodes tagged
    `cta` or `logo`. `warn` lists violations with element and time; `error`
    fails the render.

#### B3-B Text engine II (XL)

- **Rich text:**
  - `span` runs with `role`;
  - `textStyle` with `basedOn` chains (cycles are load errors);
  - the full `characterStyle`: `fontAsset` and font assets with
    `collectionIndex`, `weight`, `fontStyle`, `stretch`, `variation`
    (variable-font axes), `features` (OpenType tags), stroke with
    `strokePosition`, shadow, `baselineShift`, `tracking`, `textTransform`
    (`small-caps` through `smcp`, synthesised when absent), `decoration`,
    `highlight`.
- **Font fallback.** First the explicit `fallback` list, then Fontconfig
  coverage matching per cluster. This removes today's "no font fallback"
  limitation. The chosen fonts are recorded in the resume fingerprint.
- **Colour emoji** (`emoji="color"`) via the HarfBuzz paint API (COLRv0/v1),
  plus CBDT/sbix bitmaps, composited in linear light.
- **Line breaking:**
  - UAX #14 via ICU;
  - `wrap="balance"`: minimum-raggedness line lengths via dynamic
    programming;
  - `hyphenate`: Liang's algorithm with bundled permissive hyph-utf8
    patterns, selected by `language`;
  - `writingMode` `vertical-rl` and `vertical-lr` (HarfBuzz vertical
    shaping; upright or rotated by UAX #50).
- **Fitting:**
  - `autoFit` `shrink`, `grow` and `fit` by deterministic bisection over size
    within `[minSize, maxSize]`, to 1/64 px;
  - `maxLines`;
  - `overflow` `clip` or `ellipsis` (ellipsis inserted at a cluster
    boundary);
  - `background` with `backgroundMode` `block`, `line` or `word`, plus
    padding and radius.
- **`textAnimator`:**
  - units `character`, `character-no-space`, `word`, `line`, `span`;
  - `range` selector with shapes, `smoothness`, `easeHigh`/`easeLow`,
    `order`, `stagger`/`overlap`, `rangeUnits`;
  - `wiggly` selector (seeded by `random.c`);
  - `expression` selector (B2-1, which enables `textIndex` and
    `textTotal`);
  - `combine` `add`, `multiply` or `replace` across animators;
  - every property offset (transform, fill/stroke, tracking, lineSpacing,
    blur, baselineShift, characterOffset, variation, anchor).
- **The 24 presets** each expand at load into a documented animator stack
  (table in `xml-reference.md`). `scramble` draws from a seeded character
  pool per language script; `counter` animates digits with tabular figures.
- **`textPath`:** `startOffset`, margins, `reverse`, `perpendicular`,
  `forceAlignment`, using B1-4 arc length.

#### B3-E Paints, vectors, deformation and rigging (L)

- **Paints.**
  - `meshGradient`: bicubic Coons patches interpolated in `oklab`,
    `linear` or `srgb`; points animate.
  - `pattern`: tiling from an asset, with offset, rotation and scale.
  - `oklch` gradient interpolation, finalised using CSS Color 4 hue rules
    (shorter arc).
- **Vector assets.** `shape="svg"` through librsvg and cairo. The SVG is
  rasterised at the layer's effective scale (cached per power-of-two scale
  bucket), with no scripts and no external resources. Output is converted
  into the engine's float premultiplied working space.
- **Shape modifiers:**
  - `repeater` (copies, offsets, `startOpacity`/`endOpacity`, `composite`);
  - `offset-path` (from the B1 stroker's join code);
  - `pucker-bloat`, `zig-zag` (`corner`/`smooth`), `twist`, `round-corners`,
    `wiggle-path` (seeded);
  - `merge` (`add`, `subtract`, `intersect`, `exclude`): an in-tree Vatti
    polygon clipper on flattened paths, with a documented tolerance of
    1/256 px;
  - `trim` sequential across paths.
- **Deform modifiers:**
  - `bulge`, `pinch`, `spherize`, `ripple`, `turbulence` (seeded noise) and
    `corner-pin`, on the existing inverse grid warp;
  - `puppet`: As-Rigid-As-Possible deformation on a triangulation of the
    layer's alpha, with a fixed iteration count so it is deterministic.
    `position`, `bend` and `starch` pins are supported.
  - `skin`: linear blend skinning from `skeleton`/`bone`.
    `skeleton/@weights` is a JSON file of per-vertex bone weights (schema in
    `xml-reference.md`); without it, weights come from distance to the
    bones.
- **Constraints.** `transformConstraint type="ik"`: analytic two-bone IK with
  `bendPositive`, on bones and nodes.
- **Generator asset.** `solid`, `gradient`, `noise`, `fractal-noise`,
  `cells` (Worley), `checkerboard`, `grid`, `stripes`, `film-grain` and
  `light-rays`, all on `random.c`, with every parameter animatable.

#### B3-I Colour management and HDR (XL)

- **Colour spaces.** Every `colorSpaceType` is implemented natively:
  - primaries, white point and matrices (Bradford chromatic adaptation);
  - `raw` passthrough;
  - `xyz-d65`;
  - the ACES spaces (AP0/AP1, ACEScct, ACEScc).
- **Transfer functions.** Every `transferType`:
  - sRGB, BT.1886, gamma 2.2/2.6, PQ (ST 2084), HLG (BT.2100);
  - S-Log3, LogC3, LogC4, V-Log, C-Log3, REDLog3G10, F-Log2, N-Log,
    ACEScc, ACEScct;
  - the formulas come from the vendors' published white papers, cited per
    curve.
- **Asset decode.** Each asset's `colorSpace` and `transfer` decode into
  `colorManagement/@workingSpace`, which replaces `project/@workingColorSpace`;
  the latter stays as an alias. ICC-tagged stills convert through lcms2.
- **Looks.** ASC CDL (`slope`, `offset`, `power`, `saturation`) and LUT
  files, applied in the look's `space`:
  - `.cube` (1D/3D) and `.3dl`;
  - `.clf` and `.ctf` (Common LUT Format, parsed with libxml2);
  - `.spi1d` and `.spi3d`.
- **Output side.** Display and view transforms, and `toneMapping`:
  - `aces`: ACES 1.3 RRT + ODT, ported from the CTL reference;
  - `aces2`: ACES 2.0 output transform, ported from the reference
    implementation;
  - `agx`, `filmic`, `reinhard`.
  - Also `exposure`, and the `tonemap` effect.
- **Working precision.** `bitDepth` quantises intermediate buffers on store
  (8, 16, 16f; 32f is today's behaviour). This lets drafts reproduce
  delivery banding.
- **OCIO v2.** `ocioConfig` is read by an in-tree reader, with a vendored
  libyaml for the YAML. It supports:
  - colour spaces, roles, displays/views, view transforms, looks, file rules,
    environment and context variables;
  - `MatrixTransform`, `FileTransform`, `ExponentTransform`,
    `ExponentWithLinearTransform`, `LogTransform`, `LogAffineTransform`,
    `LogCameraTransform`, `CDLTransform`, `RangeTransform`,
    `ExposureContrastTransform`, `FixedFunctionTransform`, `GroupTransform`,
    `ColorSpaceTransform`, `DisplayViewTransform`, `LookTransform`;
  - `BuiltinTransform`: the ACES and camera built-ins used by the ACES
    studio config.
  - Any transform outside this list is a load error naming it.
  - Conformance: checked-in test vectors produced once by PyOpenColorIO for
    the ACES studio and CG configs. The tolerance is 1e-5 relative for
    matrix/log paths and 1e-3 for LUT paths.
- **HDR output:**
  - `transfer` `pq` and `hlg` with 10/12-bit pixel formats;
  - `masteringDisplay` SEI and container side data;
  - `maxCLL` and `maxFALL`: computed from the rendered frames when absent,
    in a precompute pass per D1.
- **Layered images.** Image `layer`: PSD layers through an in-tree reader
  (RLE and raw channels, blend flags, with an error on adjustment layers),
  and EXR parts and layers through libav's `part`/`layer` options.

### Milestone 3.2 — the media pipeline

#### B3-C Transitions (L)

- **All 34 non-shader types.** The algorithm for each is specified in
  `xml-reference.md`:
  - `cut`, `crossfade`, `additive-dissolve`, `dip-to-color`;
  - `wipe`, `slide`, `push`, `cover`, `reveal`;
  - `zoom-in`, `zoom-out`, `spin`, `whip-pan` (directional blur);
  - `circle-open`, `circle-close`, `iris`, `clock-wipe`, `radial-wipe`,
    `barn-door`, `blinds`, `stripe`;
  - `luma` (from the `matte` node), `blur`, `glitch`, `pixelize`;
  - `flip`, `cube`, `page-curl`, `film-roll`, `carousel`: on depth cards and
    mesh warps;
  - `squash`, `shuffle`;
  - `light-leak` (generator);
  - `morph`: an optical-flow warp from B3-J.
- **Timing.** `alignment` and handle extension: the outgoing and incoming
  nodes are evaluated past their `end` or before their `start` by the needed
  amount, clamped to their media. `curve`, `direction`, `angle`, `color`,
  `softness`.
- **Motion blur.** `motionBlur` on moving transitions, via B3-F.
- **Audio.** `audio` sets how the two nodes' audio is joined across the
  transition: `crossfade`, `equal-power`, `cut` or `none` (B3-G).
- **Sequences.** `sequence/@transition` and `@transitionDuration` insert a
  transition at every junction that has no explicit one.
- **Single-node transitions.** A transition with only `from` or only `to`
  gives the node an in or out transition.

#### B3-D Effects catalogue (XL)

- **Every remaining effect type.** Each has a unit test against a reference
  formula and appears in a golden sheet per family:
  - **Blur:** `directional-blur`, `radial-blur`, `zoom-blur`;
    `lens-blur` with aperture blade shapes from `samples` and `angle`;
    `pixel-motion-blur` (B3-J flow); `tilt-shift`.
  - **Colour:** `lift-gamma-gain`, `cdl`, `lut`, `curves` (monotone cubic
    per `channel`), `levels`, `white-balance` (temperature/tint on the
    Planckian locus), `exposure`, `hue-saturation`, `tonemap`, `tint`,
    `tritone`, `gradient-map`, `grayscale`, `sepia`, `invert`, `posterize`,
    `threshold`, `color-overlay`, `gradient-overlay`, `selective-color`.
  - **Stylise:** `film-grain` (seeded, size-aware), `noise`,
    `chromatic-aberration`, `sharpen`, `unsharp-mask`, `halation`,
    `light-leak`, `light-sweep`, `glitch` (seeded), `rgb-split`,
    `scanlines`, `vhs`, `halftone`, `pixelate`, `mosaic`, `emboss`,
    `bevel`, `inner-shadow`, `inner-glow`, `long-shadow`, `stroke` and
    `outline` (with `position`), `letterbox`, `mirror`, `kaleidoscope`,
    `tile`.
  - **Temporal** (D2): `echo` and `posterize-time`.
  - **Distort:** `displacement-map`, `turbulent-displace`, `wave-warp`,
    `ripple`, `twirl`, `spherize`, `bulge`, `lens-distortion`, `heat-haze`.
  - **Keying:** `chroma-key` (YCbCr distance with `tolerance` and
    `softness`), `luma-key`, `difference-key`, `spill-suppress`,
    `matte-choke`.
  - **Generative:** `fill`, `fractal-noise`, `god-rays`.
- **Shared plumbing:**
  - `mix`, `paint`, `compositeOriginal`, `position`, `centerX`/`centerY`,
    `samples`, `seed`, `speed`;
  - `source`: any node rendered into a buffer in the effect's space, reusing
    the B1 matte path. It provides displacement maps, gradient-map sources
    and difference-key plates. Dependency cycles through `source` are load
    errors.

#### B3-F Motion blur and physical camera (L)

- **Motion blur** (D2):
  - project `motionBlur`, `shutterAngle`, `shutterPhase`,
    `motionBlurSamples`;
  - per-node `motionBlur` (`inherit`, `on`, `off`);
  - camera `shutterAngle` overrides the project's.
  - The renderer evaluates the scene at N sub-frame times and accumulates in
    linear light; sub-frame times are stratified and fixed per frame.
  - `adaptiveMotionBlur` lowers N for nodes whose screen-space motion
    (measured from their transform at the shutter edges) is under half a
    pixel.
  - Depth cards and 3D objects blur through the same path.
- **Physical camera:**
  - `focalLength`, `sensorWidth`/`sensorHeight` (fov derived from them),
    `orthoHeight`, `exposure` (EV), `lensDistortion` (Brown–Conrady k1 with
    the inverse applied at render);
  - `depthOfField` with `fStop`, `focusDistance`/`focusTarget` and
    `apertureBlades` (polygonal bokeh), replacing today's `aperture`, which
    remains as an alias.
- **Camera switching.** The active camera at time `t` is the last active
  camera whose `[start, end)` contains `t`. `shake` uses seeded fBm on
  position, rotation and zoom within its own time window.
- **`project/@quality`.** A table of sample counts per level, applied to
  motion blur, lens blur, DoF, `antialias3d` and flow resolution:
  - `final` uses the document's values;
  - `preview` halves them;
  - `draft` quarters them and uses proxy representations.

  Each level is deterministic in its own right, and the quality level enters
  the resume fingerprint.

#### B3-G Audio engine II (XL)

- **Full timeline render at prepare time** (D1).
  - The complete mix is rendered once into a chunked float PCM cache, keyed
    by the audio fingerprint.
  - Encoding and `--range` read slices of it, so stateful DSP (compressors,
    reverbs, delays, ducking, loudness) stays range-independent.
  - This replaces the stateless per-block mixer contract with a stronger one:
    the output is identical to rendering the whole timeline.
- **Bus graph.**
  - `bus` elements with `output` routing to other buses or the master; cycles
    are load errors.
  - `volume`, `gain`, `pan` and `mute` on tracks and buses, all automatable.
  - Tracks from video layers (`audioBus`), transitions, symbols and includes
    join the graph.
  - Included `audioMix` elements become a sub-bus named after the include
    (lifting the B2-5 restriction).
- **The 16 `audioEffect` types:**
  - `eq` (with `band` kinds, RBJ biquads), `highpass`, `lowpass`;
  - `compressor`, `limiter`, `gate` (`threshold`, `ratio`, `attack`,
    `release`, `knee`, `sidechain` from any track or bus), `de-esser`;
  - `reverb`: FDN reverb with a fixed seed;
  - `delay`, `chorus`, `pitch-shift`;
  - `noise-reduction`: spectral gating from a noise profile learned over the
    track's quietest 500 ms;
  - `stereo-width`, `gain`, `distortion`, `telephone`.
- **Ducking.** `duckUnder`, `duckAmount`, `duckThreshold`, `duckAttack` and
  `duckRelease` on tracks and buses. They use an RMS envelope follower over
  the full timeline.
- **Tracks:**
  - `fadeCurve` (5 shapes), `startMarker`, `role`, `language`;
  - `speed` with `preservePitch`, done by a phase-vocoder time stretch with
    phase locking, using vendored pocketfft (BSD). Without `preservePitch`
    the track is resampled with libsamplerate;
  - `fitToDuration`: trim or loop at bar boundaries from asset `bpm`, with
    10 ms equal-power seams.
- **Master:**
  - `normalize="integrated"`: an in-tree ITU-R BS.1770-4 meter measures the
    whole mix, then a static gain is applied;
  - `normalize="dynamic"`: EBU R128 short-term levelling with a 3 s window;
  - true-peak limiting to `truePeak` with 4x oversampling; `limiter`;
  - `dither`: seeded TPDF at `bitDepth`.
- **Channel layouts.** `mono` through `7.1.4`, panned with VBAP.
  `ambisonic-1` and `ambisonic-3` encode track positions as SN3D/ACN, and 360
  outputs write the SA3D box.
- **Audiogram asset.** `bars`, `line`, `wave`, `circle` and `spectrum` from
  the prepared mix or a track, using per-frame FFT magnitudes with
  `smoothing` applied as a symmetric window (stateless).
- **Output audio streams.** Tracks with `language` produce one audio stream
  per language when `output/@audioLanguages` is set (errata E7); otherwise
  all tracks mix into one stream.

#### B3-H Captions, markers and accessibility (L)

- **Caption sources:**
  - inline `cue` and `word` elements;
  - `src` files in `srt`, `vtt`, `ass` (styles mapped to text styles),
    `ttml`, `itt` and `scc` (CEA-608 decoded to cues);
  - `transcribe`: read from `cache` with `cacheSha256`, and produced by
    `scene-render resolve` (D7).
- **Burn-in.**
  - All 13 presets, built on B3-B text animators (`karaoke`, `highlight`,
    `one-word` and so on use word timings).
  - `style`, `activeStyle`, `activeColor`.
  - Line building honours `maxWordsPerLine`, `maxCharsPerLine`, `maxLines`
    and UAX #14.
  - Placement: `x`, `y`, `width`, `safeArea`, `z`.
- **Sidecar and embedded captions.**
  - `mode="sidecar"` writes the track in its source format, or WebVTT for
    inline cues, next to each output.
  - `output/@captions` embeds `mov_text` or WebVTT streams in the container.
  - `burnCaptions` selects the burned track.
  - `profanityFilter` masks words from a bundled per-language list
    (LDNOOBW, CC-BY-4.0, credited in `NOTICE`), with a `--profanity-list`
    override.
- **Markers.** `kind="chapter"` writes container chapters. `todo` and
  `comment` markers are listed by `--validate`. `cta` markers feed safe-area
  enforcement.
- **Accessibility:**
  - `flashCheck` analyses the rendered frames (D1) for general and red
    flashes. A flash is a luminance transition pair ≥ 10 % with the darker
    state < 0.8, or the red-saturated transition, per WCAG 2.3.1 and ITU-R
    BT.1702. The area threshold is 25 % of a 10° field, scaled from
    1024×768. More than 3 per second in any 1 s window triggers `warn` or
    `error`, with the time ranges.
  - `contrastCheck` samples, per frame, the background behind each burned
    text and caption glyph mask, and checks the WCAG contrast ratio against
    `minContrast`.
  - `requireCaptions` fails when an output has no caption track in its
    language.
  - `audioDescription` becomes a separate audio stream flagged
    `visual_impaired`.
  - `description` is embedded in the container metadata.

#### B3-J Analysis-driven media (L)

- **Optical flow.** A deterministic CPU DIS flow (dense inverse search,
  fixed pyramid and iteration counts), computed at prepare time and cached
  per source frame pair (D1). It is used by:
  - `frameBlend="optical-flow"` on layers and `timeRemap`;
  - `pixel-motion-blur`;
  - the `morph` transition.
- **Stabilisation.** `stabilize` and `stabilizeSmoothness`: KLT feature
  tracking plus a similarity fit, smoothed with a Gaussian over the whole clip
  (zero phase, so it is stateless at render time), then compensated with
  auto-scale to hide the borders. Cached per asset.
- **`tracking/trackData`.**
  - Import formats: `json` (documented schema), `csv`, `nuke` (Tracker4 and
    CameraTracker `.nk` knobs), `after-effects` (keyframe clipboard text),
    `mocha` (AE corner-pin and shape exports), and `fbx` cameras via ufbx.
  - `footage` and `timeOffset` align the data to a layer.
  - How each kind is used:
    - `point`: `transformConstraint type="track"`;
    - `planar`: a `corner-pin` modifier driven by the track;
    - `camera`: drives a `camera` through `transformConstraint`;
    - `mask`: an animated path mask;
    - `face`: named landmark points, addressable as `point`.

### Milestone 3.3 — 3D, simulation, new assets and delivery

#### B3-K 3D renderer II (XL)

- **Shading** (D3). A CPU rasteriser with per-pixel PBR from the Khronos glTF
  Sample Viewer formulas:
  - metallic-roughness and `alphaMode`;
  - `doubleSided`, `unlit`, clearcoat, transmission (screen-space refraction
    with `ior` and `thickness`), volume attenuation, sheen, specular,
    iridescence, anisotropy, dispersion, emissive strength;
  - texture maps: base colour, normal, metallic-roughness, occlusion,
    emissive and displacement, with UV scale.
- **Lights:**
  - `rect-area`, `disk-area` and `sphere-area` via linearly transformed
    cosines;
  - `dome`: an equirectangular HDRI, prefiltered at load with a fixed sample
    pattern, plus `environmentVisible`;
  - IES LM-63 profiles, `colorTemperature` (Kelvin to linear RGB),
    `exposure`, `innerConeAngle`;
  - `shadowSoftness` (PCSS), `shadowBias`, `roll`, `affectsDiffuse`,
    `affectsSpecular`.
- **Geometry:**
  - primitives `cylinder`, `cone`, `torus`, `capsule`;
  - `text` (glyph outlines from B3-B, extruded with `depth` and `bevel`);
  - `extrude` of an SVG path;
  - `segments`, `width`, `height`.
- **Mesh formats:**
  - glTF/GLB via cgltf (MIT): skins, morph targets, animations
    (`animationClip`, `animationSpeed`, `animationOffset`), `morphWeights`,
    and `KHR_materials_variants` (`materialVariant`);
  - FBX via ufbx (MIT or public domain);
  - PLY;
  - OBJ (existing);
  - Gaussian splats (`.ply` and `.splat`): a CPU splat rasteriser with a
    stable depth sort, and ties broken by index;
  - USD/USDZ via TinyUSDZ (Apache-2.0), the only C++ component, behind a C
    shim (§5).
- **MaterialX.** `materialX` documents are parsed with libxml2, for
  `open_pbr_surface`, `standard_surface` and `gltf_pbr` surface shaders whose
  inputs are constants or `image` nodes. They map onto the material model.
  Other node graphs are load errors that name the node, and the schema
  documentation states this subset (errata E6).
- **Object3D:**
  - `instances` means N copies, with `index`/`count` available to
    expressions (errata E8);
  - `condition`, `parent`, `motionBlur` (B3-F), `start`/`end`, `opacity`,
    `name`.
- **Golden refresh** (D4) at the end of B3-K.

#### B3-L Physics and particles II (L)

- **Rigid bodies:**
  - shapes `capsule`, `polygon`, `path` and `convex-hull` (from alpha, with
    the quickhull input quantised to 1/16 px);
  - `collisionGroup`, `collidesWith`, `sensor`, `fixedRotation`, `bullet`
    (continuous collision), `activateAt`.
- **Soft bodies.** `kind` `jelly`, `cloth` or `rope`, and `selfCollision`.
- **Force fields:**
  - `turbulence`, `drag`, `wind`, `attractor-path`;
  - `radius`, `scale`, `path`, `seed`, `start`/`end`;
  - `affects` bodies, particles or both.
- **Constraints.** `rope`, `hinge`, `slider`, `weld` and `motor`, with angle
  limits, `axisAngle`, `motorSpeed`, `maxForce` and `breakForce` (a broken
  constraint stays broken, in the precomputed state).
- **World:** `pixelsPerMeter`, `solverIterations`, `start`, `bounds`
  (`frame`, `floor`), `cacheSha256`.
- **Scoping.** Bodies inside symbols, repeats and includes get scoped ids and
  share one world, which lifts the B2 restriction.
- **Particles:**
  - the remaining presets;
  - `drag`, `turbulence`/`turbulenceScale`, `sizeVariance`, `sizeCurve`,
    `colorCurve`, `opacityEnd`;
  - rotation and angular velocity with variance, `orientToVelocity`;
  - emitter shapes `point`, `ellipse`, `line`, `path` and `asset-alpha`;
  - particle shapes `sprite` (sheet columns, rows and fps) and `streak`;
  - `trail`, `burst`, `preroll`, `forceFields`, `collide`/`bounce`.
  - Emitters without `forceFields`, `turbulence` or `collide` keep today's
    closed-form model. The others are simulated once and cached like physics
    (D1).

#### B3-M New asset kinds (XL)

- **`lottie`** (D8). JSON and `.lottie` (libarchive) are imported at load
  into symbols. Supported:
  - shapes, fills, strokes, gradients, trims, repeaters, merges, offsets;
  - masks, mattes, precomps, time remap, text, images;
  - `slot` overrides and `segment` markers;
  - `animation` selects within a `.lottie` file.

  Lottie JavaScript expressions and unsupported features are load errors
  that list the feature and the Lottie layer.
- **`chart`.**
  - `bar`, `column`, `line`, `area`, `pie`, `donut`, `scatter`, `counter`,
    `progress`.
  - Data from `series` or from `src` (CSV or JSON, B3-A parser).
  - `labels`, `showAxes`, `showValues`, `textStyle`.
  - `format`: a documented number-format mini-language (grouping, decimals,
    prefix/suffix, percent, compact).
  - `progress` draws the chart on.
  - Rendered with the engine's own vectors and text.
- **`code`.** `qr`, `datamatrix`, `pdf417`, `ean13`, `upc-a` and `code128`,
  via vendored libzint (BSD-3). Module edges snap to whole pixels, and the
  quiet zone is enforced; a code that cannot fit its box is a load error.
- **`formula`.** A TeX math subset laid out with the HarfBuzz OpenType MATH
  API and a bundled STIX Two Math font (OFL):
  - fractions, roots, scripts, big operators with limits;
  - stretchy delimiters, accents, matrices and cases;
  - spacing commands, `\text`, Greek letters and symbols.

  Unknown commands are load errors. The subset is listed in
  `xml-reference.md`.
- **`generated`.** At render time the renderer reads `cache` only, verifies
  `cacheSha256`, and decodes the file according to `kind` (image, video, or
  speech/music/sound-effect as audio). `scene-render resolve` fills missing
  caches through provider executables (D7). `license` goes into the output
  metadata.
- **Provenance.** `assetProvenance` (`license`, `credit`, `proxy`) is
  recorded in the output manifest, and `credit` also goes into the metadata.

#### B3-N Outputs, 360 and delivery (L)

- **Codecs and containers:**
  - `dnxhr` (profiles by `profile`);
  - `webp` animated;
  - `audio-only` to `wav`, `m4a` or `mp3`;
  - containers `mov`, `mkv`, `webm` and `mxf`, with codec/container
    compatibility checked at load.
- **Stills.** `webp` and `avif` for `poster` and `thumbnail`, and a `marker`
  time.
- **Alpha outputs.** ProRes 4444/4444XQ, VP9 `yuva420p`, FFV1, PNG, TIFF and
  EXR sequences, and WebP.
- **Rate control:**
  - `twoPass`, `maxBitrate`, `bufferSize`, `profile`, `level`;
  - `maxFileSize`: a target bitrate from size, duration, audio and container
    overhead, then a verify-and-retry loop (at most 3 passes) that reports the
    final size.
- **Output properties:**
  - `fps` override (the scene is re-rendered at that rate, never
    frame-dropped);
  - `pixelAspect` (sample aspect ratio);
  - `timecodeStart` (timecode track);
  - `embedMetadata`.
- **360:**
  - `scene360/@layout`: `cubemap` (3×2), `eac`, `fisheye-180`;
  - `stereo` `top-bottom` or `left-right`, using omni-directional stereo
    with `interpupillary`;
  - spherical metadata v2 (`st3d`, `sv3d` with `cbmp`/`equi`/`mesh`
    projections);
  - viewport extraction supports every layout.
- **Delivery.** `scene-render deliver` (D7) runs after a successful render,
  per `output/destination`:
  - `file`;
  - `http-put` and `webhook` (JSON manifest POST) via libcurl;
  - `s3`, `gcs` (XML API with HMAC keys) and `azure-blob` (SAS or SharedKey),
    signed in-tree over openssl;
  - `sftp` via vendored libssh2 (BSD-3), with host-key pinning required.
  - `credentials` names an environment profile (`SR_CRED_<NAME>_*`, or a
    0600 file under `~/.config/scene-render/credentials/`).
  - Uploads are verified by checksum where the service supports it. Retries
    are bounded. Secrets are redacted from all output.

## 5. New dependencies

| Library | Licence | Source | Used by |
|---|---|---|---|
| lcms2, ICU, librsvg + cairo, libwebp, libavif, libjxl, libheif, libcurl, openssl, libarchive, libsamplerate, libsndfile | various permissive (MIT, BSD, LGPL-2.1+ as shared libraries) | Flatpak SDK | B3-I, B3-B, B3-E, B3-N, B3-M, B3-G |
| cgltf | MIT | vendored, single header | B3-K |
| ufbx | MIT / public domain | vendored, single file | B3-K, B3-J |
| TinyUSDZ | Apache-2.0 | vendored C++17, static, behind a C shim | B3-K |
| libzint | BSD-3 | vendored | B3-M |
| libyaml | MIT | vendored | B3-I |
| libssh2 | BSD-3 | vendored, uses SDK openssl | B3-N |
| pocketfft (C) | BSD-3 | vendored | B3-G, B3-J |
| hyph-utf8 patterns (permissive subset only) | MIT / LPPL-free subset | vendored data | B3-B |
| STIX Two Math | OFL-1.1 | vendored font | B3-M |
| LDNOOBW word lists | CC-BY-4.0 | vendored data | B3-H |

**Rules for vendored code:**
- Every vendored item is pinned by SHA-256 in `third_party/MANIFEST`.
- It builds with our warning flags, or with a documented per-target
  exception.
- It runs under ASan and UBSan in CI and is covered by the OOM injector
  wherever it lets us supply allocators.

**Libraries not used:**
- fftw3: GPL.
- OpenColorIO: not in the SDK, and C++ with a heavy dependency tree.
- rlottie and ThorVG: they would be a second renderer (D8).
- rubberband: GPL.

## 6. Sequencing

1. **Schema errata, then B1 and B2.** Apply every errata item (§7) in one
   schema commit before B1-0.
2. **Milestone 3.1:** A, B, E and I in parallel. Text, colour and vectors are
   consumed by nearly everything after them.
3. **Milestone 3.2:** C, D, F, G, H and J in parallel.
   - C needs F and J.
   - H needs B and G.
   - D needs I for `lut` and `tonemap`, and J for `pixel-motion-blur`.
4. **Milestone 3.3:** K, L, M and N in parallel.
   - K includes the golden refresh.
   - M needs B, E and A for Lottie, charts and formulas.
   - N needs G and H for audio and caption streams.
5. **Close-out.**
   - Turn on `schema.coverage` as a required test.
   - Remove the "unsupported" code path from the capability table. The
     table stays as the source of truth for `--validate` reports.
   - Complete `xml-reference.md`, `feature-matrix.md` and a 1.1 migration
     guide for 1.0 scenes.

Each lane follows the port-phase discipline:
- a Codex diff review;
- every finding reproduced as a failing test, then fixed;
- frame-order independence and thread invariance checked for every new
  golden;
- `make perf-check` against a budget recorded per lane.

## 7. Consolidated schema errata

These are applied in one commit before B1-0 (E1–E14), and they supersede the header
claims of the current draft.

| # | Change | Origin |
|---|--------|--------|
| E1 | Re-add node `depth` and camera `zoom`/`aperture` as deprecated aliases (of `zDepth`, the fov scale and `fStop`). 1.1 becomes a true superset of 1.0. | B1 S1 |
| E2 | Replace the reference to the missing `scene-render-1.1.sch` with a reference to the semantic rules in `docs/xml-reference.md`. | B1 S3 |
| E3 | `overrideType/@target` takes the scoped-path pattern `[A-Za-z_][A-Za-z0-9_.\-]*(/[A-Za-z_][A-Za-z0-9_.\-]*)*`. | B2 S6 |
| E4 | Document the two-argument `prop(id, property)`, the per-instance layer `text` override, and a pointer to `docs/expressions.md`. These are documentation changes only. | B2 S8, S9, S10 |
| E5 | Remove `effect type="shader"`, `transition type="shader"` and `transition/@shader`. A CPU engine cannot run arbitrary GLSL deterministically, and there is no GPU test environment. They are reserved for a later format version. | B1 §6 Q3 |
| E6 | Document the MaterialX subset (B3-K), the OCIO transform subset (B3-I), the TeX subset (B3-M) and the Lottie feature subset (B3-M) in the relevant annotations. | this batch |
| E7 | Add `output/@audioLanguages` (`xs:NMTOKENS` of language tags): one audio stream per listed language, taken from tracks with that `language`. Without it, the per-track `language` attribute would have no effect on the output. | this batch |
| E8 | Define `object3D/@instances` (N copies with `index`/`count`) and `skeleton/@weights` (JSON format), which the current draft names without semantics. | this batch |
| E9 | Document that `destination/@credentials` names an environment profile, never a secret value, and that safe-area preset insets come from a dated table in the renderer. | this batch |
| E10 | Define `startMarker`/`endMarker`: with a marker, `@start`/`@end` become offsets from the marker time, as `key/@marker` already defines for `@time`. Also state that generated beat ids are `beat.N`/`bar.N` counted from 0 at the grid offset. The draft uses both without saying what they mean. | trailer review, 2026-09-25 |
| E11 | Add `startMarker`/`endMarker` to `camera`, and `@marker` to `shake` and `burst`. Today these are the only timed elements that cannot follow the edit's markers, so a retimed cut leaves camera windows, shakes and debris bursts behind. | trailer review, 2026-09-25 |
| E12 | Add `group/@camera` (IDREF of a camera): a 2.5D group is filmed by that camera rather than the globally active one. Without it, a dissolve between two shots filmed by different cameras cannot be expressed; the best a document can do is cut after the dissolve. | trailer review, 2026-09-25 |
| E13 | Define `{{var}}` and `{{var.field}}` in text for repeats: inside a `repeat over=… var=…`, text placed by the repeat (directly or through symbol instances) substitutes the current row, using a per-copy text clone as for the B2 S8 override. A placement outside any such repeat is a load error. Today only `param(var)` in expressions is defined, so per-row card text has no defined meaning. | explainer review, 2026-09-25 |
| E14 | Add `output/@display` and `output/@view` (defaulting to the output's colour space and transfer), with `colorManagement/@display`/`@view` as the fallback. One document-wide display cannot serve SDR Rec.709, sRGB and PQ HDR deliverables at once. | explainer review, 2026-09-25 |

## 8. Verification and definition of done

- **`schema.coverage`** (§1) passes with zero gaps.
- **All goldens are frame-order independent and thread invariant.** This
  covers sequential rendering, shuffled previews, `--range` slices and killed
  and resumed segments, at 1 and 4 threads. It includes the new golden sheets
  per effect family, per transition group, per text feature, per colour
  path, per 3D feature and per asset kind.
- **Reference conformance suites:**
  - PyOpenColorIO test vectors (B3-I);
  - the Khronos glTF sample models against Sample Viewer renders, within a
    documented ΔE tolerance, as a report-only check (B3-K). Our own goldens
    stay byte-exact.
  - the EBU R128 / BS.1770 compliance signals (EBU Tech 3341/3342) for the
    loudness meter (B3-G);
  - ITU/Harding-style flash test clips for `flashCheck` (B3-H);
  - the Lottie test corpus (lottie-web samples within the supported subset)
    (B3-M).
- **Fuzzing.** Seeded generators for every new parser run under ASan and
  UBSan in CI: expressions, caption formats, tracking formats, LUT formats,
  OCIO YAML, PSD, Lottie JSON, TeX and data files. The OOM injector covers
  every in-tree parser.
- **Coverage gate.** The floors are re-measured after milestone 3.1 and
  raised, never lowered. Vendored code is excluded from the metric but not
  from the sanitizers.
- **Compatibility.** 1.0 scenes stay byte-identical in 2D. The D4 refresh for
  3D is the only change, reviewed image by image. The 1.1 migration guide
  lists every alias from E1 and every behaviour that changed.
- **Documentation.** `xml-reference.md` gains a section per construct;
  `feature-matrix.md` is regenerated from the capability table, so it cannot
  go stale; `NOTICE` lists every vendored item and data set.

## 9. Risks

| Risk | Mitigation |
|---|---|
| **Scope.** Batch 3 is larger than B1 and B2 together. | Three milestones of independent lanes. Each lane ends in a mergeable, fully tested state behind the capability table, so a partial milestone never leaves main broken. |
| **Performance** of PBR, motion blur, flow and temporal effects. | Merge `perf-orders` first. Set a per-lane `perf-check` budget. `quality` levels bound sample counts. Precomputed analyses are cached. |
| **Library version drift** (ICU, HarfBuzz, librsvg, libav) changes output. | Library versions go into the resume fingerprint and the golden manifest. The determinism promise stays per toolchain, as documented. |
| **Safe-area presets go stale** as platforms change their UI. | A dated table, versioned in the output manifest. Explicit insets override the presets. |
| **The C++ component** (TinyUSDZ). | Isolated static library with a C shim, fuzzed, and removable without affecting anything outside `usd`/`usdz`. |
| **External services** in `resolve` and `deliver`. | Kept out of the render path entirely (D7). Credentials by profile name only. Redaction is tested. |

## Appendix A. Coverage of every schema type

Every named type in `schema/scene-render-1.1.xsd` (172 in all) is assigned to the
work items that implement it. The table was generated by a script that fails
if any type is left unassigned. `schema.coverage` (§1) checks the same thing at
the level of individual attributes and enumeration values once the batches have landed.

| Schema type | Implemented by |
|---|---|
| `positiveDecimal` | B1-0 (loader value parsing); consumers listed per type below |
| `unitDecimal` | B1-0 (loader value parsing); consumers listed per type below |
| `signedUnitDecimal` | B1-0 (loader value parsing); consumers listed per type below |
| `percentType` | B1-0 (loader value parsing); consumers listed per type below |
| `durationType` | B1-0 (loader value parsing); consumers listed per type below |
| `lightIntensityType` | B1-0 (loader value parsing); consumers listed per type below |
| `spotAngleType` | B1-0 (loader value parsing); consumers listed per type below |
| `effectOffsetType` | B1-0 (loader value parsing); consumers listed per type below |
| `effectRadiusType` | B1-0 (loader value parsing); consumers listed per type below |
| `nonNegativeDecimal` | B1-0 (loader value parsing); consumers listed per type below |
| `decibelType` | B3-G |
| `lufsType` | B3-G |
| `kelvinType` | B3-K (lights), B3-D (white-balance) |
| `gridSizeType` | B1-0 (loader value parsing); consumers listed per type below |
| `fpsType` | B1-0 (loader value parsing); consumers listed per type below |
| `aspectType` | B3-A (layouts) |
| `sha256Type` | B1-0 (loader value parsing); consumers listed per type below |
| `timecodeType` | B2-8 (video timecodeStart), B3-N (project/output timecode) |
| `languageTagType` | B1 (existing text), B3-B, B3-G, B3-H |
| `numberListType` | B1-0 (loader value parsing); consumers listed per type below |
| `pointType` | B1-0 (loader value parsing); consumers listed per type below |
| `relativeLength` | B1-2 |
| `positiveRelativeLength` | B1-2 |
| `lengthType` | B1-2 |
| `positiveLengthType` | B1-2 |
| `colorType` | B1-2 (tokens); 1.0 forms existing |
| `paintRefType` | B1-4 (gradients), B3-E (mesh/pattern) |
| `paintType` | B1-4 (gradients), B3-E (mesh/pattern) |
| `colorSpaceType` | B3-I (1.0 spaces existing) |
| `transferType` | B3-I (1.0 spaces existing) |
| `alphaModeType` | B2-8 |
| `blendType` | B1-3 |
| `curveType` | B1-1 |
| `extrapolationType` | B1-1 |
| `fitType` | B2-8 (layers), B2-4 (instances) |
| `matteModeType` | B1-3 |
| `triStateType` | B3-F |
| `strokeCapType` | B1-4 |
| `strokeJoinType` | B1-4 |
| `strokePositionType` | B1-4 |
| `fillRuleType` | B1-4 |
| `expressionString` | B1-0 (loader value parsing); consumers listed per type below |
| `assetProvenance` | B2-8 (sha256), B3-M (license/credit/proxy) |
| `strokeStyle` | B1-4 |
| `trimPath` | B1-4 |
| `paramValueType` | B3-C (transition params), B3-G (audio effect params), B3-D |
| `overrideType` | B2-4, B3-A (variants, layouts) |
| `keyType` | B1-1 (curves, easeIn/Out, steps, tcb, spring), B1-5 (marker), B2-2 (spatialIn/Out, roving) |
| `animateType` | B1-1 (animate), B2-1 (expression), B2-2 (link, motionPath) |
| `expressionType` | B2-1 |
| `motionPathType` | B2-2 |
| `linkType` | B2-2 |
| `animationElements` | B1-1 (animate), B2-1 (expression), B2-2 (link, motionPath) |
| `animatedType` | B1-1 (animate), B2-1 (expression), B2-2 (link, motionPath) |
| `timeRemapType` | B2-8 (frame-mix), B3-J (optical-flow) |
| `maskType` | B1-3 |
| `transformConstraintType` | B2-3 (parent … distance), B3-E (ik), B3-J (track) |
| `stopType` | B1-4 (oklch finalised in B3-E) |
| `gradientCommon` | B1-4 (oklch finalised in B3-E) |
| `gradientBaseType` | B1-4 (oklch finalised in B3-E) |
| `linearGradientType` | B1-4 (oklch finalised in B3-E) |
| `radialGradientType` | B1-4 (oklch finalised in B3-E) |
| `conicGradientType` | B1-4 (oklch finalised in B3-E) |
| `meshPointType` | B3-E |
| `meshGradientType` | B3-E |
| `patternPaintType` | B3-E |
| `paintsType` | B1-4 (oklch finalised in B3-E) |
| `fontWeightType` | B3-B |
| `fontStyleType` | B3-B |
| `characterStyle` | B3-B |
| `tokenType` | B1-2 (tokens), B3-B (textStyle) |
| `textStyleType` | B3-B |
| `stylesType` | B1-2 (tokens), B3-B (textStyle) |
| `representationType` | B2-8 |
| `imageAssetType` | 1.0 existing; B2-8 (alpha, sha256), B3-I (transfer, layer) |
| `videoAssetType` | 1.0 existing; B2-8 (alpha, pixelAspect, rotation, timecode, audio), B3-I (transfer) |
| `imageSequenceAssetType` | B2-8 |
| `audioAssetType` | 1.0 existing; B3-G (bpm, language), B2-8 (representations) |
| `spanType` | B3-B |
| `textAssetType` | 1.0 existing; B2-7 ({{param}}), B3-B (everything else) |
| `vectorAssetType` | 1.0 existing; B1-4 (new shapes, stroke style, paints), B3-E (svg) |
| `meshAssetType` | 1.0 OBJ existing; B3-K (all other formats) |
| `slotType` | B3-M |
| `lottieAssetType` | B3-M |
| `fontAssetType` | B3-B |
| `generatorAssetType` | B3-E |
| `chartSeriesType` | B3-M |
| `chartAssetType` | B3-M |
| `audiogramAssetType` | B3-G |
| `codeAssetType` | B3-M |
| `formulaAssetType` | B3-M |
| `generatedAssetType` | B3-M |
| `assetsType` | B1-0 (dispatch) |
| `materialType` | 1.0 existing; B3-K (everything else) |
| `materialsType` | 1.0 existing; B3-K (everything else) |
| `transformAttributes` | 1.0 existing; B1-3 (skew), B1-2 (lengths) |
| `nodeAttributes` | B1-3 (matte, skew), B1-5 (markers, name, tags), B2-2 (condition), B2-3 (parent), B3-A (align*), B3-F (motionBlur), existing depth-card attributes via E1 |
| `warpPointType` | 1.0 existing |
| `puppetPinType` | B3-E |
| `modifierType` | 1.0 existing (6 types); B3-E (the other 8) |
| `deformType` | 1.0 existing |
| `boneType` | B3-E |
| `skeletonType` | B3-E |
| `rigidBodyType` | 1.0 existing; B3-L (new attributes and types) |
| `softBodyType` | 1.0 existing; B3-L (new attributes and types) |
| `textAnimatorType` | B3-B |
| `textPathType` | B3-B |
| `shapeModifierType` | B3-E |
| `nodeBehaviour` | B1-0 (dispatch) |
| `bodyBehaviour` | B1-0 (dispatch) |
| `layerType` | 1.0 existing; B1-3 (effects on layers), B2-8 (fit, crop, flip, freeze, remap, volume), B3-B (text children), B3-J (stabilize, optical-flow), B3-G (audioBus) |
| `shapeType` | 1.0 existing; B1-4 |
| `burstType` | 1.0 existing; B3-L |
| `particleEmitterType` | 1.0 existing; B3-L |
| `object3DType` | 1.0 existing; B3-K (B2-3 constraints, B2-2 condition) |
| `shakeType` | B3-F |
| `cameraType` | 1.0 existing; B2-3 (target, focusTarget), B3-F (everything else) |
| `groupType` | 1.0 existing; B1-5 (timeOffset, timeScale), B3-A (layout, size, clip, isolate, collapse) |
| `sequenceType` | B1-5, B3-C (transition attributes) |
| `instanceType` | B2-4 |
| `includeType` | B2-5, B3-G/B3-L (included audio and physics) |
| `repeatType` | B2-6 (count), B3-A (over, var) |
| `adjustmentType` | B1-3 |
| `transitionType` | B3-C |
| `nodeChoice` | B1-0 (dispatch) |
| `compositionType` | B1-0 (dispatch) |
| `symbolType` | B2-4 |
| `symbolsType` | B2-4 |
| `effectType` | 1.0 existing (8 types); B3-D (the rest), B1-3 (adjustment host) |
| `effectsType` | 1.0 existing (8 types); B3-D (the rest), B1-3 (adjustment host) |
| `lightType` | 1.0 existing; B3-K |
| `lightsType` | 1.0 existing; B3-K |
| `forceFieldType` | 1.0 existing; B3-L (new attributes and types) |
| `constraintType` | 1.0 existing; B3-L (new attributes and types) |
| `physicsType` | 1.0 existing; B3-L (new attributes and types) |
| `trackDataType` | B3-J |
| `trackingType` | B3-J |
| `audioSecondsType` | B3-G |
| `eqBandType` | B3-G |
| `audioEffectType` | B3-G |
| `duckingAttributes` | B3-G |
| `audioTrackType` | 1.0 existing; B3-G |
| `busType` | B3-G |
| `masterType` | B3-G |
| `audioMixType` | 1.0 existing; B3-G |
| `wordType` | B3-H |
| `cueType` | B3-H |
| `captionTrackType` | B3-H |
| `captionsType` | B3-H |
| `markerType` | B1-5, B3-H (chapters) |
| `beatGridType` | B1-5, B3-H (chapters) |
| `markersType` | B1-5, B3-H (chapters) |
| `paramType` | B2-7 (scalar types), B3-A (list, asset) |
| `bindType` | B3-A |
| `dataSourceType` | B3-A |
| `setType` | B3-A |
| `variantType` | B3-A |
| `parametersType` | B3-A |
| `safeAreaType` | B3-A |
| `safeAreasType` | B3-A |
| `layoutType` | B3-A |
| `layoutsType` | B3-A |
| `lookType` | B3-I |
| `colorManagementType` | B3-I |
| `metaType` | B1-2 |
| `accessibilityType` | B3-H |
| `metadataType` | B1-2 |
| `scene360Type` | 1.0 existing; B3-N (layouts, stereo) |
| `projectType` | 1.0 existing; B1-4 (background paint), B3-F (motion blur, quality), B3-N (timecode, pixelAspect), B3-A (safeArea) |
| `stillType` | B1-6 (png, jpeg), B3-N (webp, avif) |
| `destinationType` | B3-N |
| `outputType` | 1.0 existing; B1-6, B3-G (audioLanguages, E7), B3-H (captions), B3-I (HDR), B3-A (layout, variant), B3-N (the rest) |
