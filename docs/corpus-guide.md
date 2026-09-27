# doc-ray corpus guide for scene-render

This guide lists which documents in the doc-ray corpus (the claude.ai
`doc-ray-rw` connector, 950 documents) can help improve scene-render, and
what each one says that applies here.

- **Citations** are written `doc-id prefix : sentence ordinal`.
- **Sources:** nine research passes on 2026-09-25. Every takeaway was checked
  against the code at `5b7dca1` and against the schema-1.1 roadmap
  (`docs/schema-1.1-batch{1,2,3}-proposal.md`).
- **Performance, memory, parallelism, CPU graphics, 3D shadows, physics,
  fuzzing and the media pipeline** are covered in
  [performance-plan.md](performance-plan.md). This guide adds the remaining
  areas and only summarises those.

## Core shelf

These documents come up again and again. Read them first.

| Doc | Title | Use for |
|---|---|---|
| `363e9ddb` | *Fundamentals of Computer Graphics* 5e (FCG) | curves, motion paths, noise, quaternions, parenting, tone mapping, mipmaps, colour maps |
| `8bea5cf5` | *Computer Graphics: Principles and Practice* 2e in C (CGPP) | flattening, stroking, clipping, shade trees, motion blur, particles |
| `f9fdeeca` | Muchnick, *Advanced Compiler Design* | expression bytecode, constant folding, FP rules, scoped symbol tables, memory hierarchy |
| `49b31c3a` | LLVM Documentation r7 | parser shape (Kaleidoscope), FP flags, vectorisation, LTO |
| `a4b286db` | Madhav, *Game Programming in C++* | camera rigs, Catmull-Rom, quaternion API, fixed steps, SAT, sweep-and-prune |
| `a078c8ce` | *Designing Data-Intensive Applications* 2e | schema versioning and evolution, loader dispatch |
| `b9b896e2` / `c0e98e2f` / `6c9f40d5` | *Fundamentals of Software Architecture* 2e / *Clean Architecture* / *Clean Code* | registries, plugin boundaries, replacing switches with tables |
| `1088abf3` | Gregg, *Systems Performance* 2e | measurement methodology |
| `d78063ac` / `16448fc3` | *Elements of Typographic Style* / *Universal Principles of Typography* | justification, hyphenation, tracking, line height |
| `220822c0` | *The Audio Programming Book* | biquads, RMS envelopes, pan law, clipping |

## Findings by area

### 1. Architecture and extensibility

This is roadmap batch 1: the dispatch table, capability gate, version gate
and property registry.

- **Replace type switches with dispatch tables.** `effects.c` switches on
  `effect->type` in two places (≈156, ≈1038) and `xml.c` has about 40
  `strcmp` branches. Use one table per concern:
  - `SrEffectVTable[type] = {parse, validate, apply}`,
  - element rows `{name, parents, handler, min_version, capability}`.

  A switch should appear once, and only to build the polymorphic object
  [6c9f40d5:1071, 1089]. The registry is owned by the core, and handlers
  talk only to the core [b9b896e2:2276–2308].
- **Capability rows need `requires`/`excludes`.** Features can be optional
  or mutually exclusive and can have prerequisites [e84a6477:181, 1392, 3735].
  `--report-unsupported` then just walks the table.
- **Version gate.** Keep the version in one place (root `@version`). An
  unknown version stops parsing with its own diagnostic code, not
  `SR_ERR_XML` [f80bb7ea:1768, 4570–4573].
  - *Reject* documents newer than the loader. A renderer must not ignore
    unknown elements, because that silently changes the picture
    [a078c8ce:2627, 2631; df249bbb:199].
- **Additive evolution only.** Every 1.1 attribute on a 1.0 element needs a
  default that reproduces 1.0 output byte for byte [a078c8ce:2738, 2890].
  - Retired names stay in the registry as tombstones [a078c8ce:2740].
  - Keep `scene-v1.xsd` plus its fixtures as the compatibility record, and
    test that 1.0 scenes are bit-identical under the 1.1 loader [a078c8ce:2805].
- **Split `compositor.c` (~2000 lines) and `lighting.c` (~1800 lines)**
  along the lines along which the roadmap will change them: blend/matte,
  shape/stroke/gradient, and layer traversal [31d37158:879, 2087, 2111].
- **Error handling.** Mark `SrStatus` returns `warn_unused_result` behind a
  macro [b8c5067e:997, 1011]. Assert table invariants at startup; crash
  early [65eb38c8:2169, 2225].

### 2. Expression language and schema machinery

This is roadmap batch 2: expressions, links and conditions,
constraints/parenting, symbols, include, repeat and params.

