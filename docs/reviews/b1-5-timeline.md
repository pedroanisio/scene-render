# B1-5 timeline structure: evidence

Branch `b1-5-timeline` (worktree `/tmp/scene-render-timeline`), based on
`4106b84`. Spec: `docs/schema-1.1-batch1-proposal.md` §4 B1-5 and §2 S4.
Design: `docs/design/b1-5-timeline.md` (Codex-reviewed before code).

## Proposal bullets

| Bullet | Implementation | Commits |
|---|---|---|
| `markers/marker` | `src/markers.c` table (document order + sorted id index), `src/xml_timeline.c` parser; id, time, duration, kind (`cue`/`section`/`beat`), label, color (tokens) | `ba5c278` |
| `beatGrid`, generated `beat.N`/`bar.N` (S4) | 1-based virtual ids bounded by `SR_MAX_GENERATED_MARKERS`; collision check against every document id before references resolve; one grid per document | `ba5c278`, `a7123e6` |
| Node `startMarker`/`endMarker` | resolved in `xml_resolve.c` → `sr_xml_resolve_timeline`, kind checks, exact composition endpoints | `ba5c278`, `72a03b3` |
| Key `marker` snapping | `SrKeyframe.marker`; deferred validation; snap on the track's own clock, re-sort, same validation as literal tracks; all hosts (nodes, masks, modifiers, points, materials, lights, effects, cameras, fields, 3D objects, audio tracks) | `ba5c278` |
| Group `timeOffset`/`timeScale` | children clock `(P - s) * q + s + o`, nested composition, all three time bases, video speed, particle clock, physics guard | `ba5c278` |
| `sequence` with `timeGap` | slot placement in document order, slot-relative subtrees, overlap with negative gaps, item `endMarker`, exact junctions | `ba5c278`, `a7123e6`, `72a03b3` |
| `transition`/`transitionDuration` unsupported | capability rows stay unsupported; loader also rejects | `ba5c278` |
| Node `name` and `tags` | `SrNodeTimeline`; bounded; used in every B1-5 diagnostic | `ba5c278` |
| New animatable properties | none: the schema does not make `timeScale`/`timeOffset` animatable (documented) | — |
| Golden `sequence-markers` | `tests/golden/sequence-markers.xml`, frames 0, 12, 18 | `ba5c278` |

Documentation: `docs/xml-reference.md` "Timeline structure (1.1)", the B1-1
composition-time-base refinement, golden README row, regenerated
`docs/feature-matrix.md` (`e42eb2f`, `72a03b3`).

## Capability entries flipped

`scene/markers`, `markersType/marker`, `markersType/beatGrid`,
`compositionType/sequence`, `groupType/sequence`; `@name`, `@tags`,
`@startMarker`, `@endMarker` on `groupType`, `layerType`, `shapeType`,
`particleEmitterType`; `groupType/@timeOffset`, `groupType/@timeScale`,
`sequenceType/@timeGap`, `keyType/@marker`; `markerType` `@id`, `@time`,
`@duration`, `@kind` (values `cue`, `section`, `beat`), `@label`, `@color`
(form `token`); `beatGridType` `@bpm`, `@offset`, `@beatsPerBar`.

`sequenceType` inherits every implemented `groupType` row through the new
`inherits` key in `schema/capabilities.json` (implemented in
`tools/schema_inventory.py`), so group constructs other lanes flip apply to
sequences automatically.

Still unsupported, by the proposal or later batches: `sequenceType/@transition`,
`@transitionDuration` (B3-C), marker kinds `chapter`, `comment`, `todo`, `cta`
(B3-H), `beatGridType/@source` (no batch assigns tempo analysis yet),
`audioTrackType/@startMarker` (B3-G), B1-5 attributes on node types that are
themselves unsupported (`adjustment`, `instance`, `include`, `repeat`, text
nodes).

## Tests

