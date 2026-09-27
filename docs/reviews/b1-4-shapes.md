# B1-4 shapes, strokes, trims and paints: evidence

Branch `b1-4-shapes`, worktree `/tmp/scene-render-shapes`, based on `4106b84`.
Contract: `docs/schema-1.1-batch1-proposal.md` §4 B1-4 and
`docs/design/b1-4-shapes-paints.md`. All builds and tests ran in Flatpak
`org.freedesktop.Sdk//25.08`.

## Proposal bullets and commits

| Proposal bullet | Implementation | Commits |
|---|---|---|
| Geometry prerequisites: arc-length parametrisation, coverage clipping | `vector_path.c` measure/extract, `sr_coverage_clip`, mapped rasterization and its work bound; legacy entry points untouched | `9cf9b88` |
| Shape node: polygon, star (inner/outer radius and roundness), line, path, cornerRadii, fillRule | `vector_shape.c` constructors, `xml_shapes.c` parsing/validation, `compositor_shape.c` rendering; also `rounded-rect` | `9cf9b88`, `8ea0196`, `1ea8f59` |
| strokeCap, strokeJoin, miterLimit, dash, dashOffset | `vector_stroke.c` polygon stroker and dashes (phase along the contour; joined across closed seams) | `9cf9b88`, `fb555c1`, `02359aa` |
| strokePosition inside/outside via coverage clipping | double-width stroke × fill coverage / (1 − fill) | `9cf9b88`, `8ea0196` |
| paintOrder | fill-stroke / stroke-fill composition in the sample path and vector assets | `8ea0196` |
| Trim paths trimStart/End/Offset/Mode, animatable, in the registry | `sr_stroke_trim`, registry rows with `SR_PROPERTY_SHAPE_STYLE` | `9cf9b88`, `8ea0196` |
| Vector assets rounded-rect, polygon, star, line (`svg` deferred) | `procedural.c` extended rasterization at asset load | `8ea0196`, `fb555c1` |
| paints: linear, radial (focal, aspect), conic; spread, units, rotation, animated stops | `paint.c`, gradient/stop registry hosts | `9cf9b88`, `8ea0196` |
| interpolationSpace linear/srgb/oklab/oklch | premultiplied interpolation; oklch implemented with the CSS shorter-hue rule, powerless hue below chroma 1e-6 and an unadjusted exact 180° | `9cf9b88`, `fb555c1` |
| Seeded ordered dither | 8×8 Bayer, offset from `splitmix64(FNV-1a64(id,0,"paint.dither") ^ seed)` | `9cf9b88` |
| url(#id) on shape fill/stroke, vector fill/stroke, project background | `xml_shapes.c` references and resolution; `renderer.c` paint background | `8ea0196` |
| New animation hosts (B1-1) for these constructs | shape rows radius, innerRadius, outerRadius, inner/outerRoundness, trimStart/End/Offset, dashOffset; gradient x1 y1 x2 y2 cx cy r fx fy fr aspect angle rotation; stop offset/color/opacity/midpoint (all require 1.1) | `8ea0196` |
| Goldens shapes-strokes, gradients | `tests/golden/*.xml` and references | `1ea8f59`, `fb555c1` |
| Design note | `docs/design/b1-4-shapes-paints.md` | `71e054a` |

Deferred as the proposal states: `vector/@shape="svg"`, `meshGradient`,
`pattern`. Out of scope (other items): `shapeModifier`, path/polygon/star
masks (B1-3), `keyType/@value=paint-reference`, shape `effects`.

## Capability entries flipped

`scene/paints`; `paintsType/{linear,radial,conic}Gradient`;
`{linear,radial,conic,gradientBase}GradientType/{stop,animate}` and all their
attributes and values (spread, units, interpolationSpace incl. oklch);
`stopType/@offset|color|opacity|midpoint`, `stopType/animate`,
`stopType/@color=token`; `shapeType/@shape=rounded-rect|polygon|star|line|path`;
shape attributes path, fillRule, radius, cornerRadii, points, innerRadius,
outerRadius, innerRoundness, outerRoundness, strokeCap, strokeJoin, miterLimit,
dash, dashOffset, strokePosition, paintOrder, trimStart, trimEnd, trimOffset,
trimMode with all their values; `vectorAssetType/@shape=rounded-rect|polygon|
star|line`; vector radius, points, innerRadius and the stroke-style attributes
and values; `paint-reference` forms of shape/vector fill and stroke and
`projectType/@background`. `docs/feature-matrix.md` regenerated.

## Deviations and decisions (all documented in xml-reference/design)

- Rect/ellipse shapes without any B1-4 construct keep the 1.0 analytic
  renderer (rule 8). Any B1-4 attribute selects the path renderer and the XSD
  defaults (butt, miter 4), so a stroked rect then has sharp miter corners.
- Vector assets without additions keep the 1.0 round-capped raster; extended
  vectors are limited to 16384 px per side and static paints (an animated
  paint on a vector asset is a load error: it is rasterized once).
- `trimOffset` is in outline fractions (AE degrees / 360); trims affect the
  stroke only, not the fill.
- `line` is `(0,h/2)→(w,h/2)`; inside/outside strokes on a line are errors.
- Radial focal circles are not clamped (SVG 2 / canvas cone); pixels outside
  the cone are transparent.
- Area-accumulation limitation: overlapping stroke pieces within one partially
  covered boundary pixel (retraced paths) sum before the clamp; tested bound.
- Gradient paints evaluate on the project clock (no owning node, per B1-1).

## Tests

- `unit.shapes` (9): arc length/extraction, coverage clipping and mapped
  raster areas, cap/join/miter-limit areas against closed forms, closed rings,
  degenerate dots, hairpins, trim intervals (wrap, swap, negative offset,
  sequential), dashes (phase, offset, trim interaction, zero dashes, piece
  limit), seam joining invariance, dot winding, precision failures,
  constructors (rounded-rect, CSS radius scaling, ellipse, polygon, star,
  roundness, line), flattening counts, ledger accounting and short quotas.
- `unit.paint` (9): linear/units/rotation, two-point conical radial against
  an independent bisection solver, focal cone, aspect, identical circles,
  conic angles, spreads, colour hints, stop clamping, animated opacity,
  Oklab reference values (Ottosson), interpolation spaces, oklch shorter hue
  and powerless hue, translucent Oklab, tiny vectors, repeat boundary,
  dither determinism/period/zero mean, background fill at 1/3 threads.
- `unit.xml_shapes` (8): fixture fields, 22 rejection diagnostics (with
  attribute and line), version gates and the 1.0 new-attribute exception,
  96 seeded valid/mutated/truncated documents loaded twice and rendered,
  legacy identity (analytic vs path renderer bit-identical on aligned rects),
  exact area, 1/4 threads and warm shuffled compositors with scene memory
  unchanged, deformation and depth card, runtime dash limit, transform
  overflow (also with deformation), direct-C preparation rejections.
- `unit.oom`: load replay of `tests/data-shapes.xml` (120 allocations),
  `gradients.xml` (120), `shapes-strokes.xml` (81); one-frame render replay of
  the fixture at 1 and 3 threads (226 allocations each), all returning
  SR_ERR_MEMORY without leaks and reproducing the reference frame.
- Goldens `shapes-strokes` and `gradients` (320×180, 24 frames, frames
  0/12/23), 1 vs 4 threads with warm frames, and frame order (sequential,
  shuffled, sliced, interrupted/resumed) for all 23 goldens.
- Fixture `tests/data-shapes.xml` uses every new construct and every new
  animatable property.

Visual review of the references: rounded rect with TL/BR 2 and TR/BL 14
radii; pointy-top hexagon with round joins and slight roundness; star whose
inner radius/roundness animate between frames; rotated round-capped line;
evenodd star with an empty centre; three zigzags showing miter, miter-limit
bevel and round join/caps; dashed rect whose dashes move and join at the
start corner; round dots on an ellipse; inside stroke on a star, outside stroke
on a rounded rect; stroke-fill pentagon with half the stroke hidden; the
sequential draw-on path growing from a sliver (f0) to both contours (f23); a
rotating trimmed arc; a waving deformed star; a tilting depth-card triangle.
Gradients: padded 3-stop ramp with flat ends, rotated reflect, diagonal hard
repeat stripes, radial sun, focal point moving left→right, aspect-2 repeating
elliptical rings, rotating octagon conic, a moving/recolouring middle stop,
early-midpoint black→white hint, gradient-stroked star, four red→blue bars
(linear brightest middle, srgb darker, oklab/oklch more saturated), dithered
and undithered dark ramps, oklab vector chip with an inside stroke, and the
vertical backdrop gradient.

## Verification numbers (final tree `02359aa`, benchmark tool `fbd4220`)

- SDK Release CTest: 85/85 (includes frame_order, unit.golden 23/23,
  integration, unit.oom).
- SDK ASan/UBSan CTest (`ASAN_OPTIONS=detect_leaks=0`): 85/85.
- Coverage gate: 92.44% lines / 77.42% branches; floors raised from
  90.30/75.25 to 90.40/75.40. New modules: compositor_shape.c 91.1/72.2,
  paint.c 95.0/77.0, xml_shapes.c 91.8/77.8, vector_shape.c 97.9/80.7,
  vector_stroke.c 98.0/86.1.
- Byte oracle vs `/tmp/scene-render-b1-reference`: 309/309 previews across 42
  scenes, 3/3 encodes, 2 expected rejections (run with the untracked
  `assets/generated` copied into the worktree). Existing golden references
  and `tests/golden.sha256` unchanged.
- Read-only Codex review of `4106b84..1ea8f59`: 9 findings, all reproduced as
  failing tests and fixed in `fb555c1`. Follow-up review of `fb555c1`: 7 fixes
  confirmed, 2 incomplete and 1 earlier precision gap, fixed with tests in
  `02359aa`. No rejected findings.

## Performance

`scripts/perf-check.py` could not give a usable result: the machine ran other
lanes (load 8–16) and even the unchanged reference binary measured +27% to
+39% against the baseline. Three alternating pairs of the 4106b84 base build
and this branch on `benchmarks/perf-scene.xml` (TOTAL CPU s): base 22.16,
19.72, 16.47; branch 29.39, 16.98, 19.32 (medians 19.72 vs 19.32; composite
medians 3.36 vs 3.27). Ranges overlap completely; the 2% gate remains open
and `benchmarks/baseline.json` was not updated. By construction legacy scenes
gain one flag test per shape draw, one term in `sr_node_uses_compositing`, a
kind test per op band and one pointer test for the background.

Feature cost (`tools/shape-benchmark.py --runs 3`, 640×360, 24 frames, 4
threads, CPU s): path renderer vs analytic on 1200 aligned rects with
identical hashes: composite 0.092 → 0.125 (+36%); dashed round-joined strokes
vs legacy strokes: 0.173 → 0.387 (+123%); oklab dithered background vs
solid: clear 0.008 → 1.497 (about 62 ms per 640×360 frame, ~270 ns per pixel,
dominated by per-pixel transfer encode/decode). Noisy machine; treat as
orders of magnitude.

## Remaining / not done

- Strict 2% performance gate: not established (noisy machine), baseline
  untouched.
- The background gradient's per-pixel cost could be reduced (transfer LUTs)
  if it matters; not required by the contract.

## Merges with main (B1-6 `e059102`, B1-5 `d29c3d2`)

`a228f17` merges B1-6 outputs (and owner docs `e93a478`); `0ed16ae` merges
B1-5 timeline. Both are merge commits; conflicts were resolved keeping both
sides:

- `Makefile` (both source sets), `src/scene.c` (includes; B1-6 output frees
  then B1-4 paint frees), `include/scene_render/scene.h` (paints and the
  timeline pointer), `src/xml_internal.h` (still, marker, beat-grid, paint and
  stop element kinds), `src/xml_nodes.c` (B1-5 name/tags/startMarker/
  endMarker common attributes with the larger allowed array B1-4 needs),
  `tests/unit/test_golden.c` (all goldens), `docs/xml-reference.md` (both
  sections), `tools/coverage-gate.sh` (main's higher floors, 90.80 / 76.50).
- `schema/capabilities.json`: union of all profiles (B1-6's removed output
  occurrence limit and B1-5's `inherits` key kept); capability data and the
  feature matrix regenerated (`tools/feature-matrix.py --check` passes).
- B1-5 integration notes: gradient and stop hosts are now enumerated by the
  shared-host track resolver in `src/xml_timeline.c`, so `key/@marker` works
  on them (project clock), and paint ids are checked against beat-grid
  generated ids. New test `xml_shapes.key_markers_on_paint_hosts` (keys on a
  gradient and a stop snap to a marker; a painted shape with name/tags).
  B1-4 adds no node types; shapes get B1-5's common attributes through
  `sr_xml_start_node`.

Evidence on the merged tree `0ed16ae`:

- Release CTest: 94/94, including `frame_order` (all goldens, including
  `outputs`, `sequence-markers`, `shapes-strokes`, `gradients`), unit.golden
  and `integration` (B1-6 outputs at 1 vs 4 threads, `tests/golden.sha256`).
- ASan/UBSan CTest (`ASAN_OPTIONS=detect_leaks=0`): 94/94.
- Coverage gate: 92.89% lines / 78.50% branches, above floors 90.80 / 76.50.
- Byte oracle vs `/tmp/scene-render-b1-reference`: 309/309 previews across 42
  scenes, 3/3 encodes, 2 expected rejections.
- `tests/golden.sha256` and all golden references other than the two new
  B1-4 goldens are identical to main.
