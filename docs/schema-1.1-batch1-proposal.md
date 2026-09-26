# Scene format 1.1 — first implementation batch (proposal)

Status: accepted, 2026-09-25. Input: `schema/scene-render-1.1.xsd` (3020 lines,
not yet committed) against `schema/scene-v1.xsd` (692 lines, fully implemented).

## 1. Where we start

- The renderer implements every element and attribute of `scene-v1.xsd`. No
  schema-only features exist today (see `docs/feature-matrix.md`).
- The 1.1 schema is about 4x the size of 1.0. It adds 15 new top-level
  sections and 9 new asset kinds. It adds about 80 effects, 29 blend modes,
  36 easing curves, templating, captions, an audio bus graph, colour
  management and multi-output delivery.
- Most of 1.1 is a **target contract**, not something one batch can deliver.
- Hard constraints that carry over:
  - Deterministic output.
  - Byte-identical frames at every thread count.
  - Golden images that do not move for existing scenes.
  - Build and test only inside the Flatpak SDK.

The first batch therefore has two jobs:

1. Make 1.1 the contract the loader speaks without breaking any 1.0 document.
2. Implement the foundation that most later 1.1 features depend on.

Those foundations are the animation core, lengths, paints, compositing and
the timeline structure.

## 2. Findings in the 1.1 schema that must be fixed first

These were verified by validating all 45 repository scenes with `xmllint`
against both schemas.

| # | Finding | Evidence | Proposed fix |
|---|---------|----------|--------------|
| S1 | **1.1 is not a superset of 1.0**, despite its header. It removes the depth-card attribute `depth` from `nodeAttributes`, and `zoom` and `aperture` from `camera`. | 42 scenes pass 1.0 validation; 2 of them fail 1.1: `examples/dusk-depth.xml` (10 errors) and `tests/data-depth.xml` (7 errors). | Re-add `depth`, `camera/@zoom` and `camera/@aperture` as deprecated aliases. `depth` maps to `zDepth`; `aperture` stays as-is, and `fStop` is defined in terms of it. |
| S2 | **Depth-card trigger semantics changed.** In 1.0, any of `depth`, `rotationX` or `rotationY` turns a node into a depth card. In 1.1 that needs `threeD="true"`. | `scene-v1.xsd` comment on nodeAttributes; `xml_elements.c:383-387` | `version="1.0"` documents keep the implicit trigger. `version="1.1"` documents need `threeD`, and the loader warns when `rotationX`, `rotationY` or `zDepth` is set without it. **Owner decision** (see §6). |
| S3 | The header points to a companion `scene-render-1.1.sch` that does not exist. That file would hold the version gate and the cross-field rules. | `schema/` contains only the two XSDs. | Implement those rules in C in `xml_resolve.c`, next to the existing semantic checks, and do not ship Schematron. libxml2's Schematron support is minimal, and a second validation engine adds attack surface. Record each rule in `docs/xml-reference.md`. |
| S4 | libxml2 does not check that an IDREF resolves. For example, `key/@marker="beat.4"` validates even though no element has that id; beat ids are generated from `beatGrid`. | Probe document validated. | Keep all reference resolution in `xml_resolve.c` (already the pattern there). Add generated marker ids (`beat.N`, `bar.N`) to the id table before resolving. |
| S5 | `animate/@property` and `expression/@property` are plain strings. The real vocabulary lives only in code (`scene.c:365-400`, `xml_elements.c:388-457`). | Inventory | Build a single property registry (§4, B1-1). Later, `override`, `bind` and `link` need the same table. |

## 3. Guiding rules for the batch

1. **Implemented profile vs contract.** The XSD accepts everything in 1.1. The
   loader then checks a single *capability table*. The table lists, per
   element, attribute and enumeration value, what this build implements.
   - Anything not implemented fails with a distinct diagnostic:
     `unsupported in this build: <effect type="vhs">` with the line number.
     This keeps "invalid document" separate from "not implemented yet".
   - `--validate --report-unsupported` lists every such use instead of
     stopping at the first one.
   - This replaces the current rule that any unknown element is an error
     (`xml.c:315`).
2. **Version gate.**
   - `version="1.0"` documents may use only 1.0 elements and enumeration
     values. New *attributes* on 1.0 elements are allowed, as the 1.1 header
     says.
   - Anything else requires `version="1.1"`.