- `unit.markers` (15 cases): beat/bar closed forms, boundary counts, exact
  bar/beat equality for a non-dyadic tempo, canonical id syntax, the
  generated-id limit at exactly 1048576 and one over, marker table and
  lookups, marker count/label/time limits, collisions (marker, node, asset,
  effect ids), version gate, unsupported kinds/source; nested group clocks
  against hand-computed maps for composition/local/normalized tracks;
  combined-scale and offset limits; default groups keep absolute children
  (B1-1 correction); sequence placement with gaps/overlaps, slot-relative
  keys, open last items, missing-end and startMarker errors, item endMarker,
  transition rejections; exact junctions under a scale-3 and a scale-10
  clock including descendants of a group item; marker endpoints exact under
  scale 10; exclusivity and interval errors; unknown/not-a-marker/out-of-grid
  diagnostics; key snapping on nodes, colour channels, a scaled local track
  and a material, negative offsets, re-sorting, duplicate-after-snap and
  negative-result errors; names/tags with limits and duplicates; video speed
  under scale 2; physics guard and offset allowance; particles: keyed rate
  under timeScale 2 at t is byte-identical to the plain emitter at 2t (four
  times), and a dyadic 0.25 offset is byte-identical at three times;
  `tests/data-timeline.xml` (every construct) loads, resolves and renders.
- `unit.fuzz_markers`: 4 seeds x 12 samples of generated documents (markers,
  grid, sequence items with tags/names, all time bases, valid and invalid
  references) plus one mutation and one truncation each, loaded twice to check
  determinism; 4096 seeded generated-id strings against the canonical form.
- `unit.oom`: load replay of `tests/data-timeline.xml` and the golden scene
  (every allocation fails once; no leak, `SR_ERR_MEMORY` or clean XML error).
- Golden `sequence-markers`: 1 and 4 threads, warm-frame check, and
  `frame_order` (sequential, shuffled, sliced, resumed).
- `tests/test_verification_tools.py`: 1-based generated ids through the
  project end in `tools/validate-scene.py`.

Every Codex finding was reproduced as a failing test before its fix
(`a7123e6`: marker junction, legacy start acceptance, sequence shift bound;
`72a03b3`: descendant junction).

## Visual check of the golden

Frames 0, 12 and 18 (t = 0, 0.5, 0.75 s) were inspected, and object extents
measured from the PNGs matched hand computation:

- Row 1 (gapped sequence): frame 0 shows the red item at x=10; at t=0.25 the
  row is empty (gap until 0.333); at 0.5 the yellow ellipse sits at x=125
  (normalized over its open end mapped into the slot: 110 + 60 * 0.25); at
  0.75 the green item has started (x=229) with its local mask narrowed to
  13 px.
- Row 2 (overlap -0.125): blue alone at 0; blue+violet overlap region at 0.5
  shows violet ramping in; at 0.75 violet and pink overlap (pink starts at
  exactly 0.75).
- Row 3 (timeScale 2, timeOffset 0.25, starting at 0.25): the teal square is
  absent at 0, at x=61 at 0.5 and x=112 at 0.75 (child clock 2t); the
  particle plume is visible from 0.25 and rises at double speed.
- Row 4: the yellow bar (startMarker `hit`, endMarker `out`) appears at 0.5
  and is visible at 0.75; the white/orange ellipse is at 100 (bar.1), 150
  (beat.2), 120 exactly at beat.3 with its fill stepped to orange at `hit`,
  and eases toward 290.

## Verification numbers

- Release CTest: 84/84 (includes `frame_order` over all 23 goldens).
- ASan/UBSan CTest (`ASAN_OPTIONS=detect_leaks=0`): 84/84.
- Byte oracle (`tools/equivalence-oracle.sh /tmp/scene-render-b1-reference`
  vs the `72a03b3` Release binary): 309/309 previews across 42 scenes at
  threads 1/3/22, 3/3 encodes, 2 expected rejections. No existing golden
  or `tests/golden.sha256` hash changed (`SR_UPDATE_GOLDEN=1` rewrote only
  the three new references).
- Coverage gate: 92.27% lines / 77.49% branches (base `4106b84`: 92.25% /
  77.25%); `markers.c` 95.65% / 80.13%, `xml_timeline.c` 91.13% / 80.96%.
  Branch floor raised 75.25 -> 75.45; line floor stays 90.30 (already about
  2 points below). Uncovered lines are defensive checks the XSD or the
  capability pass reject first (negative durations, unknown kinds, invalid
  NMTOKENs, `transition`, `source`) and allocation-failure branches outside
  the replayed fixtures.