- **Parser.**
  - Use recursive descent for primaries, calls and indexing.
  - Use precedence climbing from a single table for binary operators, with
    `**` right-associative and `?:` explicit [49b31c3a:6963, 7024–7049, 7604].
  - Stop at the first error with a single `SrExprError{offset, element,
    attr, msg}`. Show a caret line mapped back to the XML line and the
    include chain [ffa5f379:2737, 2947, 3101].
- **Evaluation.**
  - Lower to linear postorder bytecode for a stack VM
    [f9fdeeca:863, 1048, 1086, 7350]. A tree-walking interpreter only suits
    trivial grammars [ff240781:3486, 3490].
  - Fold constants at load using **the same C function per operation** as
    the VM [f9fdeeca:3371–3384, 3700, 3734].
  - **No floating-point algebraic rewrites.** No reassociation and no
    `x+0 → x`; the only IEEE-safe ones are dropping redundant coercions and
    turning division by a constant into multiplication [f9fdeeca:3464, 3472].
  - Hoist time-independent subexpressions once per element [f9fdeeca:3928–3981].
  - Pin `-ffp-contract=off` and keep FMA out of the VM ops [49b31c3a:747, 2088, 9099].
- **Dependency graph** (`prop()`, links, constraints, `@parent`):
  - Take a topological order once at load [cd3e3356:2323, 2329]. Kahn's
    iterative version has no recursion depth, which matters for scenes of
    about 10⁶ nodes [23c96ead:4220–4231].
  - On failure, report *every* strongly connected component with more than
    one member, or with a self-loop, in a single error [cd3e3356:5228, 5239, 4848].
  - Memoise only on `(node, prop, t)`, and route recursion through the
    memoised entry point [029d95d4:1461, 1861, 1877].
- **Symbols, include and repeat.**
  - Expand at load as a Prototype deep clone, *before* id resolution
    [ff240781:1962, 1986]. Flyweight-style per-instance overrides cost more
    [ff240781:2889–2918].
  - Generated ids use an unforgeable separator, like gensym hygiene
    [ea5e6fa3:1569–1576].
  - Use a stack-plus-hash scoped symbol table instead of the linear
    `strcmp` scans in `xml_resolve.c` [f9fdeeca:662–688].
  - Give each document an explicit base for relative references, like
    `xml:base` [df249bbb:218, 251, 260].

### 3. Curves, time and mathematics

These affect `timeline.c`, `vector_path.c`, `camera.c`, `particles.c` and
`audio.c`.

- **Cubic-bezier easing.** `sr_bezier_progress` runs 24 fixed bisection
  steps. Use a safeguarded Newton solve with a fixed iteration count, which
  reaches machine precision in about 5 steps and falls back to bisection
  when x′≈0 [23c96ead:6367, 6370; 94b61e83:1820].
- **Auto tangents.** Add a `smooth` (Catmull-Rom) curve type for keys given
  without tangents. C1 continuity is enough [363e9ddb:6750–6754;
  a4b286db:3622, 3628]. Linear keys break velocity continuity even when the
  keys are collinear [8bea5cf5:17434].
- **Motion paths and constant speed.**
  - Position = p(u(s(t))) [363e9ddb:6798–6811]. Build an arc-length table
    by summing chords over the flattened polyline, then binary search and
    interpolate linearly [363e9ddb:6820–6823].
  - `constantSpeed` should take an easing curve for s(t), not just a flag.
  - Replace the fixed 16-step `flatten_cubic` with adaptive de Casteljau
    subdivision [8bea5cf5:8251, 8333, 8341].
  - Trim and text-on-path can reuse the same table.
- **Rotation.**
  - 3D and depth cards: unit quaternions with shortest-arc slerp, falling
    back to normalised lerp near φ≈0 [363e9ddb:6875–6891; 8bea5cf5:8487;
    92d431da:4201].
  - The **360 camera** stays on yaw/pitch plus an up vector: raw quaternion
    interpolation tilts the horizon [8bea5cf5:17491–17493].
  - Multi-turn spins need intermediate keys [8bea5cf5:17489].
- **Noise and wiggle.**
  - Classic Perlin: Hermite weights, gradients rejection-sampled from the
    unit sphere, and a 256-entry permutation table seeded from the existing
    splitmix64 [363e9ddb:4255–4268].
  - Turbulence sums octaves and stops once an octave reaches pixel size
    [363e9ddb:4296; 8bea5cf5:17219].
- **Audio analysis bands.**
  - Use Butterworth biquads [220822c0:6092, 6094] cascaded into 4th-order
    Linkwitz-Riley crossovers that sum flat [f23393e9:1125–1127].
  - The RMS window is about 15 ms [220822c0:3075], while one frame at
    30 fps is 33 ms; decide explicitly which the feature uses.
- **Strokes and booleans.** Use a miter limit [8bea5cf5:15766]. For batch 3
  booleans, use Weiler region clipping, which handles concave polygons with
  holes [8bea5cf5:11183, 15422].

