# Scene format 1.1 — second implementation batch (proposal)

Status: accepted, 2026-09-25. Input: `schema/scene-render-1.1.xsd`. This builds
on `docs/schema-1.1-batch1-proposal.md` ("B1"), whose owner decisions (B1 §6)
are still open.

## 1. Theme and dependencies

Batch 1 lays the static foundation: the loader tables, the animation core,
lengths, paints, compositing, the timeline structure and outputs. Batch 2
makes documents **programmable and reusable**. The goal is that the templating
batch (parameters, variants, layouts, text animators and transitions) can
follow without reworking the core.

| Batch 2 item | Needs from batch 1 |
|---|---|
| Expressions, links, conditions | B1-0 capability table; B1-1 property registry and curves; B1-5 markers and beat grid |
| Motion paths, spatial keys | B1-4 arc-length parametrisation in `vector_path.c` |
| Symbols, instances, repeat | B1-0 dispatch table; B1-5 group `timeOffset`/`timeScale` (an instance's clock is a time mapping on a subtree) |
| Responsive media | B1-2 relative lengths (`boxWidth` and `boxHeight` accept `%`, `vw`, …) |

## 2. How the current engine constrains the design

These facts were checked in the source, 2026-09-25.

- **Evaluation is lazy, pure and per frame.**
  - Every stage calls `sr_anim_eval(track, time)` while drawing
    (`timeline.c:97-125`).
  - World transforms are recomputed recursively in `sr_draw_node` from
    `context->time` (`compositor.c:976-994`). Nothing is cached.
  - Deform modifiers evaluate tracks per pixel inside row workers
    (`compositor.c:270-272`), so **evaluation must stay read-only and
    thread-safe**.
- **Any frame must be computable in isolation.**
  - `--preview-frame` renders one arbitrary frame.
  - `--range` starts anywhere.
  - `--resume` renders 150-frame segments and may skip some
    (`renderer.c:599-606`, `:751`, `:762-797`).
  - Physics shows the one accepted way to carry state: it is precomputed over
    the whole duration at prepare time and cached on disk with a content
    signature (`physics.c:863-985`).
- **Nodes are owned exactly once and have no parent pointer.**
  `sr_node_free` recurses into children (`scene.c:267-306`), so a subtree
  cannot be shared.
- **Id lookup is linear.**
  - `find_node` is a DFS; `sr_scene_id_exists` scans every table.
  - It runs once per parsed node (`xml_nodes.c:213`), so loading is O(N²).
  - There is no node-count limit.
  - Instances and repeats multiply the node count, so this must change first.
- **Randomness.**
  - The only generator is a counter-based splitmix64 in `particles.c:23-44`.
  - Its seed is the element's seed, or else project seed XOR FNV-1a of the id.
  - No noise function exists anywhere.
- **Media time.**
  - A `source.time` track bypasses clipIn, speed and loop
    (`compositor.c:635`).
  - A source time between frames always shows the earlier frame
    (`assets.c:199-231`).
  - A decoded video frame is valid only until the next request on the same
    source (`video.h:62-63`), so frame blending must copy the first frame.
- **Audio** is decoded only in `sr_run_audio_open` (`renderer.c:330-345`).
  That runs after asset and physics preparation, and the preview path returns
  before it. No amplitude analysis exists.
- **Loader.**
  - Parse state lives on the stack, so a nested parse is possible.
  - `sr_scene_init` resets the scene, so an include must parse *into* the
    existing scene instead of calling the top-level loader.
  - Assets resolve against a single `scene->base_dir` (`assets.c:127`).
  - Resume fingerprints cover the scene bytes plus each asset's single
    `source` (`resume.c:105-131`).
- **Floating point.**
  - Builds use `-ffp-contract=off -fno-fast-math`.
  - libm `sin`, `cos`, `pow` and `atan2` are already on the render path.
  - The determinism promise is per toolchain (`docs/feature-matrix.md:94-97`).
    Expressions inherit that promise and do not tighten it.

## 3. Findings in the 1.1 schema for this batch

S1–S5 are in B1 §2. S6 and S7 were verified with `xmllint` against the
schema.

| # | Finding | Proposed fix |
|---|---------|--------------|
| S6 | **Nested scopes are not addressable.** The instance documentation says inner ids become `instanceId/innerId`, but `override/@target` is `xs:NCName`, so `target="a/bg"` is rejected. An instance of a symbol that contains instances therefore cannot override the inner elements. | Change `overrideType/@target` to a scoped-path pattern: `[A-Za-z_][A-Za-z0-9_.\-]*(/[A-Za-z_][A-Za-z0-9_.\-]*)*`. Resolve it relative to the enclosing instance. |
| S7 | **IDREF kinds are not checked.** `instance symbol="a"` validates even when `a` is an instance and not a symbol. The same applies to `parent`, `matte`, `transformConstraint/@target` and `link` sources. | In `xml_resolve.c`, check that every reference points to an element of the expected kind. This comes almost free with the B2-0 index. |
| S8 | **Per-instance text cannot be written.** Assets live outside symbols, so an `override` cannot reach a text asset. Changing a title per instance is the most common reason to use a symbol. | Allow `override property="text"` (and the text-asset character attributes) on a **layer** inside the symbol. The loader then gives that instance a copy-on-write clone of the asset. **Owner decision** (§6 Q3). |
| S9 | **`prop("id.property")` is ambiguous.** `xs:ID` allows dots and property names contain dots, so `prop("card.position.x")` could mean id `card` with property `position.x`, or id `card.position` with property `x`. | Add a two-argument form, `prop(id, property)`. Resolve the one-argument form by the longest id that matches, and make it a load error when both readings resolve. |
| S10 | **The expression language is only named, not specified.** The schema says "a pure subset of ECMAScript expressions" plus a list of built-ins. It gives no grammar, no types, no rule for coercing a result to the property type, and no limits. | Write `docs/expressions.md` as the first deliverable of B2-1 (outline in §4). The XSD keeps pointing to it. |
| S11 | **Some built-ins are stateful by nature.** `link/@smoothing`, `link/@delay`, `wiggle`, `spring`, `loopIn`/`loopOut` and `valueAtTime` must be defined so that any frame can be computed in isolation (§2). | Define each one in closed form or as prepare-time precomputation (B2-1, B2-2). This is recorded as a normative rule in the expression spec. |
| S12 | **Included items are private.** Merging an included document under the `@id/` namespace makes its assets and symbols unreachable from the host's IDREFs, because `/` is not allowed in `xs:ID`. | Accept this as the rule: the host uses an include only as a node, and reaches inside it only through `override`, which now takes scoped paths (S6). State it in the schema documentation. |
| S13 | **`repeat/@over` depends on templating.** It points to list parameters or data rows, which arrive in batch 3. | Batch 2 implements `@count`. `@over` stays "unsupported in this build". |

## 4. Batch 2 work items

Items are in dependency order. Sizes are relative: S is about a week, M two
to three, L more than that. Each item gets a Codex diff review before merge.

### B2-0 Scene index, evaluation core, randomness (M, prerequisite)

- **Scene index.**
  - Build a hash map from id to `(kind, pointer)` at load, used by both the
    parse-time uniqueness check and resolution. This fixes the O(N²) load.
  - Add a parent pointer to `SrNode`.
  - Add `SR_MAX_NODES`: 1,000,000 nodes after expansion, can be lowered from
    the CLI. Exceeding it is a load error that names the element causing the
    expansion.
- **Property pipeline.** `sr_anim_eval` becomes `sr_prop_eval(node, prop, t)`,
  which applies in order:
  1. keyframes;
  2. expression, which replaces the keyed value; `value` in the expression
     still reads the keyed value;
  3. link, which replaces the value;
  4. transform constraints, for transform properties only.

  Physics then overrides the pose as it does today (`compositor.c:241`).
  Everything stays pure: identical inputs give an identical result, with no
  writes to the scene.
- **Dependency graph.**
  - Built at load from literal `prop()`, `link` and constraint references.
  - `prop()` arguments must be string literals, which makes the graph static.
  - A cycle is a load error that lists the cycle.
  - At draw time, a reference evaluates the referenced property recursively
    at the same time `t`.
- **Optional per-frame memo.** A per-thread memo table for `sr_prop_eval`,
  enabled only when a scene has expressions. It must not change results; the
  equivalence oracle confirms that.
- **`src/random.c`.**
  - Move splitmix64 and the seed derivation out of `particles.c`, keeping the
    particle streams bit-identical.
  - Add seeded 1–3D gradient noise and fBm (octaves, amplitude multiplier) for
    `noise()` and `wiggle()`.
  - Seed rule: the element's `seed` if given; otherwise project seed XOR
    FNV-1a of the scoped id and the property name.
  - Copies inside a repeat or an instance therefore wiggle independently
    unless they set `seed`.

### B2-1 Expression language (L)

**First deliverable:** `docs/expressions.md`, reviewed before any code. Outline:

- **Grammar.** An expression, optionally preceded by `const name = expr;`
  bindings. The language has:
  - literals: number, string, boolean, and array (`[x, y]`, `[r, g, b, a]`);
  - arithmetic: `+ - * / % **`;
  - comparisons, `&& || !` and `?:`;
  - indexing (`v[0]`) and function calls to built-ins.

  It has no statements beyond `const`, and no loops, user functions, objects,
  assignment or `this`. `Math.*` names alias the built-in maths functions
  (`Math.sin` and so on).
- **Values.**
  - Types are number (IEEE double), vector (1–4 numbers), string and boolean.
  - Vector arithmetic works element by element, and scalars broadcast over
    vectors, as in After Effects.
  - Truthiness follows ECMAScript rules for these types.
- **Coercion to the property type** uses the B1-1 property registry:
  - number ← number;
  - point ← 2-vector;
  - colour ← 4-vector in [0, 1], a 3-vector, or a colour string;
  - boolean ← any value, by truthiness;
  - anything else is a per-frame error.
- **Error policy.** A per-frame runtime error, such as NaN or a failed
  coercion, fails the render and reports the element, the time and the
  sub-expression. Silent fallbacks would break determinism of intent.
  `--expression-errors=hold` may keep the last keyed value for drafts.
- **Built-ins:**
  - time and context: `time`, `frame`, `value`, `index`, `count`, `seed`;
  - references: `param(name)` (B2-7), `prop(...)` (S9), `markerTime(id)`,
    `beat()` (B1-5 beat grid);
  - `valueAtTime(t)`: the property's *keyed* value at `t`. It never runs the
    expression again, so it cannot recurse.
  - `loopIn(type, n)` / `loopOut(type, n)`: `cycle`, `pingpong`, `offset` or
    `continue` over the keys, in closed form;
  - `wiggle(freq, amp[, octaves, ampMult])` and `noise(x[, y, z])`: fBm from
    `random.c`, with time as a noise coordinate;
  - `random([min, max])`: hashed from the seed and the frame index, so it
    changes every frame. `random()` with a fixed per-element seed can be
    obtained with `seed`.
  - `linear`, `ease`, `easeIn`, `easeOut` (AE argument order), `clamp`,
    `lerp`, `smoothstep`;
  - `spring(t, stiffness, damping, mass)`: closed-form damped oscillator;
  - `audioAmplitude(trackId[, band])` (B2-2);
  - `textIndex` and `textTotal`: reserved. Using them outside a text animator
    is a load error until batch 3.
- **Limits:**
  - source ≤ 64 KiB (already in the schema);
  - ≤ 4096 AST nodes and nesting depth ≤ 64;
  - a fixed operation budget per evaluation;
  - `prop()` chain depth ≤ 32.

**Implementation:**
- A hand-written recursive-descent parser. It compiles at load into a
  compact, immutable bytecode stored on the owning element.
- A stack VM with a caller-supplied stack, so it is thread-safe and does no
  allocation per evaluation.
- No `eval`, no string building beyond concatenation, and no I/O.
- **Where expressions may appear:** every host of the `animationElements`
  group once B1-1 has registered its properties. Effects, lights, paints,
  masks, modifiers and force fields all become expression-capable in one
  step.

### B2-2 Links, conditions, audio analysis, motion paths (M)

- **`link`.** Sources are `nodeId.property` (or the two-argument S9 form),
  `param:name`, `audio:track[:low|mid|high]` and `marker:id`.
  - Output = `clamp(source(t - delay) * scale + offset, min, max)`.
  - `smoothing` means a normalised box average of the source over
    `[t − smoothing, t]`, sampled at the project frame rate. This is stateless
    and exact at any frame, and costs O(smoothing × fps) evaluations. The
    alternative, exponential smoothing, carries state and is rejected.
- **`condition`.** A node attribute holding an expression evaluated at each
  frame. When it is falsy the node is skipped, as if it were not visible.
  Physics bodies ignore `condition`, because their simulation is precomputed.
- **Audio analysis for `audioAmplitude` and `audio:` links.**
  - Move audio loading before `sr_physics_prepare`, and run it in the preview
    path too.
  - Precompute, per video frame and per audio track, the RMS of the track's
    own post-fade signal:
    - overall;
    - `low` (< 250 Hz), `mid` (250 Hz – 4 kHz) and `high` (> 4 kHz), using
      fixed 4th-order Linkwitz-Riley crossovers run forward over the whole
      track.
  - The table is built once, so frames stay order-independent.
  - Values are linear amplitudes in [0, 1] at 0 dBFS. The table is cached next
    to the physics cache, keyed by the audio fingerprint.
- **`motionPath`.**
  - Position follows an SVG path over `[start, end]`, with `interpolation`,
    `constantSpeed` (arc length from B1-4), `autoOrient`, `orientOffset` and
    an animated `progress`.
  - The key attributes `spatialIn`, `spatialOut` and `roving`, deferred from
    B1-1, land here. Spatial bezier position keys share the same arc-length
    code.

### B2-3 Transform constraints and parenting (M)

- Node `@parent`, which is shorthand for `transformConstraint type="parent"`.
- `transformConstraint` types:
  - `parent`, `look-at` and `follow-path`;
  - `copy-position`, `copy-rotation`, `copy-scale` and `copy-transform`;
  - `distance`.
- Every type supports `influence`, `space`, the offsets, `progress` and
  `autoOrient`.
- `ik` (needs skeletons) and `track` (needs tracking data) are unsupported in
  this batch.
- Camera `target` (look-at) and `focusTarget`: the focus distance is the
  camera-space depth of the target at time `t`. The same goes for
  `transformConstraint` on lights and on `object3D`.
- **Order:** keys, then expressions and links, then constraints in document
  order (each blended by `influence`), then physics. World transforms of
  targets are computed through the new parent pointers. Parent cycles are load
  errors (the B2-0 graph).
- **Semantics** (owner decision, §6 Q2). Proposed, following After Effects:
  `@parent` replaces the transform inherited from the XML ancestors with the
  target's world transform. Opacity, masks, blending, isolation and draw order
  still come from the XML tree.

### B2-4 Symbols and instances (L)

- **Model: expand at load.** Each `instance` becomes a deep clone of the
  symbol's subtree under a synthetic group node that carries:
  - the time mapping: `local = (t − start) × speed + clipIn`, then
    `clipOut`, `loop` and `reverse` exactly as for layers; or `timeRemap`
    keys. This reuses the B1-5 group time path, because drawing already runs
    from `context->time`;
  - the symbol box: when the symbol has `width` and `height`, the group is
    isolated and clipped to the box and paints `background`. `fit`,
    `boxWidth` and `boxHeight` scale the box into the placement box. Without
    a size, the group passes through;
  - scoped ids `instanceId/innerId`, registered in the B2-0 index;
  - overrides applied to the clone after cloning. An override on an animated
    property replaces the animation with the constant value. Overrides are
    parsed with the target attribute's type from the property registry.
  - Why expand rather than draw by reference: drawing by reference needs
    per-instance override tables threaded through every draw function, while
    expansion keeps the render path unchanged and bit-identical. The cost is
    memory, bounded by `SR_MAX_NODES`.
- **Symbols may contain instances.** A symbol that instantiates itself,
  directly or indirectly, is a load error. Expansion counts nodes before
  allocating, so an exponential "instance bomb" fails fast.
- **Not allowed inside symbols in this batch** (reported as unsupported):
  - `rigidBody` and `softBody`: physics samples are owned per node and
    constraints use global ids;
  - `camera`;
  - node types from later batches (`transition`, `skeleton`).
- **S8 clones** (if approved): a layer override of `text` gives the instance
  its own copy of the text asset. The decoded glyph cache is per asset copy.

### B2-5 Include (M)

- `include src="other.xml"` parses the other document *into* the current
  scene through a new re-entrant entry point, not `sr_scene_load_xml`. The
  included file must:
  - pass the same XSD and semantic checks;
  - have a version no newer than the host's.
- **What is merged:** assets, paints, styles, effects, materials and symbols,
  under the namespace `@id/`. Each merged asset keeps its own base directory,
  so the single `scene->base_dir` becomes per-asset.
- **What is used:** with `@symbol`, that symbol; without it, the included
  composition becomes an implicit symbol sized and timed by the included
  project.
- **What is ignored, with a warning:** the included `project` except size and
  duration, plus `output`, `layouts`, `metadata` and `captions`.
- **What is an error:** included `physics` and `audioMix` (until batch 4),
  and `lights`.
- **Safety:**
  - `src` resolves relative to the including file, and absolute paths and
    URLs are rejected.
  - Paths that leave the top-level document's directory are rejected unless
    `--allow-external-includes` is given.
  - Include depth ≤ 8, and cycles are detected by canonical path.
  - `sha256`, when present, is checked before parsing.
  - The 64 MiB size cap applies to each file and to their total.
- The resume fingerprint (`resume.c:105-131`) covers every included file's
  bytes and every merged asset.

### B2-6 Repeat (S)

- `repeat count="N"` expands at load into N copies of its children, like
  instances.
- Copy *i* receives the cumulative offsets `i·offsetX`, `i·offsetY`,
  `i·rotationStep`, `scaleStepⁱ` and `opacity − i·opacityStep`, and a start
  delay of `i·timeStep`. Each copy is wrapped in a synthetic group, so its
  children's own transforms still apply.
- `from` and `step` shift `index`. Expressions inside a copy read `index` and
  `count`.
- Scoped ids `repeatId/i/innerId`. The same physics and camera restrictions
  apply as for symbols.
- `@over` and `@var` are unsupported until batch 3 (S13).

### B2-7 Minimal parameters (S)

Needed so that `param()` and `condition` are useful before full templating.

- `parameters/param` of type `string`, `number`, `boolean`, `color`, `enum`
  or `time`. `default`, `required`, `min`, `max`, `maxLength`, `pattern` and
  `options` are checked at load.
- `--param id=value`, repeatable. A value that fails the declared type or
  constraints is a load error.
- `{{name}}` substitution in text-asset `text` and `span` content, done once
  at load.
- `param(name)` in expressions and conditions, and `link source="param:name"`.
- **Deferred to batch 3:** `bind`, `data`, `variant`, the `list` and `asset`
  types, and `output/@variant`.

### B2-8 Responsive and remapped media (M)

- **`imageSequence` asset.**
  - Numbering by printf pattern or `####`; `first`, `last`, `step`, `fps`.
  - `missingFrame`: `error`, `hold`, `black` or `transparent`.
  - Frames decode lazily through a bounded LRU shared with stills. The resume
    fingerprint covers each member file.
- **Layer placement:**
  - `fit`: `none`, `contain`, `cover`, `fill`, `scale-down` or
    `contain-blur` (the blurred cover copy reuses the blur kernel);
  - `boxWidth`, `boxHeight`, `focusX` and `focusY`;
  - `cropLeft`, `cropTop`, `cropRight` and `cropBottom` (fractions of the
    source);
  - `flipX` and `flipY`.
- **Time:**
  - `freezeAt`.
  - The `timeRemap` element, with keys mapping local time to *absolute source
    seconds* as today's `source.time`; it replaces speed, reverse and loop.
    `source.time` stays as a 1.0 alias.
  - `frameBlend="frame-mix"`: a linear mix of the two neighbouring source
    frames by the fractional position, copying the first frame before fetching
    the second (§2).
  - `optical-flow` is unsupported.
- **Video asset attributes:**
  - `alpha` (`straight` or `premultiplied`), `pixelAspect`,
    `rotation` (90/180/270), and `timecodeStart` (stored and written as
    output timecode).
  - `hasAudio`, `audioStream`, and layer `volume`/`mute`: the video's own
    audio is mixed into the master as an implicit track. `audioBus` is
    unsupported until batch 4.
- **Asset handling:**
  - `representation` children, selected by `output/@representation` or
    `--representation`, falling back to `src`.
  - `assetProvenance/@sha256` is verified at load for every asset kind that
    has it. The hash is recorded in the resume fingerprint in place of a
    re-read.
- **Unsupported:** `stabilize`, image `layer` (PSD layers and EXR parts), and
  transfers other than `auto`, `srgb` and `linear` (colour management is
  batch 5).

## 5. Moved to later batches

Batch 3 is expected to contain:
- full templating: `variant`, `bind`, `data`, `repeat/@over`, layouts and safe
  areas;
- text: spans, styles, `autoFit`, text animators (which enable
  `textIndex`/`textTotal`) and text on path;
- transitions, together with `sequence/@transition`.

These all consume what batch 2 provides: expressions, overrides, instances
and parameters. The rest of the plan (B1 §5) is unchanged: audio graph,
captions, the effects catalogue, colour management, motion blur, 3D and the
new asset kinds.

## 6. Owner decisions

Adopted 2026-09-25: the owner accepted every recommendation below (see
`docs/schema-1.1-batch3-proposal.md` §2).

1. **Expression runtime errors** (B2-1). Fail the render (recommended), or
   hold the keyed value with a warning by default?
2. **Parent semantics** (B2-3). Should `@parent` replace the XML-ancestor
   transform, After Effects-style (recommended)? The alternative is to compose
   it with the ancestor transform.
3. **Per-instance text** (S8). Should a layer override of `text` clone the
   asset per instance (recommended)? The alternative is to wait for batch 3
   templating.
4. **External includes** (B2-5). Confine includes to the document's directory
   tree unless a CLI flag says otherwise (recommended)?
5. **Schema edits S6 and S9** (scoped override targets and two-argument
   `prop`). Commit them with the B1 §2 fixes before B1-0 starts, so the
   schema changes only once?

## 7. Verification plan

- **Frame-order independence** (new CTest, required for every batch 2
  golden). Render the scene sequentially, in shuffled single-frame previews,
  in `--range` slices, and through `--resume` with killed segments; compare
  SHA-256 per frame. This is the test that catches hidden state in
  expressions, links, smoothing and audio analysis.
- **Thread invariance.** 1 vs 4 threads, byte for byte, as for the existing
  goldens.
- **Expression engine:**
  - unit tests per built-in against closed-form references;
  - a seeded grammar generator that produces random valid and invalid
    expressions, run under ASan and UBSan;
  - OOM injection (`unit.oom`) extended to the compiler and to instance
    expansion.
- **Limit tests:** instance bomb, include cycle, include depth, `SR_MAX_NODES`,
  expression operation budget, `prop()` cycles, and parent cycles. Each must
  fail at load with a precise diagnostic.
- **Compatibility.** All 1.0 scenes and all batch 1 goldens stay
  byte-identical. The B2-0 refactors (index, property pipeline, `random.c`)
  land first and alone, and are verified with the 120-frame equivalence
  oracle before any feature is built on them.
- **New goldens:**
  - `expressions` (wiggle, loopOut, spring, prop chain);
  - `links-audio` (a synthetic tone generated by `media_synth.h`);
  - `motion-path`;
  - `parenting-constraints`;
  - `symbols-instances` (nested, overrides, remap);
  - `include`;
  - `repeat`;
  - `params-condition`;
  - `media-fit-crop`;
  - `image-sequence`;
  - `frame-mix`.
- **Coverage gate** at the current floors (88.30 % lines / 69.29 %
  branches). The expression VM and parser count toward the floors from the
  start.