3. **No golden movement.**
   - All 14 golden scenes and the `tests/golden.sha256` integration hashes
     must stay byte-identical.
   - Every new feature brings its own golden scene (at most 320×180, 24
     frames).
   - Arithmetic changes to existing paths stay deferred to the planned single
     golden refresh.
4. **One embedded schema.** Once S1 is fixed, the build embeds only the 1.1
   XSD. `--print-schema` prints it. `scene-v1.xsd` stays in the repository as a
   test fixture: every file that validates under 1.0 must also validate
   under 1.1 (a new CTest).

## 4. Batch 1 work items

Items are ordered by dependency. Each is sized so it can go through one Codex
diff review, as the port phases did.

### B1-0 Loader infrastructure (prerequisite)

- Replace the `strcmp` if-chain in `on_start` (`xml.c:116-316`) with a
  table-driven dispatcher. Each entry holds: element, allowed parent kinds,
  handler, minimum version and capability flag. The 1.1 schema has about 110
  element contexts, and the if-chain will not scale to that.
- Add the capability table and the "unsupported in this build" diagnostic
  described in §3.1.
- Add the version gate from §3.2.
- Embed the 1.1 XSD once S1 is fixed in the schema.
- Add a CTest that validates every 1.0 fixture against 1.1.
- Accept the new root sections (`metadata`, `styles`, `markers`, `paints`,
  multiple `output`). Everything else in the root sequence parses into the
  capability check only.
- Fix: `E_PARTICLES` is never pushed; emitters push `E_LAYER`
  (`xml_nodes.c:315`). The table makes that mismatch visible, so fix it here.

Acceptance:
- Goldens are byte-identical.
- Every 1.1-only construct is either implemented or rejected with a specific
  diagnostic.
- The coverage gate still passes (88.30 % lines / 69.29 % branches).

### B1-1 Animation core

- **Property registry.** One table mapping `(host kind, property name)` to
  field offset, value type (number, point, colour, paint, path, string) and
  bounds. It replaces the mapping now split between `scene.c` and
  `xml_elements.c`.
- **Curves** (`timeline.c`):
  - `hold` (alias of step);
  - `steps` with `steps` and `stepPosition`;
  - the Penner family (sine, quad, cubic, quart, quint, expo, circ, back,
    elastic, bounce; each in, out and in-out);
  - `catmull-rom`;
  - `tcb` (tension, continuity, bias);
  - `spring` (stiffness, damping, mass), evaluated analytically so there is
    no integration state.
- **Key handles.** After Effects-style `easeIn` and `easeOut` (influence and
  speed) converted to cubic-bezier. `spatialIn`/`spatialOut` and `roving` are
  deferred together with motion paths.
- **animate attributes:**
  - `extrapolateBefore` / `extrapolateAfter`: `hold`, `linear`, `loop`,
    `ping-pong`, `offset`;
  - `additive`;
  - `timeBase`: `composition`, `local`, `normalized`.
- **Animation children on new hosts.** Hosts that accept the
  `animationElements` group accept `animate` now. `expression`, `link` and
  `motionPath` are parsed but reported as unsupported until batch 2.

Tests:
- A unit test per curve against reference values (Penner closed forms, spring
  closed form).
- One golden scene with a row of shapes, one per curve family.

### B1-2 Lengths and style tokens

- **Relative lengths.** `lengthType` and `positiveLengthType` accept `%`,
  `vw`, `vh`, `vmin` and `vmax`. They are used on the transform, on masks, on
  shape width and height, and on group width and height.
  - Resolve them once per frame, at evaluation time, so a later `layout` can
    change the frame size.
  - `%` is relative to the parent box. A group without `width`/`height` passes
    the frame box through.
  - This rule is not in the schema; add it to `docs/xml-reference.md` (see
    §6 Q2).
- **Tokens.** `styles/token` and `var(--name)` in any `colorType`. They
  resolve at load time; an unknown token is a load error.
- `metadata`: parsed and kept; written into the container as tags when the
  output has `embedMetadata="true"`. This is cheap and completes the root
  sequence.

### B1-3 Compositing

- **Blend modes.** Add the rest of `blendType` to `raster.c`:
  - the separable modes;
  - the non-separable modes `hue`, `saturation`, `color` and `luminosity`;
  - `darker-color` and `lighter-color`;
  - `dissolve`, seeded from the node id and the project seed;
  - the stencil and silhouette modes, `alpha-add` and `behind`, which act
    within the isolated parent buffer.

  Formulas follow W3C Compositing Level 1 where it defines them, and are
  documented per mode for the rest. These are pure per-pixel kernels, so they
  are easy to test exhaustively.