- Performance (`scripts/perf-check.py`, not a pass): the fixed-baseline run
  failed at TOTAL +88.4% with every stage marked noisy; machine load average
  was about 15 (other lanes building and testing). Alternating runs of the
  pre-batch reference and this binary gave TOTAL 17.91 / 21.60 / 21.72 /
  24.25 CPU s (ref, cur, ref, cur) against the 10.44 s baseline, rising
  monotonically with load, so no per-stage delta can be attributed. By
  design the render path changes only in `particles.c` (the perf scene has
  2 emitters): one multiply/divide by a clock rate of exactly 1.0 per time
  conversion; all timeline work is at load. The baseline was not updated;
  the 2% gate stays open for a quiet-machine measurement.

## Deviations and decisions

- **Composition time base inside re-timed groups.** B1-1 said composition
  keys use project seconds. Inside a timed group or a sequence they use the
  enclosing (nested) timeline; otherwise a sequence could not carry
  animated content. Identical for every document without the new
  constructs. Recorded in `docs/xml-reference.md` and both design notes.
- **B1-1 local-clock correction.** B1-1 added ancestor group starts to the
  local clock at parse time, which disagreed with node visibility (group
  starts do not shift children in 1.0). No repository scene uses that
  combination and B1-1 is not batch-merged, so the resolver computes the
  correct clock; a regression test pins it. Codex rated keeping the old
  behaviour a blocker for byte identity; the oracle shows no fixture change
  and the old value was a defect, so this is a deliberate, reported change.
- **Beat numbering** is 1-based (musical convention; the schema does not
  say). `tools/validate-scene.py` used 0-based ids and was aligned.
- **Sequence items:** `startMarker` is rejected (the sequence places the
  item); `endMarker` is converted into the slot. `object3D`/`camera` inside
  a sequence keep their scene-global timing, as in any group.
- **Physics under timeScale != 1:** dynamic/kinematic rigid bodies and soft
  bodies are "unsupported in this build" (simulation runs on the project
  clock); offsets are allowed and keep existing semantics.
- **Duplicate tags** are an error (the XSD allows them).
- **Load-order:** the timeline pass runs after the lengths preflight
  (which bounds node and mask counts for recursive consumers) and before
  every consumer of node intervals and clocks.
- **Assets:** `assets/generated/` (git-ignored) was copied from the main
  checkout into the worktree so the oracle's examples could render.

## Files outside the lane's own modules

Shared files touched (merge attention): `include/scene_render/scene.h`
(SrNode timeline/clock fields, SrScene timeline pointer),
`include/scene_render/timeline.h` (`SrKeyframe.marker`), `src/scene.c`,
`src/xml.c` (dispatch rows), `src/xml_internal.h` (element kinds,
prototypes), `src/xml_nodes.c` (allowed attributes, hook),
`src/xml_elements.c` (key marker), `src/xml_animation.c` (node clocks
deferred; shared validator), `src/xml_resolve.c` (one call),
`src/particles.c` (clock conversion), `schema/capabilities.json`,
`src/xml_capabilities_data.inc`, `docs/feature-matrix.md`,
`tools/schema_inventory.py`, `tools/validate-scene.py`, `CMakeLists.txt`,
`Makefile`, `tests/unit/harness.h`, `tests/unit/test_main.c`,
`tests/unit/test_golden.c`, `tests/unit/test_oom.c`,
`tests/test_verification_tools.py`, `docs/xml-reference.md`,
`docs/design/b1-1-animation.md`, `tests/golden/README.md`.

New modules: `include/scene_render/markers.h`, `src/markers.c`,
`src/xml_timeline.c`, `tests/unit/test_markers.c`,
`tests/unit/test_fuzz_markers.c`, `tests/data-timeline.xml`,
`tests/golden/sequence-markers.xml` and its three references.

Integration notes for other lanes: any new node type that should accept
`name`/`tags`/markers must add them to `sr_xml_start_node`'s allowed list
and flip its capability rows; new animatable hosts must be registered in
the property registry (the resolver enumerates tracks through it) or a key
marker on them fails with "unsupported in this build: key marker on this
animation host".
