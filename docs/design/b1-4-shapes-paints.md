# B1-4 shapes, strokes, trims and paints

Scope: the complete B1-4 item of `docs/schema-1.1-batch1-proposal.md` §4,
based on `4106b84`. This is the implementation contract. Capability entries
stay unsupported until rendering, tests and reference documentation are done.
New attributes on 1.0 elements keep the loader's 1.0 new-attribute exception;
new elements (`paints`, gradients, `stop`) and new enumeration values
(`shape="polygon"` and so on) require 1.1. Runtime dispatch uses resolved
flags, never the document version.

Deferred by the proposal and kept behind the unsupported gate:
`vector/@shape="svg"`, `meshGradient`, `pattern`. Also out of scope because
other items own them: `shapeModifier` (batch 5), path/polygon/star *masks*
(B1-3), `keyType/@value=paint-reference` (animating a colour track into a
paint, no evaluator exists) and shape `effects` (B1-3 compositing).

## Compatibility

- A `shape` with `shape="rect|ellipse"` and none of the B1-4 attributes,
  paint references or B1-4 animations keeps the 1.0 analytic renderer
  (signed-distance fill, stroke `|sd| - w/2` with rounded outer rect
  corners). Existing goldens and every 1.0 fixture therefore cannot move.
- Any B1-4 construct on a shape sets `SrShapeStyle.extended`. Extended shapes
  use the path renderer below, which follows the XSD defaults (butt caps,
  miter joins, miter limit 4, centred stroke, fill then stroke). This is the
  one intended visual difference: a rect that gains e.g. `paintOrder` gets
  sharp miter corners instead of the legacy rounded outer stroke corners.
- A `vector` asset without stroke-style attributes, new shapes or paint
  references keeps its 1.0 rasterization (distance-field rect/ellipse,
  exact-area path fill, round-capped distance stroke). With any of them it
  uses the new path renderer once at asset load, like the legacy asset.
- Extended shapes set `scene->compositing_required` (via
  `sr_node_uses_compositing`), so they always run on the bounded compositor
  path with the shared resource ledger. Legacy scenes gain one flag test per
  shape draw and no traversal.

## Modules

| Module | Owns |
|---|---|
| `vector_path.c` (extended) | arc-length measure/extract, coverage clipping, transformed polygon rasterization with the existing exact-area accumulator |
| `vector_stroke.c` (new) | trim, dash and the polygon stroker (caps, joins, miter limit) |
| `shape_geometry.c` (new) | rect/rounded-rect/ellipse/polygon/star/line/path contour constructors with tolerance-based flattening |
| `paint.c` + `scene_render/paint.h` (new) | gradient data, time evaluation, colour-space interpolation, dither, background fill |
| `compositor_shape.c` (new) | per-frame extended shape evaluation, coverage grids, per-pixel sampling |
| `xml_shapes.c` (new) | shape/stroke-style attributes, `paints`, gradients, `stop`, `url(#id)` resolution |

Shared files receive only hooks: one `SrShapeStyle` field in `SrNode`, one
`SrVectorStyle` block in `SrAsset`, `paints`/`background_paint` in `SrScene`,
new registry hosts/rows, dispatch-table entries, a new op kind in
`compositor.c` whose row kernel calls `compositor_shape.c`, the renderer
background hook and `procedural.c` dispatch. `compositor.c` is not otherwise
reorganized.

## Data structures

