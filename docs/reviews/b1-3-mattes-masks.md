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
5. Effect scratch keeps the existing thread-local caches; bounded renders
   reserve a conservative scratch bundle and pass work on the calling thread
   for each effect call and release it afterwards.

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
