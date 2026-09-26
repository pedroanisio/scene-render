# B1-3 compositing

Scope: the complete B1-3 item in the batch-1 proposal, based on `6eb035b`.
This is the implementation contract. Capability entries remain unsupported
until their implementation, reference documentation and verification pass.
New attributes retain the loader's 1.0 version exception; new elements and
enum values require 1.1. Runtime dispatch uses resolved feature flags, never
the document version. Explicit defaults must preserve legacy output.

## Compatibility and modules

Keep the six existing blend enum values, their float arithmetic, tiny-alpha
fallbacks and hot inline paths unchanged. Legacy intersect-only masks retain
their analytic coverage and operation order. Zero skew uses the existing
matrix sequence. Scenes without these features allocate no matte plan or
coverage grids and do not acquire another per-frame tree traversal.

Separate prerequisites from feature commits:

1. Extract stateless randomness and seed derivation into `random.c`, retaining
   the particle wrappers, stream mapping and historical hash offset exactly.
   This pulls forward only the output-neutral prerequisite of B2-0; gradient
   noise and fBm remain B2 work. Verify against the preserved `6eb035b` binary.
2. Introduce immutable prepared vector paths without changing existing path
   flattening or coverage arithmetic. Existing public entry points wrap the
   prepared representation. Verify with the oracle before adding masks.
3. Extract only the shared compositor types and interfaces needed by new
   coverage, matte and adjustment modules. Do not reorganize unrelated loops.
   Verify this extraction separately before introducing visible behavior.

`raster.c` owns the new pure blend kernels. `random.c` owns all random bit
generation. `compositor_coverage.c` owns new scalar coverage preparation and
sampling; `compositor_matte.c` owns the reference plan and capture cache;
`compositor_adjustment.c` owns backdrop copies and replacement. Shared private
types and callbacks live in `compositor_internal.h`. Loader parsing and
reference resolution use a separate `xml_compositing.c` module. Prepared
paths belong to `vector_path.c`; no second SVG parser is introduced.

The scene owns authored strings, tracks, prepared paths and a resolved
compositing plan. Render contexts own evaluated parameters, surfaces, private
depth and per-frame caches. Queued operations borrow immutable surfaces whose
owners survive the last queue flush. Discarding a failed queue releases its
borrows before recycling surfaces. No render changes a scene field.

## Blend arithmetic