- **Skew.** `skewX` and `skewY` added to the node transform matrix.
- **Track mattes.** `matte`, `matteMode` (`alpha`, `alpha-inverted`, `luma`,
  `luma-inverted`) and `matteVisible`.
  - The matte node renders into a coverage buffer in the same parent space.
  - A matte is not drawn itself unless `matteVisible` is set.
  - A cycle through `matte` references is a load error.
- **Masks.**
  - `mode`: `intersect` (the 1.0 behaviour and the default), `add`,
    `subtract`, `lighten`, `darken`, `difference`, `none`.
  - `opacity`, `feather` (separable blur of the coverage), `expansion`.
  - `path`, `polygon` and `star` mask types, reusing `vector_path.c`.
- **Adjustment layer** (`adjustment` node). It applies `effects` to the
  composite below it inside the parent buffer, reusing the group-effect path
  (`effects.c:718`). Its masks, matte and opacity limit the result.

### B1-4 Shapes and paints

- **Shape node:**
  - `polygon`, `star` (inner and outer radius, inner and outer roundness),
    `line`, `path`;
  - `cornerRadii`;
  - `fillRule`.
- **Stroke style:**
  - `strokeCap`, `strokeJoin`, `miterLimit`;
  - `dash`, `dashOffset`;
  - `strokePosition`: `inside` and `outside` via coverage clipping;
  - `paintOrder`.
- **Trim paths.** `trimStart`, `trimEnd`, `trimOffset` and `trimMode`
  (draw-on animation). They depend on the arc-length parametrisation added to
  `vector_path.c`.
- **Paints.**
  - `paints/linearGradient`, `radialGradient` (including focal point and
    aspect) and `conicGradient`.
  - `spread`, `units`, `rotation` and animated stops.
  - `interpolationSpace` of `linear`, `srgb` or `oklab`. `oklch` is included
    if the hue interpolation rules are settled; otherwise it is deferred.
  - `dither`: seeded ordered dither, so it stays deterministic.
  - `paintType` (`url(#id)`) is accepted on shape `fill` and `stroke`, on
    vector asset `fill` and `stroke`, and on the project `background`.
  - `meshGradient` and `pattern` are deferred.
- The same shape constructors also extend `vector` assets to
  `rounded-rect`, `polygon`, `star` and `line`. `shape="svg"` is deferred.

### B1-5 Timeline structure

- `markers/marker` and `beatGrid`. Beat grids generate `beat.N` and `bar.N`
  ids (see S4).
- Node `startMarker` and `endMarker`, and key `marker` snapping.
- Group `timeOffset` and `timeScale`.
- **`sequence` node.** Each child's start is the previous child's end plus
  `timeGap`. The `transition` and `transitionDuration` attributes are
  reported as unsupported until transitions land.
- Node `name` and `tags`, stored for diagnostics and later for safe-area
  enforcement.

### B1-6 Outputs

- **Multiple outputs.** Several `output` elements, each with an `id`.
  - Outputs at project resolution share one render pass, and each encoder
    consumes the same frames.
  - Outputs with a different `width`, `height` or `fps` render in their own
    pass. They do not rescale, because rescaling would break relative
    lengths later.
- Per-output `start` and `end` ranges.
- **Codecs.** Add the ones the SDK's libav already has (verified with
  `ffmpeg -encoders` in `org.freedesktop.Sdk//25.08`):
  - `prores` via prores_ks, with `proresProfile`;
  - `vp9` via libvpx-vp9;
  - `av1` via libsvtav1;
  - `png-sequence`, `tiff-sequence`, `exr-sequence`;
  - `gif`, `apng`.

  Each codec needs a bit-exact check at 1 and 4 threads, as the existing
  codecs have. Pin encoder threads so the container bytes are
  thread-invariant (see the x264 thread-count gotcha).
- `container`, `keyframeInterval`, `bFrames`, `faststart`, `loopCount`.
- `poster` and `thumbnail` stills as PNG and JPEG. `webp` and `avif` are
  reported as unsupported until their encoders are checked.

## 5. Explicitly not in batch 1

The following are proposed as later batches, roughly in this order.

