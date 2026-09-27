# B1-3 completion: operators, advanced masks, mattes, adjustments

Branch `b1-3-mattes-masks`, worktree `/tmp/scene-render-masks`, based on
`main` at `4106b84` (which contains the earlier B1-3 work merged at
`d524376`). Spec: `docs/schema-1.1-batch1-proposal.md` §4 B1-3; contract:
`docs/design/b1-3-compositing.md` and `docs/design/b1-compositing-resources.md`.

## Proposal bullets and commits

| Proposal bullet | State | Commit |
|---|---|---|
| Blend modes: separable, nonseparable, darker/lighter-color | done earlier (22 modes) | pre-`d524376` |
| Blend: `dissolve` seeded from node id and project seed | done | `65199a5` (render), `e0edb83` (enable) |
| Blend: stencil/silhouette alpha+luma, `alpha-add`, `behind` within the isolated parent | done | `65199a5`, `e0edb83` |
| Skew | done earlier | pre-`d524376` |
| Track mattes: `matte`, `matteMode` (4 modes), `matteVisible`; coverage buffer in parent space; hidden unless visible; cycle is a load error | done | `65199a5`, `e0edb83` |
| Masks: `mode` (7), `opacity`, `feather`, `expansion` | done | `65199a5`, `e0edb83` |
| Masks: `path`, `polygon`, `star` reusing `vector_path.c` | done | `65199a5`, `e0edb83` |
| Adjustment node: effects on the composite below inside the parent buffer, reusing group effects; masks, matte, opacity limit it | done | `65199a5`, `9631239`, `e0edb83` |
| Goldens `blend-modes` (complete), `mattes-masks`, `adjustment` | done: `blend-operators.xml` completes the existing `blend-modes.xml` family (the old references may not move) | `e0edb83` |

Commits:

- `65199a5` feat(render): B1-3 operators, advanced masks, mattes and adjustments
- `9631239` fix(render): account card depth-of-field scratch; reject adjustment cards
- `e0edb83` feat(loader): enable B1-3 operators, masks, mattes and adjustments
- (this file, design-doc status, coverage floors) docs/chore commit

## Implementation summary

- **Operators** (`raster.c`, `compositor.c`): a node with an operator blend
  is drawn with its own effects/masks/matte/opacity and a normal root blend
  into a pool buffer, then one `SR_OP_OPERATOR` acts on the parent target.
  Stencil covers the whole receiving clip, the others their image (they are
  identities where the source is transparent). Operator and adjustment
  children isolate their non-root parent automatically (static, including
  inactive children). Luma variants use physical Y with the working gamut
  row (`sr_color_luminance_row`) and transfer decode. Dissolve uses
  `sr_random_property_seed` (canonical FNV-1a id, 0, "blend.dissolve",
  XOR project seed) and `sr_random_pixel_value` over composition pixels,
  mapped through the plane homography inside projective cards.
- **Advanced masks** (`compositor_coverage.c`, `mask_outline.c`): node-local
  raster at one sample per local pixel, cropped to the inverse-mapped clip
  intersected with the masks' extent plus halo, with a constant exterior
  value; per mask: analytic or exact-area path coverage, disk morphology
  (van Herk rows), three-box feather with fractional blending, inversion,
  opacity, mode combination. Legacy intersect-only masks (including
  explicit defaults) keep the analytic 1.0 path. Polygon/star outline
  generation is isolated in `mask_outline.c` (`sr_mask_outline`) for B1-4 to
  share. Path masks are parsed at preparation into scene-owned immutable
  `SrMaskPath` geometry; `sr_prepared_path_fill_offset` (new, additive in
  `vector_path.c`) rasterizes translated contours into caller scratch.
- **Mattes** (`compositing.c`, `compositor.c`, `compositor_matte.c`,
  `xml_compositing.c`): sorted-index id resolution before sorting;
  preparation builds sources, static-OR suppression, consumer lists and a
  bounded dependency graph (containment and adjustment-prefix edges,
  iterative colored DFS, cycle diagnostics with the id chain). Each frame
  renders every needed source once, in dependency order, through a
  restricted traversal (ancestors: placement/opacity/visibility/lifetime
  only) into a composition-sized scratch, reduced to (alpha, alpha*Y).
  Consumers sample through a matte link in the ordinary coverage chain;
  groups with mattes isolate.
- **Adjustment** (`compositor.c`): flush, copy the backdrop over the clip
  grown by the effects' reach, apply the group effects, `SR_OP_REPLACE`
  writing `D + w*(blend(D,F)-D)`. As a matte source, the parent prefix is
  replayed (root: on the project background) and the image is `w*F`.