### 4. Rendering architecture, shaders and post

- **User shaders.** A CPU renderer should not target GLSL. Use a
  deterministic **shade tree or bytecode "effect program"** at fixed hook
  points: surface for the MaterialX subset, pixel for effects and
  transitions [8bea5cf5:13223–13243]. A small, C-like language that runs
  data-parallel over points (as VEX does) is a precedent [7bd731a1:637–640].
- **Parenting.**
  - World matrix = the product of the chain up to the root, built once per
    frame after keys, then expressions, then constraints, and cached per
    node [363e9ddb:4625, 6939; a4b286db:1133, 2170].
  - Once 3D objects inherit parent transforms, `lighting.c` needs the
    **inverse-transpose for normals** [363e9ddb:2183, 2190]. Today it only
    uses `|scale|` per axis.
  - Add a `weight` to each constraint [a0109298:492, 497].
- **Motion blur.** Shutter-weighted temporal supersampling, with jitter from
  seeded hashes. Too few samples show as multiple exposures
  [8bea5cf5:12612–12619, 12869].
- **Tone mapping.** Compress luminance and rebuild colour from the channel
  ratios, with global operators by default [363e9ddb:9042–9053, 9128, 9301].
  Keep **exposure manual**: auto-exposure is stateful and breaks rendering
  any frame independently [362bf31f:547–556].
- **Depth cards.**
  - `card.c` blurs a whole card using its pivot depth
    (`sr_card_blur_radius`), so **tilted cards blur wrongly**. The corpus
    gives the thin-lens circle-of-confusion basis [363e9ddb:5544;
    8bea5cf5:12598] but no variable-radius blur.
  - A three-layer parallax preset is a cheap, believable baseline
    [a4b286db:906–914].
- **Cameras.** A spring-follow camera integrated per frame breaks render-any
  frame; use a closed-form critically damped solution
  [a4b286db:3577–3581]. A spline camera looks slightly ahead on its curve
  [a4b286db:3636, 3670]. The physical camera links filmback, FOV and f-stop,
  and a debug focus-plane view helps [362bf31f:1260–1266].
- **Particles.**
  - value = centre + random × variance, which the current `*_variance`
    already follows [8bea5cf5:16948].
  - Sub-emitters are hierarchical particle systems [8bea5cf5:16941].
  - `streak` = antialiased segments along the in-frame path [8bea5cf5:16953].

### 5. Typography and text (`text.c`, batch 3 B3-B)

- **Justification.** `emit_line` spreads all slack over word spaces. Add a
  `maxWordSpacing` cap and put the remaining slack into letter spacing
  [d78063ac:2812]. Under about 40 characters per line, warn or fall back to
  ragged [d78063ac:285, 295].
- **Hyphenation defaults:**
  - `minBefore=2`, `minAfter=3` [d78063ac:554],
  - at most 3 hyphenated lines in a row [d78063ac:557],
  - no last line shorter than 4 letters [d78063ac:556].

  Justification doesn't prevent widows [16448fc3:1005], so add a widow
  check to `balance` and `maxLines`.
- **Tracking.**
  - Add 5–10% for capitals and small caps by default
    [d78063ac:324, 360].
  - Track tighter at large sizes, negative only at very large sizes
    [16448fc3:386, 390].
  - Set the `opsz` axis from the font size automatically [16448fc3:316].
- **Vertical metrics.** Line height is a multiple of the font size
  [16448fc3:1109, 349]. Ascender/descender centring varies between fonts
  [16448fc3:162, 935], so add a cap-height/baseline trim `valign`.
- **Hanging punctuation** is not in the roadmap yet [d78063ac:1154; c7e67c56:239].

### 6. Motion design, colour and layout

- **Camera moves.** Default camera keys to ease-in-out, since abrupt changes
  make viewers sick [8bea5cf5:1722–1725]. Keep an FOV `zoom` separate from
  a translating `dolly`/`truck`: a zoom flattens depth, a dolly creates
  parallax [7392030a:478–488].
- **Animation principles.**
  - Overlap actions (start the next before the last ends) [363e9ddb:6692–6696].
  - Offer a velocity-driven, area-preserving squash/stretch modifier; it
    also fights strobing [363e9ddb:6698–6703].
  - Stage one event at a time [8bea5cf5:1727, 1732].
- **Timing tokens.** Named durations `xs…xl` on a base-plus-steps scale,
  with typical moves at 200–500 ms [892bdce5:1782–1786; 71cc1125:1407].
- **Gradients.**
  - Default `interpolationSpace` to oklab.
  - When the endpoints are nearly complementary, take a 60–120° hue span
    or add an automatic mid-stop, to avoid "muddy" greys
    [ce309fed:336–337; c99916a9:3477].
  - Chart presets: monotonic-lightness ramps, not rainbow maps
    [363e9ddb:10431, 10440].