| Batch | Area | Why it waits |
|-------|------|--------------|
| 2 | **Expression language**, `link`, `motionPath`, `condition` | The largest single design item. It needs its own design document covering the grammar (an ECMAScript expression subset), the evaluator, seeded `random`/`noise`/`wiggle`, the cost model, and a sandbox without I/O. It depends on the B1-1 property registry. |
| 2 | Symbols, `instance`, `include`, `repeat`, `override` | They depend on id scoping (`instance/inner`), the property registry and B1-5 timing. `include` also needs the sha256 and path-sandbox rules. |
| 2 | `imageSequence` asset, layer `fit`, `boxWidth`/`boxHeight`, crop, flip, `freezeAt`, `timeRemap` element | Small, but they belong with the responsive-media work. |
| 3 | Parameters, variants, data rows, `bind`, layouts, safe areas | Templating. It needs expressions (`param()`), `override`, relative lengths and multiple outputs. |
| 3 | Text: spans, text styles, `autoFit`, `wrap`, `maxLines`/`overflow`, background boxes, `textAnimator`, `textPath`, variable fonts and OpenType features, font assets, fallback | These rework `text.c` layout and depend on B1-1 and B1-4. |
| 3 | Transitions (the 35 types), plus `sequence/@transition` | They need handle extension and the B1-5 timeline structure. `shader` transitions depend on the GLSL decision below. |
| 4 | Audio: buses, `audioEffect` (the 16 types), ducking, BS.1770 loudness normalisation, `fadeCurve`, `preservePitch`, channel layouts | A separate DSP subsystem that needs its own determinism tests. |
| 4 | Captions: burn-in presets and sidecar files | They depend on the text batch and multiple outputs. Transcription reads from cache only. |
| 4 | The rest of the effects catalogue, about 80 types | Grouped as: pointwise colour effects (quick); blur family; distort; keying; stylise. `lut` needs .cube/.clf parsing. `shader` depends on the GLSL question. |
| 5 | Colour management: transfer functions, looks, tone mapping, OCIO, HDR output metadata | OCIO is a new dependency and there is no OpenCL or GPU path to test with. |
| 5 | Motion blur, `shutterAngle`, per-node `motionBlur` | Up to 16–256x render cost at the current ~9 s/frame. It needs the performance work first. |
| 5 | 3D: the new primitives, PBR material extensions, area and dome lights, glTF meshes, physical camera, camera switching | These go beyond the stated "no PBR" boundary and need a real 3D design pass. |
| 5 | Physics and particle extensions; rigging (skeleton, puppet, IK); shape modifiers; tracking data | These are niche and each is self-contained. |
| — | Lottie, chart, code (QR/barcode), formula (TeX), audiogram, generator and `generated` assets; `destination` upload; accessibility checks; 360 cubemap and stereo | Each needs a new dependency or an external service. `destination` in particular probably belongs in a delivery tool, not in the renderer, because it involves credentials and network I/O. |

## 6. Owner decisions

Adopted 2026-09-25: the owner accepted every recommendation below (see
`docs/schema-1.1-batch3-proposal.md` §2).

1. **S2, depth-card trigger.** Keep implicit depth cards for 1.0 documents
   only (recommended), or also for 1.1?
2. **`%` inside groups without a size.** Recommended: they inherit the frame
   box. The alternative is the union of the children's bounds, which is
   circular for layout.
3. **Custom GLSL** (`effect type="shader"`, `transition type="shader"`). The
   renderer is CPU-only and deterministic. Recommended: remove both from the
   1.1 enumerations, or mark them reserved, until a CPU GLSL interpreter or a
   deterministic GPU path is chosen.
4. **Schema ownership.** Recommended: commit `scene-render-1.1.xsd` together
   with the S1 fixes before B1-0 starts, so the contract and the loader move
   together.

## 7. Verification plan for the batch

- Every item passes the full CTest suite inside the Flatpak SDK, including
  ASan and UBSan (`ASAN_OPTIONS=detect_leaks=0`).
- Every item passes the coverage gate.
- Every item passes the 1-thread vs 4-thread golden comparison.
- **Compatibility check.** All 42 scenes that are 1.0-valid today must render
  byte-identically to the pre-batch binary. This reuses the equivalence
  oracle from the perf work (120 preview frames and 3 full encodes, compared
  by SHA-256).
- Each item gets a Codex diff review before merge. Findings are reproduced as
  failing tests, then fixed.
- New goldens:
  - `curves`
  - `blend-modes`
  - `mattes-masks`
  - `shapes-strokes`
  - `gradients`
  - `sequence-markers`
  - `adjustment`
  - one still per new codec where the format is lossless