- **Ledger** (in scope): coverage grids and all mask scratch, operator/
  replacement work, capture scratch/store/table, adjustment copies,
  reservations for group/card/adjustment effects and card DOF blur.
  **Out of scope** (documented in `xml-reference.md` "Compositing resource
  limits" and the resources design): lighting/shadow scratch, renderer outer
  targets, viewport extraction and global effects. These are pre-existing
  consumers not used differently by B1-3 features; the ledger is therefore
  not claimed as a complete per-frame bound.

## Deviations from the design contract

1. `object3D` is rejected as a matte source ("is not a group, layer, shape,
   particleEmitter or adjustment") instead of a private per-object lighting
   pass. The proposal bullet does not require 3D sources.
2. Adjustment layers do not support `threeD` (capability stays unsupported;
   direct-C preparation rejects `card`). The proposal bullet does not
   require adjustment cards.
3. Captures and adjustment-prefix replays contain 2D content (and the
   project background for root prefixes) but not 3D objects; card sources
   in captures are near/far clipped only (no shared depth read or write).
4. A card ancestor on a capture path keeps its camera depth-of-field blur
   (it is placement) but not its effects, masks or matte.
5. Legacy effect calls keep the existing thread-local scratch caches.
   Bounded renders run each effect in a private scratch scope (review
   finding 3) with a conservative reservation of its scratch and pass work.

## Tests

New suites: `blend_operators` (7), `mask_advanced` (11), `matte` (7),
`adjustment` (7), `fuzz_mask_xml` (1200 seeded valid/mutated/truncated
documents, loaded and rendered at 1/4 threads). References are closed form
(operator formulas, W3C/renderer mask combination table, luma rows, FNV-1a
vectors) or independent implementations (1D zero-exterior box^3 feather
reference, disk-membership expansion cases, exact-area path pixels,
group-effect equivalence for adjustments). Also: loader and preparation
diagnostics with attribute and line, version gates, runtime range failures
(overshooting feather track), coverage-dimension limit, 1/4-thread and
shuffled-time identity, scene immutability, direct-C preparation contracts.
`unit.oom`: XML load of `tests/data-compositing.xml` (97 allocations) and
its preview render at 1 and 3 threads (87 allocations each), all returning
`SR_ERR_MEMORY` without leaks.

Goldens (visually reviewed at frames 0/12/23):

- `blend-operators.xml`: all seven operators; checked stencil shows the
  backdrop only inside the source and clears outside, silhouettes cut holes
  to the background, luma variants follow the grey bars, alpha-add and
  behind composite as described, dissolve scatters with opacity (dense at
  frame 12, sparse at 0) and keeps a fixed per-pixel pattern, the card tile
  stencils and dissolves inside the projected plane.
- `mattes-masks.xml`: six mode tiles show intersection/union/difference/
  xor-like combinations of the ellipse and moving rect; path ring with
  triangular hole blurs as feather grows; polygon grows by expansion; star
  radius/innerRadius animate (automatic and tracked); inverted rounded-rect
  at 0.6 opacity; alpha/inverted/luma/luma-inverted consumers follow their
  moving sources and bars; only the `matteVisible` source is drawn.
- `adjustment.xml`: blur confined to the growing feathered star; greyscale
  confined to the moving luma lens (lens itself hidden); the red rect above
  the adjustment stays saturated; multiply tint inside the panel strengthens
  with opacity and leaves the later green rect untouched; vignette appears
  from 1 s.

## Verification

- Release: `ctest` 87/87 passed (including `frame_order` over all 24
  goldens and the new ones, `unit.golden` 24/24 with existing references
  byte-identical, integration, OOM).
- ASan/UBSan (`build/asan`, `ASAN_OPTIONS=detect_leaks=0`): 87/87 passed.
- Coverage gate: 92.39% lines / 78.12% branches; floors raised from
  90.30/75.25 to 90.35/76.10. New files: compositor_coverage.c 90.89/82.46,
  compositor_matte.c 100/87.50, xml_compositing.c 89.76/86.14,
  mask_outline.c 100/100, compositing.c 94.90/86.37.
- Oracle: `tools/equivalence-oracle.sh /tmp/scene-render-b1-reference
  build/scene-render`: 309/309 previews across 42 scenes, 3/3 encodes,
  2 expected rejections (run on the core commit and on the final tree).
  `tests/golden.sha256` unchanged.
- Performance: `scripts/perf-check.py --tolerance 0.02` could not produce a
  usable measurement on the shared machine (load average 16-18 from other
  lanes): every stage, including untouched lighting and effects, read
  +50-100% against the owner baseline. Alternating A/B runs of the base
  commit `4106b84` and this branch on `benchmarks/perf-scene.xml` gave
  composite medians base 2.49/3.44/4.21 s versus branch 2.93/3.81/2.70 s:
  the base binary alone varied by 70%, so no regression or speedup can be
  claimed. Legacy scenes take no new traversal or allocation; the only
  legacy hot-path change is one extra pointer test per mask link in
  `sr_link_coverage` and larger queued op/link records. The baseline was not
  updated. The strict 2% gate remains to be rerun on a quiet machine.

## Files outside the compositor area

`include/scene_render/scene.h` (blend enum, `SR_NODE_ADJUSTMENT`, mask and
matte fields), `raster.h`, `random.h`, `color.h`, `property.h`; `src/xml.c`
(dispatch entry, `E_ADJUSTMENT`), `xml_internal.h`, `xml_elements.c` (mask
handler, animate host), `xml_nodes.c` (matte attributes, exported effect-id
parser), `xml_resolve.c`, `xml_lengths.c`, `length_frame.c`, `property.c`,
`scene.c`, `vector_path.c`/`vector_path_internal.h` (additive offset fill),
`color.c`, `random.c`, `raster.c`; `schema/capabilities.json` and generated
files; `CMakeLists.txt`, `Makefile`; `tests/unit/harness.h`, `test_main.c`,
`test_golden.c`, `test_oom.c`, `test_blend_color.c`; `tools/coverage-gate.sh`.

## Diff review (Codex, 4106b84..53de26e)

The read-only Codex review reported 9 major and 1 minor findings. Each was
reproduced as a failing case in `tests/unit/test_b13_review.c` (suite
`b13_review`): built against `53de26e`, all ten cases fail; on this branch
all pass. All findings were accepted; none was rejected.

| # | Finding | Test | Fix |
|---|---|---|---|
| 1 | Effect/DOF memory reservation leaked when the following work check failed | `work_rejection_releases_reservation`: seven work limits in the effect's window; ledger balance after scope end must be zero, failure owned by `adjustment@effects` (pre-fix: 3,244,288 bytes / 548,864 pixels outstanding) | `b6b0ae6` `sr_effect_bounded` admits memory and work together and releases on rejection |
| 2 | Path-mask work ignored horizontal edge spans | `path_edges_charge_columns`: 400 shallow edges across 4096 columns must charge at least edges x columns x 4 | `6b6695e` per-edge clipped row span and column charge |
| 3 | Bounded effects borrowed warmed thread-local scratch larger than their reservation | `effects_use_private_scratch`: after a 1024x1024 legacy blur warms the cache, a bounded 32x32 adjustment reports identical peaks and output cold and warm, the legacy cache is intact and legacy output is unchanged (pre-fix: no private scope exists) | `b6b0ae6` private scratch scope (`sr_effects_private_begin/end`): bounded calls allocate their own scratch for the call's lifetime, retained bytes are checked against the reservation; legacy calls keep the cache and identical arithmetic (oracle below) |
| 4 | Inactive/invisible/zero-opacity adjustment matte source exposed its prefix | `absent_adjustment_source_is_empty` (three states) | `b6b0ae6` activity check before prefix replay |
| 5 | Masked adjustment chains processed incomplete intermediate images | `masked_chain_uses_complete_inputs`: grey then blur inside a small mask over red must stay grey | `b6b0ae6` effect i processes the region grown by the reach of later effects |
| 6 | Ancestor opacity folded into the source before its own rendering | `ancestor_opacity_after_capture`: opaque dissolve source in a 0.5 group gives uniform 0.5 coverage | `b6b0ae6` capture scaled once by the ancestor product after rendering |
| 7 | Flattened operator cards lost their depth writes | `flattened_card_writes_depth`: opaque dissolve card vs normal card over an intersecting card, same occlusion away from edges | `b6b0ae6` card depth test/write deferred to the operator op on the final source |
| 8 | Adjustment prefix replay ignored card-run sorting | `adjustment_prefix_sorts_cards`: near white over far black gives full luma coverage | `b6b0ae6` prefix replay through `sr_draw_children_limit` |
| 9 | Parent masks clipped an adjustment's input pixels | `parent_mask_keeps_adjustment_input` | `b6b0ae6` isolated parent fill grown by adjustment reach before its masks |
| 10 | Additive star innerRadius used base 0 | `additive_inner_radius_base`: radius 20 plus additive 2 equals innerRadius 12 | `6b6695e` omitted innerRadius base is half the static outer radius |

Codex's open question (object3D matte sources and adjustment cards) is
answered by the deviations listed above; those are declared limitations
awaiting owner acceptance, not fixed here.

Post-review verification (tree at `5294c88` plus this doc): SDK Release
88/88 CTests (including `frame_order` over all 24 goldens), ASan/UBSan
88/88, coverage gate 92.47% lines / 78.20% branches (floors 90.35/76.10),
oracle against `/tmp/scene-render-b1-reference` 309/309 previews, 3/3
encodes, 2 expected rejections. Golden references and
`tests/golden.sha256` are unchanged by the fixes. The strict performance
gate still needs a quiet-machine run.

## Merges

- `90caac6` merges main `d29c3d2` (B1-5 timeline, B1-6 outputs). Conflicts
  in `scene.h`, `xml_internal.h`, `xml_nodes.c`, `xml_resolve.c`,
  `capabilities.json` and `coverage-gate.sh` were resolved keeping both
  sides; main's floors (90.80 / 76.50) kept. B1-5 integration: adjustment
  accepts `name`, `tags`, `startMarker`, `endMarker` (allowed list and
  capability rows); adjustment and the new mask properties are registry
  hosts, so key markers resolve on them; `sequenceType` inherits the
  groupType matte, blend and adjustment rows; sequences and re-timed groups
  are matte sources (`b13_review.sequence_and_clock_matte_sources`,
  `adjustment_timeline_attributes`).
- `62a0c58` merges main `1e17ede` (B1-4 shapes and paints). Conflicts in
  the build files, `property.h/.c`, `xml_internal.h`, `xml_nodes.c`,
  `vector_path_internal.h`, `compositing.c`, `compositor.c` and
  `capabilities.json` resolved keeping both sides (both op kinds and row
  kernels; extended shapes are a compositing feature named "shape").
  Outline deduplication: `mask_outline.c` now builds polygon/star vertices
  with B1-4's `sr_shape_vertex`; all goldens, including `mattes-masks`,
  remain byte-identical.

## Follow-up review (Codex, 53de26e..f2a8dc5)

Seven fixes were confirmed complete. The remaining items, each reproduced
first (the four behaviour cases fail on `62a0c58`; the finding-3 cases fail
when the private scopes are removed from `sr_effect_bounded`):

| Item | Test | Fix |
|---|---|---|
| 9 (card groups) | `card_parent_mask_keeps_adjustment_input` | `21f9ed8`: card groups with adjustment children draw them unmasked into an isolated buffer over the grown clip, then apply the group masks |
| 3 (test strength) | `bounded_effects_allocate_privately` (aligned_alloc observation of a real bounded render with a large warmed cache; exact pointer/capacity of a small warmed cache after a larger bounded call; nested scopes), `oom.private_effect_scratch_survives_allocation_failures` (12 injected failures, cache unchanged after every replay, no leaks) | `dac2ce4` (tests; `--wrap=aligned_alloc` makes effect scratch injectable) |
| A (quadratic suffix reach) | `adjustment_effect_reference_limit` (256 accepted and rendered, 257 rejected) | `21f9ed8`: `SR_MAX_ADJUSTMENT_EFFECTS` at load and preparation; one charged reach pass |
| B (empty matte on stencil card) | `empty_matte_stencil_card_clears` | `21f9ed8`: deferred depth test recorded before the matte |
| C (HDR at full coverage) | `full_coverage_card_operator_is_exact` (1e8 backdrop, exact white) | `21f9ed8`: copy at coverage >= 1 |

Codex's note on finding 4: the zero initial capture scale (finding 6) also
empties an inactive capture, so the activity guard is now defence in depth;
the test still covers the observable behaviour.

## Final evidence (tree `dac2ce4`, after both merges)

- SDK Release CTest 100/100, including `frame_order` over every golden
  (all lanes), `unit.golden`, `integration` (B1-6 outputs, 1 vs 4 threads,
  `tests/golden.sha256`) and `unit.oom`.
- ASan/UBSan CTest (`ASAN_OPTIONS=detect_leaks=0`): 100/100.
- Coverage gate: 93.03% lines / 79.14% branches (floors 90.80 / 76.50).
- Byte oracle vs `/tmp/scene-render-b1-reference`: 309/309 previews across
  42 scenes, 3/3 encodes, 2 expected rejections.
- Relative to main, only the nine B1-3 golden references are added;
  `tests/golden.sha256` and every other reference are unchanged.
- The strict 2% performance gate still needs a quiet-machine run.