```c
typedef enum { SR_LINE_CAP_BUTT, SR_LINE_CAP_ROUND, SR_LINE_CAP_SQUARE } SrLineCap;
typedef enum { SR_LINE_JOIN_MITER, SR_LINE_JOIN_ROUND, SR_LINE_JOIN_BEVEL } SrLineJoin;
typedef enum { SR_STROKE_CENTER, SR_STROKE_INSIDE, SR_STROKE_OUTSIDE } SrStrokePosition;
typedef enum { SR_PAINT_FILL_STROKE, SR_PAINT_STROKE_FILL } SrPaintOrder;
typedef enum { SR_TRIM_SIMULTANEOUS, SR_TRIM_SEQUENTIAL } SrTrimMode;

typedef struct {           /* shared by shape nodes and vector assets */
    bool set;              /* any stroke-style attribute was authored */
    SrLineCap cap; SrLineJoin join; double miter_limit;
    double *dash; size_t dash_count;   /* owned; expanded to even count */
    SrAnimValue dash_offset;           /* static on vector assets */
    SrStrokePosition position; SrPaintOrder order;
} SrStrokeStyle;

typedef struct { char *id; struct SrPaint *paint; size_t source_line; } SrPaintRef;

typedef struct {
    bool extended;
    SrFillRule fill_rule;                  /* shape default: nonzero */
    SrAnimValue radius; double corner_radii[4]; bool corner_radii_set;
    uint32_t points;
    SrAnimValue inner_radius, outer_radius; bool inner_radius_set, outer_radius_set;
    SrAnimValue inner_roundness, outer_roundness;
    struct SrPreparedPath *path;           /* owned, immutable after load */
    SrStrokeStyle stroke;
    SrAnimValue trim_start, trim_end, trim_offset; SrTrimMode trim_mode;
    SrPaintRef fill_paint, stroke_paint;
} SrShapeStyle;
```

`SrShapeType` gains `ROUNDED_RECT`, `POLYGON`, `STAR`, `LINE` (appended; the
existing values keep their numbers). `SrPreparedPath` gets a struct tag so the
scene can own one through a pointer. `SrPaint` (in `paint.h`) holds the id,
type (linear, radial, conic), spread, units, interpolation space, dither flag,
the animatable geometry (`x1 y1 x2 y2`, `cx cy r fx fy fr aspect`, `angle`,
`rotation`), focal-set flags and an owned `SrGradientStop` array
(`offset`, `SrAnimColor color`, `opacity`, `midpoint`, source line). The scene
owns `paints[]` in document order; references are borrowed pointers resolved
at load and never written during rendering.

Registry: new hosts `SR_PROPERTY_LINEAR_GRADIENT`, `_RADIAL_GRADIENT`,
`_CONIC_GRADIENT`, `_GRADIENT_STOP`; rows (all `SR_PROPERTY_REQUIRE_1_1`)
for shape `radius`, `innerRadius`, `outerRadius`, `innerRoundness`,
`outerRoundness`, `trimStart`, `trimEnd`, `trimOffset`, `dashOffset`
(flag `SR_PROPERTY_SHAPE_STYLE`, which marks the shape extended); gradient
`x1 y1 x2 y2 cx cy r fx fy fr aspect angle rotation` on their types; stop
`offset`, `color`, `opacity`, `midpoint`. Key bounds come from the rows.
Paints and stops use the project clock (B1-1: project paints have no owning
node), so `timeBase` local/normalized refer to the project span.

## Geometry

Local coordinates are the node's box `[0,w] x [0,h]` (w/h after relative-length
evaluation). All contours are closed polylines except `line` and open path
subpaths.

- `rect` / `rounded-rect`: the box; `radius` or `cornerRadii="TL TR BR BL"`
  (overrides `radius`) give circular corners. Radii are clamped at zero and
  scaled by the CSS Backgrounds 3 §5.5 factor
  `f = min(1, w/(tl+tr), w/(bl+br), h/(tl+bl), h/(tr+br))`.
- `ellipse`: inscribed in the box, sampled exactly on the curve.
- `polygon`: centre `(w/2,h/2)`, outer radius `outerRadius` or `min(w,h)/2`,
  `points` in [3,4096]; vertex k at angle `-90 + 360k/n` degrees, i.e. the
  first vertex points up and vertices advance clockwise in y-down pixels.
- `star`: 2n vertices alternating outer radius R and inner radius r. Without
  an `innerRadius` attribute or track, r = R/2 at every time; an authored track
  overrides that relationship. Evaluated radii are clamped to `0 <= r <= R`.