- **Accessibility and safe areas.**
  - Flash check: at most 3 flashes per second, with an area limit
    [7282fb0c:8990].
  - Text contrast lint on lightness, not hue [c99916a9:2077].
  - Safe-area presets `broadcast-title` (10% inset), `broadcast-action` and
    a `lower-third` guide [4236b5cd:1444, 1453–1455].
  - Grid layout snaps to a baseline grid [a18fdfa5:111, 158, 160].

### 7. Testing and delivery

- **Property tests** derived from invariants, each with a logged seed:
  - monotonic cubic-bezier,
  - springs settle,
  - `steps` hits its declared levels,
  - colour round-trips,
  - coverage stays within [0, 1].

  Freeze failures as regression tests [65eb38c8:3783, 3822–3827, 4674].
- **Metamorphic tests** instead of more golden images
  [39e3e917:1953–1954]:
  - translating a layer shifts its pixels,
  - opacity 0 matches the layer being absent,
  - relative lengths behave the same at 2× canvas,
  - output is thread-count invariant on every example.
- **An internal `sr_test_*` API** that renders one layer or evaluates one
  property, to reduce fragile full-pipeline goldens [c0e98e2f:2876–2891].
- **CI.** The repo has **no CI config**. Add a commit stage that builds,
  runs unit tests, static analysis, the golden hashes, the coverage gate
  and the equivalence oracle [f9a032df:2238, 3098, 3372; b9b896e2:1239, 5268].
  Also write a semver/N-2 support policy [8885a8b5:486, 1757].

### 8. Already covered: see performance-plan.md

- Memory hierarchy and parallelism: Muchnick, Gregg, *C++ High Performance*.
- Compiler flags and vectorisation: LLVM.
- CPU graphics algorithms and 3D shadows: FCG, CGPP.
- 2D physics solvers: Madhav, FCG.
- Fuzzing and robustness: SDL, threat modelling, *Pragmatic Programmer*.
- Media pipeline, pan law, dither and gamut: *Audio Programming Book*, Gersho.

## Gaps in the corpus

The corpus has little or nothing on the topics below. These are the
suggested additions.

- **Language implementation:** *Crafting Interpreters*, the Dragon Book,
  *Engineering a Compiler* (Pratt parsing, VM design, error recovery,
  sandbox budgets).
- **Reproducible floating point:** correctly rounded libm (CRlibm/CORE-MATH)
  and cross-platform determinism.
- **Signal analysis:** onset/beat detection (Bello et al.) and FFT
  derivation.
- **Geometry:** robust booleans (Vatti, Greiner-Hormann), curve offsetting,
  constrained Delaunay / ear clipping, simplex noise.
- **Font engineering:** the OpenType specification (GPOS, vertical metrics,
  variations), UAX #14 line breaking, and Knuth-Plass.
- **Colour:** OKLab/OKLCH and CSS Color 4 hue interpolation, ACES/filmic
  tone curves, `.cube` LUT internals.
- **Rendering:** render-graph and frame-graph design; scatter/gather depth
  of field; per-pixel motion vectors.
- **Motion design:** a craft book (*The Illusion of Life*, Williams, After
  Effects references).
- **Standards:** broadcast safe areas (EBU R95/SMPTE), subtitle timing, and
  After Effects/Lottie expression semantics.
- **Engineering practice:** *Working Effectively with Legacy Code* and
  *Refactoring*; C API/ABI design (opaque handles, versioned structs,
  symbol versioning); golden-image and perceptual diff testing; mutation
  testing for C.

## Corpus data quality

**Broken extractions** (the body contains other books' text; re-ingest them):

| Doc | Title |
|---|---|
| `941aaff5` | *Calculus for Computer Graphics* |
| `3d57d415` | *Modern C* 2e |
| `c9022c82` | *Optimized C++* (titled "Optimized C…") |

**Other issues:**
- **Truncated:** `c7e67c56` *Thinking with Type* (only 275 sentences).
- **Garbled OCR:** `6082bdb8` *Math for CG Applications* (formulas);
  `d19fae74` *Data Structure and Algorithmic Thinking with Python* (code);
  `f01504f1` *CG lecture notes* (spaces stripped).
- **Duplicates:**
  - `0913c1bf` / `b4795ad9` (*Graphic Design and Print Production Fundamentals*),
  - `dbfe113e` / `40331c22`,
  - `b6447b19` / `db190c70`,
  - LLVM Kaleidoscope chapters appear twice inside `49b31c3a`.
- **Suspect:** `6915157b` *Dashboard Design* contains compiler-paper text.
- **Search noise:** full-text search ranks the Dune novels highly for
  generic terms ("metamorphic", "squash stretch").