Reference: [W3C Compositing Level 1, sections 9 and 10](https://www.w3.org/TR/2024/CRD-compositing-1-20240321/).
For source alpha `a`, backdrop alpha `b`, straight colors `s,d` and
premultiplied colors `S,D`, new color blend modes use:

```
A = a + b * (1-a)
C = (1-b)*S + (1-a)*D + a*b*B(d,s)
```

Only the straight inputs to new `B` functions are clamped to [0,1]. The
uncovered premultiplied contributions retain HDR values. The existing modes
keep their individual historical clamp policies, including unclamped `add`.
New color modes avoid division at zero alpha and use the existing normal
fallback below `SR_BLEND_MIN_ALPHA`. Porter-Duff operators do not use that
fallback: transparent source is significant for stencil operations.

All color modes, including plus-lighter, leave the backdrop bit-identical
when source alpha is zero. For plus-lighter this is an explicit exception to
the sum-and-clamp formula below: it does not clamp an HDR backdrop where no
source contributes. Any positive source alpha uses the plus-lighter formula,
including values below the ordinary color-mode tiny-alpha fallback.

The table specifies the modes not already implemented. `sat(x)` clamps to
[0,1]. In division branches, test exact endpoint conditions before dividing.

| Mode | Blend function or operation |
|---|---|
| exclusion | `d+s-2*d*s` |
| subtract | `max(0,d-s)` |
| divide | `0` if `d==0`; otherwise `1` if `s==0`, else `min(1,d/s)` |
| darken / lighten | channelwise `min(d,s)` / `max(d,s)` |
| darker-color / lighter-color | choose the entire color with smaller/larger `Lum`; ties choose backdrop |
| color-dodge | `0` if `d==0`; otherwise `1` if `s==1`, else `min(1,d/(1-s))` |
| color-burn | `1` if `d==1`; otherwise `0` if `s==0`, else `1-min(1,(1-d)/s)` |
| linear-dodge | `min(1,d+s)` |
| linear-burn | `max(0,d+s-1)` |
| hard-light | multiply branch when `s<=.5`, screen branch otherwise, per W3C |
| soft-light | W3C piecewise polynomial/square-root formula, including the `d<=.25` branch |
| linear-light | `sat(d+2*s-1)` |
| vivid-light | burn with `2*s` when `s<=.5`, dodge with `2*s-1` otherwise |
| pin-light | `min(d,2*s)` when `s<=.5`, `max(d,2*s-1)` otherwise |
| hard-mix | `0` when vivid-light is below `.5`, otherwise `1` |
| hue / saturation / color / luminosity | W3C `SetLum`, `SetSat` and `ClipColor` algorithms |
| plus-lighter | `C=clamp(S+D,0,1)`, `A=min(a+b,1)` |
| behind | destination-over: `C=D+(1-b)*S`, `A=b+(1-b)*a` |
| stencil-alpha / stencil-luma | `C=D*k`, `A=b*k` |
| silhouette-alpha / silhouette-luma | `C=D*(1-k)`, `A=b*(1-k)` |
| alpha-add | `A=min(a+b,1)`; `C=(S+D)*A/(a+b)` when the sum is positive, zero otherwise |

`Lum=.30*r+.59*g+.11*b` and `Sat=max(rgb)-min(rgb)` are the W3C
nonseparable color helpers. Darker/lighter-color deliberately use this same
`Lum`, with deterministic backdrop ties. The non-W3C formulas in the table
are renderer definitions, not claims of bitwise compatibility with another
application. In particular `alpha-add` preserves the alpha-weighted average
color when summed alpha exceeds one; `plus-lighter` clips the sum instead.

For alpha stencil/silhouette, `k=a`. For their luma variants, `k=a*Y(s)`.
Luma mattes use the same `Y`: unpremultiply, clamp straight RGB, decode the
project working transfer function unless already linear, then dot with the
working gamut's Y row from `color.c`. These rows are sRGB/Rec.709
(.21267290,.71515220,.07217500), P3 (.22897456,.69173852,.07928691), and
Rec.2020 (.26270021,.67799807,.05930172). Clamp the result to [0,1]. This
physical luminance is distinct from the W3C `Lum` helper. Pass a precomputed
project color configuration to the new kernels, without global state.

### Parent operators and dissolve

Stencil, silhouette, behind and alpha-add operate once on the flattened
source node, including all particles or group children. The source capture
uses normal root blending; its descendants retain their own blends. Its own
effects, masks, matte and opacity produce the source RGBA before the parent
operator. An empty active stencil therefore clears its whole parent target.
An inactive, invisible or zero-opacity node is absent and performs no
operation. This preserves node lifecycle semantics, even for stencil.

The immediate non-root parent of one of these operators, or an adjustment,
is isolated automatically, including when the child is inactive. Isolation
is a load-time plan property, so activation cannot change the coordinate
system. The isolated buffer starts transparent and contains only that
parent's children. Its own opacity, mask, matte, effects and blend apply
when it is composited into its parent. The composition root already has a
backdrop: preserve its target representation and existing 3D/card ordering.
Do not enable unrelated `group/isolate`, `clip` or `collapse` attributes in
this item.

Apply whole-backdrop operators throughout the applicable parent clip, not
just the source dirty rectangle. Treat all pixels outside source content as
transparent. Parent/ancestor coverage limits the operation as an interpolation
between the old destination and the operated destination; source masks remain
part of source coverage. Thus a source mask's empty area clears a stencil's
parent, whereas a mask on that parent limits the isolated parent's output.
Dirty bounds of the destination include cleared regions. Do not let the old
transparent-source fast path discard stencil work.

`dissolve` converts source alpha to Bernoulli coverage: keep the straight
source color at alpha one if `U<a`, otherwise use transparent black. Apply
it once to a flattened node after its own effects, masks, matte and opacity,
then source-over the result. Ancestor coverage limits the resulting parent
operation separately. This prevents overlapping particles from receiving
independent random choices for the same emitter pixel.

`U` is a stateless random value from the project seed, the stable scoped node
id, the domain string `blend.dissolve`, and integer composition-canvas pixel
coordinates. It does not depend on frame number, traversal order, worker or
buffer origin. For an offscreen plane, map the pixel center through the full
plane-to-composition transform and floor in composition coordinates before
hashing; reject nonfinite/out-of-range mappings. After projection the final
node dissolve runs in the receiving raster with this mapping. The same source
captured as a matte and drawn visibly uses the same choices. Negative
coordinates use their defined signed-64-bit to unsigned conversion, with
separate x/y mixing; no width-dependent flattened index.

The fixed pattern follows the distinction between dissolve and animated
dancing dissolve in [Adobe's blend-mode documentation](https://helpx.adobe.com/after-effects/desktop/work-with-layers/work-with-layer-blending-modes/blending-modes-layer-styles.html).
Seed derivation uses canonical 64-bit FNV-1a over `id`, a zero separator,
then the property/domain string, XOR project seed. An explicit seed, where
the host schema provides one, overrides derivation. Existing particles alone
retain their historical offset `1469598103934665603`, id-only bytes and
`splitmix64(seed ^ splitmix64(index*8+stream))` mapping. New shared helpers
must not silently replace these compatibility rules.

## Skew

Add `SrAnimValue skew_x,skew_y` to `SrTransform`, and registry rows
`skew.x` / `skew.y` for implemented node hosts (including adjustment).
Static attributes are `skewX` / `skewY`; units are degrees. The matrix is:

```
T(x,y) * R(rotation) * Kx(tan(skewX)) * Ky(tan(skewY))
       * S(scaleX,scaleY) * T(-anchorX,-anchorY)
```

`Kx` adds k*y to x; `Ky` adds k*x to y. Either nonzero skew inserts only its
own multiplication. Both zero retain the exact old operations. Static values
and keys must be within +/-89 degrees (`SR_MAX_SKEW_DEGREES`). Evaluate and
validate interpolated/extrapolated values at render time too: overshooting
curves may otherwise cross a tangent pole. Nonfinite matrices or failed
required inverses produce a diagnostic; ordinary zero-scale legacy behavior
is preserved when no new skew is involved.

Cards use the same skewed plane transform before camera projection. Soft-body
rest poses use base skew in the same order as the compositor; bump the
physics cache version and test invalidation. Rigid-body collision geometry
retains its existing authored box/shape policy, just as existing scale and
visual deformation do; skew changes its rendered image, not its solver shape.
Physics position/rotation overrides still take priority. Do not add skew to
object3D or camera, whose XSD does not include these node transform attributes.
Particle centers follow the node matrix; the existing screen-space sprite
radius policy remains, as it does for other emitter transforms.

The static attributes use the proposal's new-attribute exception and are
accepted in both document versions. The new animation property names
`skew.x` / `skew.y` require 1.1. Invalid skewed card projections, sort keys
and transformed content bounds must fail, including geometry consumed for
sorting/bounds even when opacity prevents drawing. Preserve the mutable
root lighting-pass handoff when copying contexts for skew propagation.

## Masks

Extend `SrMask` with a mode enum, animated opacity/feather/expansion,
fill rule, point count, inner radius and an owned prepared path. Add registry
rows for `opacity`, `feather`, `expansion` and `innerRadius`; preserve current
length resolution for x/y/width/height. Path text and integer point count are
static in this item. All properties have source-aware semantic diagnostics.

Masks combine in authored order. Skip `none` completely. Initialize the
accumulator to one when the first participating mode is intersect, subtract
or darken, and zero when it is add, lighten or difference. No participating
masks means coverage one. For accumulated A and current B:

| Mode | Result |
|---|---|
| intersect | A*B |
| add | A+B-A*B |
| subtract | A*(1-B) |
| lighten | max(A,B) |
| darken | min(A,B) |
| difference | abs(A-B) |
| none | unchanged |

For each mask, compute antialiased shape coverage, expansion, feather, then
inversion, then multiply by opacity. Clamp intermediate scalar coverage to
[0,1]. Later add/difference masks may restore coverage after zero, so neither
early zero return nor intersection of every shape's bounds is valid. Only
the all-legacy-intersect path keeps the old shortcuts. Inversion applies
outside the finite stored grid too: retain an explicit exterior value.

Advanced masks use a node-local raster at one sample per local pixel, with
pixel centers at half-integers. Map receiving pixel centers through the node
inverse and sample bilinearly, with the defined exterior value. Thus feather
and expansion are local pixel distances and follow node scaling/skew/card
projection. Simple legacy masks continue their output-resolution analytic AA.
Crop preparation to the inverse-mapped receiving clip plus the required
filter halo; never crop shape coverage before computing that halo. A
projective mapping crossing the horizon falls back to the bounded full local
extent or fails its resource limit, never silently truncates a mask.

Expansion is grayscale morphology on this antialiased grid: positive values
take the maximum over an integer-radius Euclidean disk, negative values the
minimum. Interpolate between the floor and ceiling absolute radius results
for fractional expansion; radius zero is identity. Disk membership is
`dx*dx+dy*dy<=r*r`. Implement each disk row as a horizontal sliding extremum
and combine rows in a fixed order, bounded by the work budget below. This
defines an isotropic integer-grid footprint rather than an unspecified
vector offset. Read transparent exterior before inversion.

Feather uses three separable box passes approximating a Gaussian, with
sigma=feather/2, the same integer-radius interpolation convention as the
existing group blur. Zero is exact identity; halo is `3*ceil(feather/2)`.
The scalar implementation uses tracked allocations and fixed row/column
arithmetic. Its normalization includes transparent samples outside the
stored shape. No row worker shares mutable sums with another worker.

Geometry rules resolve the XSD's optional dimensions explicitly:

- Rect, ellipse and rounded-rect require positive width/height as before.
  Rounded-rect uses radius; other simple shapes retain legacy radius
  handling. New path/points/innerRadius attributes on these kinds are errors
  unless they are harmless explicit schema defaults.
- Path requires nonempty `path`. Its coordinates are local; x/y translate
  them. Optional width/height define a positive clipping box beginning at
  x/y, before expansion/feather; both must be provided together. They do not
  silently rescale path coordinates. `fillRule` selects nonzero/evenodd.
- Polygon/star use x/y as center, positive radius as outer radius, and
  points in [3,4096]. The first vertex points up; subsequent vertices advance
  clockwise in the renderer's downward-y coordinates. Star alternates outer
  and inner vertices; omitted innerRadius is half the outer radius, and an
  explicit innerRadius must be in [0,radius]. If neither its attribute nor
  a track is authored, innerRadius follows half the evaluated outer radius
  at every time. An authored innerRadius track overrides that automatic
  relationship; its static base is the explicit attribute or half the static
  outer radius when the attribute is omitted. Validate the evaluated pair
  after interpolation, including overshoot. Optional width/height use the
  same explicit clipping-box rule as path. Polygon rejects a nondefault
  innerRadius. Immutable scene-owned topology stores the point count and
  vertex-angle ordering. At each evaluated time, bounded render-owned
  vertices use the evaluated radii; these temporary contours share winding,
  AA and morphology with path masks. A changing inner/outer ratio must not
  reuse one uniformly scaled frozen star contour.

The existing SVG subset remains M/L/H/V/C/Q/Z, relative and absolute. Keep its
16-piece cubic and 12-piece quadratic flattening for compatibility. New mask
parsing adds bounded bytes/commands/contours/points and rejects nonfinite
coordinates including overflow after relative additions or flattening.
Parsed path storage is immutable and owned by the scene. Validation and
rendering use the same representation, avoiding parse-per-frame work.
Polygon/star contours are the separate render-owned parameterized geometry
described above and are freed with their frame coverage resources.

## Track mattes

Resolve `matte` through the typed scene index. Eligible sources are all
implemented render nodes: group, layer, shape, particleEmitter, adjustment
and object3D. An asset, material, effect, light, camera or internal composition
root is a wrong-kind reference. Deferred node types stay capability-gated;
B1-5 extends the same plan for sequence. Resolve ids before sorting children
and retain authored source lines in every edge.

`matteVisible` belongs to the consumer. A referenced source's normal draw is
suppressed unless at least one reference sets matteVisible=true. This is a
static OR over all references, independent of consumer activity. Its own
visible/lifetime/opacity rules still apply. Keep suppression separate from
authored visibility, length preparation, card-run membership, depth-sort keys
and matte capture; removing a hidden card from the sort run changes siblings.

A source capture is its own image before blending into its parent: own
subtree, effects, masks, matte and opacity apply, and its root blend is normal
(dissolve still defines its intrinsic sampled image). Preserve the source's
authored placement: ancestor transforms/card projection, relative-length
scopes, animation clocks and physics poses. Ancestor visibility, lifetime and
opacity apply. Ancestor masks, mattes, effects, blends and sibling content do
not become part of the source image. This makes the reference an image of
the selected node, not a replay of its ancestors' composites. The source may
therefore contribute pixels that an ancestor mask would hide in normal draw.

The canonical source cache is an isolated, transparent composition-sized
image. A restricted traversal of its authored ancestor path preserves the
coordinate mappings and opacity while excluding the ancestor processing just
listed. Ancestor card projections require that path, not an affine shortcut.
The consumer samples this image using its parent-to-composition mapping,
including the homography of a cropped projective card plane. This is how both
images share the consumer's parent space. Do not render a full-canvas source
directly into a differently sized card crop. Pixels outside the composition
have zero source coverage.

Alpha coverage is source alpha. Luma coverage is alpha times Y as defined
above. Inverted modes use one minus that final coverage, including outside
the image. Apply matte coverage after own effects and masks and before node
opacity/parent blending. Captures are reusable by all consumers at that time;
no cache survives into another time or compositor. Apply the capture root's
ancestor opacity multiplier exactly once after its ordinary subtree render;
do not multiply it again for each child. A source under an authored
inactive ancestor has empty coverage. Zero opacity likewise avoids geometry
evaluation, consistently with the existing length prepass.

An object3D source uses a separate full-composition lighting pass, drawing
only the selected object with private depth and no planar point-shadow blob.
It retains material lighting and received shadow maps from the scene. Matte
suppression removes its normal color, depth and blob draws, but does not
override authored castShadow: its shadow casting is still part of scene
lighting. Do not claim support for the separately gated object3D lifecycle
attributes. Nested lighting must use the existing local-scratch fallback;
never release shared lighting state while a live pass holds it. Source
captures cannot write the main frame's depth or visibility state.

### Dependency graph

Build a bounded directed evaluation graph, not just a matte-reference DFS.
Vertices distinguish node image capture, group child composite and adjustment
prefix. Edges represent child image requirements, matte requirements and
adjustment inputs. Coordinate/opacity ancestry uses prepared values rather
than ancestor image edges. A child referring to its containing group creates
a cycle through containment and is rejected. A group referring to a child
can be valid because capture excludes ancestor mattes; do not reject it solely
because ids share ancestry. Prefix edges include the preceding effective
draw items in stable z/order, with card-depth ordering handled by the bounded
per-frame plan. Conservatively include every potentially preceding member of
a depth-sorted card run, so animated depth cannot introduce a runtime cycle.

Use iterative colored DFS/topological validation with a bounded edge stack.
Report the closing matte/adjustment edge with element, attribute, line and id
chain. A render-time in-progress guard is a final defense for programmatic
scenes, not a substitute for loader validation. Cache each source/prefix once
per frame and reject excess planned work; diamond-shaped graphs must not
cause exponential captures or repeated lighting setup.

## Adjustment layers

Add `SR_NODE_ADJUSTMENT`, the registry host and matching XML dispatch.
Resolve its required effects as typed references; all effect ids must exist.
Its lifetime, transforms, opacity, masks and matte use the common node rules.
It has no child content. The parent isolation rule above bounds its scope.

At its draw position, flush queued writes and any preceding lighting resolve.
Copy the parent's completed backdrop D into a private work surface F. Apply
the referenced group effects to F in authored order using the adjustment's
local-to-parent transform. The input rectangle is the complete parent, not
the mask bounds: blur/glow must read pixels beyond the eventual mask. For
normal blend, write `D + w*(F-D)` in premultiplied RGBA, where w is adjustment
opacity times its own masks and matte. Preserve D until this replacement is
complete. Source-over would double translucent backdrop alpha and is wrong.

For an explicitly non-normal adjustment blend, first compute the selected
blend operation with D as backdrop and F as source, then interpolate from D
to that result with w. The blend's full-parent/transparent rules still apply.
This is an explicit renderer extension. There is no second application of
opacity or masks. The transform positions masks and effect coordinates;
it does not translate the already completed backdrop pixels.

The root backdrop includes project background and the 3D/card items actually
drawn before this position. A nested isolated group's backdrop starts clear.
Do not force an offscreen root representation that disables 3D interleaving.
An adjustment card uses the parent's finite raster rectangle as its local
plane domain. Pull D into that plane through the adjustment's full
plane-to-parent mapping, preserving this unfiltered sample P. Apply effects
to a copy F, and compute the weighted premultiplied delta `w*(F-P)` for normal
blend, or `w*(blend(P,F)-P)` for another blend. Project the delta back and add
it to the original D. Never project a replacement copy of P: even inverse
maps cannot undo resampling loss. A zero delta leaves the destination bits
unchanged, including an identity effect over a one-pixel checkerboard. Clamp
resulting alpha to [0,1], floor negative RGB at zero and zero RGB at zero
alpha, retaining positive HDR values. Outside the projected domain the delta
is zero. Camera depth limits where replacement applies; the adjustment does
not create a new depth occluder. Halo samples outside the plane domain read
transparent pixels.

When an adjustment is itself a matte source, rebuild its authored parent
prefix in a private target. Its captured image is the effected prefix F
multiplied by its own mask/matte/opacity, before replacement into D. Ancestor
placement/opacity and activity follow ordinary source capture. Prefix
dependencies participate in cycle checks and caching. The prefix replay uses
the same ordering/background/3D policy as the normal parent path and cannot
read or mutate the in-progress destination frame.

## Limits and failure handling

Compositing limits apply when these features are used, independently of
`has_relative_lengths`. Preserve legacy acceptance for scenes that bypass
the new subsystem. Use overflow-checked arithmetic before allocating or
converting coordinates to integers. Limits are named in a shared private
header and enforced for programmatic scenes as well as XML where relevant.

Milestone sequencing: color kernels and skew use constant-size local math
and the existing renderer's allocations/traversals. Their milestones do not
claim the shared graph/surface limits below are implemented. The bounded
compositing preparation phase must install those checks for all new B1-3
features before B1-3 is complete, including direct-C scenes; it remains an
explicit requirement alongside advanced-mask and matte preparation.

| Constant | Bound |
|---|---:|
| SR_MAX_COMPOSITE_NODES | 65,536 including root |
| SR_MAX_COMPOSITE_MASKS | 262,144 aggregate |
| SR_MAX_COMPOSITE_DEPTH | 256 ancestry or evaluation stack entries |
| SR_MAX_COMPOSITE_EDGES | 1,048,576 |
| SR_MAX_COMPOSITE_CAPTURES | 65,536 per frame, including prefixes |
| SR_MAX_COVERAGE_DIMENSION | 16,384 including halo |
| SR_MAX_COMPOSITE_PIXELS | 536,870,912 live scalar-equivalent pixels |
| SR_MAX_COMPOSITE_BYTES | 2,147,483,648 live owned bytes |
| SR_MAX_COMPOSITE_WORK | 1,073,741,824 scalar pixel/edge steps per frame |
| SR_MAX_MASK_PATH_BYTES | 1,048,576 |
| SR_MAX_MASK_PATH_COMMANDS | 65,536 |
| SR_MAX_MASK_PATH_CONTOURS | 4,096 |
| SR_MAX_MASK_PATH_POINTS | 262,144 after flattening |
| SR_MAX_MASK_VERTICES | 4,096 outer points (star has twice as many vertices) |
| SR_MAX_MASK_FILTER_RADIUS | 4,096 local pixels for feather or abs(expansion) |
| SR_MAX_MASK_COORDINATE | 1e9 absolute local units, including derived vertices |
| SR_MAX_SKEW_DEGREES | 89 absolute degrees |

RGBA counts as four scalar-equivalent pixels; include matte cache, prefix
copies, temporary morphology/blur grids and new isolation surfaces in live
accounting. The byte budget also includes descriptors and prepared geometry.
Estimate work with checked products before starting expensive raster loops:
coverage pixels times disk diameter, blur passes, transformed path edge scan
extent, captures and replacement pixels all consume the shared frame budget.
An implementation may use a tighter proven bound, never omit work because a
loop is in a helper. Bound color/camera coordinate conversion independently
before floor/casts; existing project/frame dimensions remain in force.

Load-time excesses report the owning element/attribute and source line.
Runtime extent/work/animated-range violations fail the render with the same
source information. OOM is SR_ERR_MEMORY and cleans up every partially
constructed graph, path, buffer and borrowed queue entry. A resource limit is
not OOM and must not silently remove a mask or matte. No new external files
are introduced; XML remains in the source fingerprint and physics cache
versioning covers the changed prepared pose.

## Verification and milestones

After the reviewed neutral prerequisites, implement separately: color blends;
skew; prepared advanced masks; parent operators/dissolve; matte graph/capture;
adjustments. Each leaves unfinished capabilities gated. Add XML enablement
only with complete semantics, docs and rendering fixtures for that group.
Record any contract amendment and obtain focused review before depending on it.

Tests include independent small-buffer references for every blend, all
endpoint branches, alpha/HDR/ties and nonseparable gamut clipping; fixed
random/seed vectors; explicit-default equality; static/animated skew and
cache invalidation; masks in every order with soft and inverted coverage;
analytic shape and path fixtures; fractional morphology and blur borders;
animated polygon/star radii with explicit, automatic and tracked innerRadius,
shuffled-time evaluation and unchanged scene topology;
card and relative-length mappings; nested parent operations with empty,
transparent and many-particle sources; zero-opacity/inactive distinctions;
translucent adjustment replacements and effect reach outside mask bounds.
Pin identity adjustment cards over a one-pixel checkerboard at fractional
translation/scale and perspective tilt, including disabled effect stacks.

Matte tests cover each source/consumer kind, wrong-kind and forward refs,
direct/indirect/containment/prefix cycles, valid group-to-child references,
visibility OR across active/inactive consumers, source lifetime, source
ancestor policy, hidden-card sort stability, private depth, 3D shadows,
projective cards, diamond reuse, and source reuse from two transformed,
differently sized parents with relative geometry and local clocks. Render
frames shuffled through warm compositors and at one/four threads; compare
the source scene memory/fingerprints before and after.

Add seeded valid/mutated/truncated XML/path inputs; count/byte/work boundary
tests; allocation-failure replay for each new operation; and cleanup tests
after partial queue submission, capture failure and lighting failure.
Parser rejection diagnostics must name the attribute and source line.

The three new goldens are `blend-modes`, `mattes-masks` and `adjustment`, each
at most 320x180 and 24 frames. Inspect their reference images; never refresh
old goldens or integration hashes. Run the complete SDK Release, ASan/UBSan
and coverage suites, one/four-thread goldens, frame-order, OOM and integration
checks. Floors start at 89.85% lines / 73.75% branches and never decrease.
Run the legacy oracle at every structural commit and the final feature gate.

Performance budget: every measured legacy stage stays within the strict 2%
budget against the unchanged owner baseline. Run timing only after other
jobs stop. Add feature measurements for a blend sheet, many shared mattes,
large feather/expansion, and repeated adjustments; validate complete hashes
and successful summaries before accepting timing. Report feature CPU and
peak surface/work counts, including cache reuse, rather than claiming a
universal constant cost. Document the expected surface copies and additional
passes. Independent read-only review audits determinism, frame order, thread
safety, limits, OOM, fingerprints, 1.0 identity and the verification evidence.