- Roundness follows the lottie-web star/polygon construction (MIT,
  `ShapeProperty.js`): at a vertex of radius ρ the tangent handles are
  perpendicular to the radius with length `roundness * 2πρ / (4·points)` for
  both polygons and stars (lottie's `perimSegment`), and consecutive vertices
  are joined by cubic segments; the outer vertices use `outerRoundness`, the
  inner ones `innerRoundness`. Roundness is clamped to [0,1] after evaluation. `polygon`
  rejects authored `innerRadius`/`innerRoundness`; `rect`/`ellipse`/`line`/
  `path` reject polygon/star attributes and `path` is required exactly for
  `shape="path"`, like vector assets.
- `line`: `(0,h/2) -> (w,h/2)`; rotate the node for other directions. It has
  no fill area. `strokePosition` inside/outside is a load error on a line.
- `path`: the existing SVG subset `M/L/H/V/C/Q/Z` in local coordinates,
  parsed once at load by the bounded parser (`SR_MAX_MASK_PATH_*` limits and
  the 1e9 coordinate bound; the legacy 16/12-piece curve subdivision). The box
  is not used to rescale path coordinates.

Generated curves are flattened in raster space with tolerance
τ = 0.01 px. For an arc of radius ρ and angle θ,
`n = ceil(θ / (2 acos(1 - τ/(sρ))))`; for a cubic with control points P,
Wang's bound `n = ceil(sqrt(0.75 M s / τ))`,
`M = max|P_i - 2P_{i+1} + P_{i+2}|` (Sederberg, *CAGD notes* §10.6). s is the
largest singular value of the local-to-raster linear map (1 for asset and
deformation grids). Every count is clamped to [1,1024] per quarter turn or
per cubic, which bounds error, not correctness.

## Arc length, trim and dash (`vector_path.c`, `vector_stroke.c`)

The arc length of a flattened contour is the in-order double sum of its
segment lengths, including the closing segment of a closed contour.
`extract(contour, s0, s1)` returns the sub-polyline between arc lengths
s0 <= s1 with linearly interpolated end points and the tangent at s0 (kept
for zero-length pieces).

Trim: `a = clamp(trimStart,0,1)`, `b = clamp(trimEnd,0,1)`, swapped when a > b;
`o = trimOffset` in *fractions of the outline* (1 = one full turn; After
Effects' degrees divided by 360). No stroke when b - a <= 0, the untrimmed
outline when b - a >= 1. Otherwise the visible interval is
`[u, u + (b-a)]` with `u = (a+o) - floor(a+o)`; a part past 1 wraps to the
start. For `simultaneous` every contour applies these fractions to its own
length; for `sequential` the contours are concatenated in document order and
the fractions apply to the total. A wrapped interval on a single closed
contour is one continuous piece through the start vertex (a join, not two
caps); otherwise every visible part is an open piece with caps. Trim affects
the stroke only; the fill is the untrimmed shape (the draw-on use case).

Dash: non-negative finite lengths in local units, at most 64 values; an odd
list is repeated once (SVG 2 §13.5.5). An all-zero list is a solid stroke;
negative values or a zero-sum list with nonzero entries cannot occur after
validation. The pattern phase is the arc length along the *original* contour
plus `dashOffset` (modulo the period, floor based), so dashes stay fixed while
a trim animates. Dashes intersect each visible trim piece. A dash crossing a
closed contour's start is joined across it, and one dash covering a whole
closed contour is stroked closed. A zero-length dash is a dot with round or
square caps and nothing with butt caps; dots use the winding of every other
stroke polygon. A dash that cannot advance the double-precision position
fails the render.

## Stroker

Each piece becomes nonzero-filled polygons (the Skia/FreeType stroker
construction). Consecutive points closer than 1e-9 local units are merged.
With h = width/2 and left normal `n = (-d.y, d.x)` of the unit direction d:

- Open piece: left offsets forward with joins, the end cap, the left offsets
  of the reversed piece with joins, the start cap, as one closed polygon.
- Closed piece: the left offset loop and the reversed piece's left offset
  loop, each closed. They have opposite orientation, so nonzero leaves the
  inside of a closed stroke empty.
- Join at vertex V between offsets A = V + h·n_in and B = V + h·n_out: if the
  outgoing direction turns toward this side (`d_out · n_in > 0`) it is the
  inner side and the outline goes A, V, B through the pivot; the resulting
  loop has the same orientation as the outline, so nonzero union covers it.
  Otherwise: bevel A, B; round arc A→B around V; miter tip
  `V + h·m/(m·n_in)` with `m = normalize(n_in + n_out)` when
  `1/(m·n_in) <= miterLimit` (SVG miter ratio `1/sin(φ/2)`), else bevel.
  `miterLimit` must be >= 1.
- Caps: butt none; square extends both offsets by h along d; round is a
  half circle. A degenerate piece is a round dot or a square oriented by its
  tangent.

Coverage: fill contours with `fillRule`, stroke polygons with nonzero, both
through the existing exact-area accumulator after the affine map to raster
space. Where two stroke polygons overlap partly inside one boundary pixel the
accumulator sums their areas before clamping (the documented limitation of
this rasterizer); interior overlaps are exact.

`strokePosition`: inside/outside rasterize a stroke of width 2w and clip it by
the fill coverage cf of the untrimmed shape (open contours closed implicitly):
`cs_in = cs·cf`, `cs_out = cs·(1-cf)`. This coverage clipping lives in
`vector_path.c` and is exact where either coverage is 0 or 1.

`paintOrder` with premultiplied blend-space colours F (fill) and S (stroke):
fill-stroke `C = S·cs + F·cf·(1 - S_a·cs)`, stroke-fill
`C = F·cf + S·cs·(1 - F_a·cf)`. Then node opacity, masks and blend apply as
for every node.

## Rendering path

`sr_draw_shape` tests `extended` first. The extended path:

1. Charges every consumed track, evaluates geometry parameters at the render
   time, builds local contours, trims, dashes and strokes them.
2. Without deformation, maps all polygons by the node's world matrix into the
   target, computes their pixel bounds (+1 px), intersects the clip, and
   rasterizes fill and stroke coverage grids of exactly that size.
   With deformation it rasterizes in a local grid (one sample per local unit,
   one pixel halo, pixel centres at half integers) and each target pixel
   samples it bilinearly after `sr_deform_inverse`; bounds come from
   `sr_deformed_bounds` padded by the geometry's overhang of the box.
3. Submits one `SR_OP_COVERAGE` op immediately (queue flush, then row-parallel
   execution), so the coverage grids have a stack lifetime and are released
   before return. For each pixel the row kernel computes the local point
   through the op inverse, asks `compositor_shape.c` for the composed colour,
   multiplies opacity and masks, and blends with the node's mode.

The grids are built on the calling thread; workers only read them. Nothing is
cached across frames or written to the scene.

## Paints

`paints/linearGradient|radialGradient|conicGradient` with `stop` children.
A gradient needs at least one stop. The painted box is the shape box, the
vector asset box, or the frame for `project/@background`. `units="object"`
maps local point p to `(p.x/w, p.y/h)`; `units="user"` uses local pixels.
`rotation` (degrees, clockwise in y-down) rotates the gradient about the box
centre: the sample point is `c + R(-θ)(p - c)` in local pixels before the unit
mapping.

- Linear: `t = ((q - P1)·(P2 - P1)) / |P2 - P1|^2`; coincident points paint
  the last stop colour (SVG).
- Radial: the two-point conical definition of HTML canvas
  `createRadialGradient` (focal circle `(fx,fy,fr)` at t=0, end circle
  `(cx,cy,r)` at t=1; the largest t with r(t) >= 0; no solution is
  transparent). fx/fy default to the evaluated cx/cy. `aspect` scales the
  gradient's y axis by `aspect` before solving (ellipses with rx/ry = aspect).
  The focal point is not clamped (SVG 2 cone behaviour).
- Conic: `t = frac((θ - angle)/360)`, θ = atan2(dx, -dy) in degrees, i.e.
  0 at the top, increasing clockwise, in gradient (unit-mapped) space.

Spread: pad clamps t to [0,1], repeat `t - floor(t)`, reflect `1 - |m - 1|`
with `m = t mod 2`. Conic t is already in [0,1).

Stops: evaluated offsets are clamped to [0,1] and then to at least the
previous stop's offset (SVG/CSS). Colours are evaluated like every colour
track, multiplied by `opacity`. `midpoint` H of a stop applies to the segment
to the next stop: `w = P^(ln 0.5 / ln H)` for H in (0,1) (CSS Images 4 colour
hints), `w = [P > 0]` for H = 0, `w = [P >= 1]` for H = 1. Before the first or
after the last offset the end colour holds; at equal offsets the later stop
wins for t equal to that offset.

Interpolation of premultiplied colours (CSS Color 4 §12.3):

- `linear`: linear-light working RGB.
- `srgb`: the working space's transfer-encoded values (the authored numbers).
- `oklab`: Ottosson's Oklab from linear working RGB via the working gamut's
  RGB→XYZ matrix and Ottosson's XYZ→LMS matrix, L/a/b premultiplied.
- `oklch`: Oklab to polar form. Hue is powerless when chroma < 1e-6 and then
  takes the other endpoint's hue (both powerless: hue 0). Hue uses the CSS
  "shorter" rule: with Δ = h2 - h1, add 360 to h1 when Δ > 180 and to h2 when
  Δ < -180; Δ = ±180 exactly is not adjusted. L and C are premultiplied, hue
  is not. This is a complete, deterministic rule, so oklch is implemented.

Results convert back to linear working RGB, unpremultiply, clip to [0,1]
(no gamut mapping), then to the straight encoded colour for dithering and
finally to blend space with the same conversion as solid colours.

Dither (`dither="true"`, the default): an 8x8 Bayer threshold
`(B[y][x] + 0.5)/64 - 0.5` times 1/255 is added to each encoded RGB channel
before clamping; alpha is not dithered. The matrix is offset by
`(k & 7, (k >> 3) & 7)` with
`k = splitmix64(FNV-1a64(id, 0, "paint.dither") XOR projectSeed)`. Coordinates
are integer pixels of the receiving raster (frame, isolated group, card plane
or vector asset). The pattern does not change with frame, thread or order.

Hosts: `fill`/`stroke` of shapes and vector assets and `project/@background`
accept `url(#id)`. The id must name a gradient; unknown ids, other kinds and
colour animations of a url-painted `fill`/`stroke` are load errors. Vector
assets are rasterized once, so a paint with any animation used by a vector
asset is a load error. A paint background replaces the solid clear with a
row-parallel gradient fill of the composition.

## Limits

| Constant | Bound |
|---|---:|
| SR_MAX_PAINTS | 4,096 per scene |
| SR_MAX_GRADIENT_STOPS | 256 per gradient |
| SR_MAX_DASH_ENTRIES | 64 authored values |
| SR_MAX_SHAPE_POINTS | 4,096 (`points`) |
| SR_MAX_SHAPE_VERTICES | 4,194,304 per evaluated shape (fill + stroke polygons) |
| SR_MAX_DASH_PIECES | 1,048,576 per evaluated shape |
| SR_MAX_SHAPE_COORDINATE | 1e9 local or raster units (radii, dash, evaluated geometry) |
| SR_MAX_CURVE_PIECES | 1,024 per cubic or quarter arc |
| miterLimit | [1, 1e6] |
| trimOffset | abs <= 1e6 |
| coverage grids | SR_MAX_COVERAGE_DIMENSION per side, plus the shared ledger |

Load-time violations report element, attribute and line. Runtime violations
(animated values outside the coordinate bound, vertex/piece limits, nonfinite
raster coordinates) fail the render with the node's element, attribute and
line. Direct-C scenes get the same structural checks in
`sr_scene_prepare_compositing` (points, dash count, stop count and order of
storage, prepared path presence); invalid storage fails preparation.

## Resource accounting

Everything on the extended path uses `sr_composite_alloc/realloc/free` with
the frame ledger: geometry arrays (bytes only), the two coverage grids and the
accumulator (bytes plus one scalar pixel per float). Work is reserved before
the loops it covers (logical units, conservative):

- each consumed track: the existing 64/1 rule;
- geometry: 16 units per emitted local vertex (constructors, trim, dash,
  stroker) and 4 per transformed vertex;
- rasterization: per edge `8 + 4·(rows + 1) + (columns + 2)` of its raster
  extent, then one unit per grid cell for resolution and one for clipping;
- per-pixel sampling: `32 + 8·ceil(log2(stops + 1))` per bounds pixel for each
  gradient paint, 4 for a solid colour, charged before submission in addition
  to the ordinary op cost.

Prepared shape paths count toward the plan's aggregate prepared bytes. Vector
assets are rasterized in `procedural.c` at asset load with raw allocations,
bounded by the asset dimensions (at most 16,384 per side as today) and the
geometry limits above.

## Determinism

Evaluation is a pure function of the scene and time: no caches, no scene
writes, no randomness except the seeded dither pattern. All geometry is built
serially in a fixed order with double arithmetic; row workers only read the
immutable grids and paint evaluation. Frames are therefore identical at every
thread count and render order. No external files are introduced, so the XML
fingerprint covers every input; physics colliders keep their box/circle policy
(the rendered outline changes, not the solver shape).

## Performance

Legacy scenes: one boolean test per shape draw, one flag term in
`sr_node_uses_compositing`, one pointer test for the background. Budget: <2 %
per stage on `benchmarks/perf-scene.xml`. Feature cost: O(vertices), plus
O(edges x rows) rasterization and O(area x paint cost) per extended shape;
recorded on the shapes golden and a large-gradient benchmark scene.

## Test plan

- Unit (`test_shapes.c`, `test_paint.c`): arc length and extraction against
  closed forms (rectangle perimeter, circle 2πr within flattening error);
  trim intervals including wrap, swap, offset ±, sequential; dash phases and
  odd lists; stroker areas against closed forms (butt/square/round caps on a
  segment, miter/bevel/round corner areas, miter limit switch, closed square
  ring area `4·(outer² - inner²)`); coverage clipping identities; polygon and
  star vertex positions, roundness handle lengths vs lottie formula; corner
  radius scaling; linear/radial/focal/conic t against independent formulas;
  spread modes; stop clamping and midpoint exponent; oklab round trip and
  Ottosson reference values; oklch hue shorter/powerless cases; dither
  pattern determinism and bounds.
- XML (`test_xml_shapes.c`): every attribute and error path with line
  numbers, url resolution, wrong kinds, animated paint on vector assets,
  version gating, 1.0 exceptions; seeded valid/mutated/truncated fuzz of dash,
  cornerRadii and whole shape/paint documents.
- Render: legacy identity for rect/ellipse without B1-4 constructs; extended
  rect equals the analytic area; thread 1/4 and shuffled/warm frames;
  immutable scene memory before/after; card and deformation paths; exact and
  short ledger quotas; background paint.
- OOM: XML load and one-frame render of the fixture; asset rasterization.
- Goldens (<=320x180, <=24 frames): `shapes-strokes` (all shape kinds, caps,
  joins, miter limit, dashes, positions, paint order, animated trim) and
  `gradients` (linear, radial with focal/aspect, conic, spreads, units,
  rotation, animated stops, all four spaces, dither, background and vector
  asset paints). Frame-order CTest, oracle, coverage gate and perf-check as in
  the lane brief.
