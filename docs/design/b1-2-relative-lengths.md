# B1-2 relative lengths

Base: `d106925`. This completes the remaining B1-2 component; tokens and
metadata are already implemented. This design covers every relative-length
host required by the batch proposal, including their existing animation paths,
depth cards, viewport rendering and physics preparation. It does not enable
the deferred layout or responsive-media constructs.

## Contract and coordinate boxes

The XSD's `lengthType` and `positiveLengthType` accept unitless pixels and
`%`, `vw`, `vh`, `vmin`, `vmax`. An explicit `px` suffix is not in the contract.
Relative syntax follows the XSD decimal grammar; unitless parsing retains the
existing finite-double grammar. Positive attributes must be greater than zero.
New lengths preserve their unit and coefficient instead of replacing the
authored value at load time.

The frame reference is the **output frame**, `project.width/height`, as stated
in the XSD's introductory conventions. In viewport mode it is not the larger
scene360 composition raster. Temporary group/card buffers do not change it.
`vw`, `vh`, `vmin`, `vmax` use one hundredth of the corresponding output width,
height, minimum or maximum. `%` uses one hundredth of the relevant parent-box
axis. Compute `coefficient * reference / 100.0` in double precision.

The root box is the output frame. A group's explicit width and height resolve
against its containing group's box, independently per axis. Each missing axis
uses that output-frame axis, including an unsized group nested inside a sized
group. Group dimensions establish percentage scope; they do not introduce
clipping, scaling, layout or child-bounds measurement.

Node `x`, `y`, `anchorX` and `anchorY` use the containing group box. Shape
width/height use that same box. A mask is a child of its host node: its x/width
use the host's local width and y/height use the host's local height. That box
is the resolved shape size, media asset dimensions, or group dimensions with
the per-axis frame fallback. An emitter has no local box, so its masks use the
frame box. Neither node scale nor rotation changes a local percentage reference.

Examples to pin the distinction: in a 320x180 output, a group of 200x100 makes
its child's `x="50%"` equal 100. An unsized group inside it makes a grandchild's
`x="50%"` equal 160. A 40x20 shape's `mask width="50%"` is 20. `anchorX="50%"`
on that shape still uses the containing group's width, not the shape's width.

Supported attributes are group/layer/shape/emitter transform x/y/anchors,
mask x/y/width/height, shape width/height and group width/height. Mask radius,
stroke width, rotation, scale, 3D coordinates and particle-size parameters
retain their existing numeric types. Margins, layout spacing and layer
boxWidth/boxHeight remain gated with their deferred parent features.

The existing animatable node position/anchor and mask x/y/width/height
properties accept mixed-unit keys. Shape and group dimensions remain static
attributes in this slice: they are evaluated against the current boxes each
frame, but this proposal item does not add size-animation properties to the
existing registry. Other numeric properties must reject unit-bearing keys.

## Data and parsing

Add a zero-valued pixel unit and the five relative units in a small lengths
API. Add a unit to scalar bases and numeric keys; zero-initialized legacy
values remain pixels. Tracks record whether any key has a relative unit.
Keep existing shape width/height numeric storage and add its unit tags; groups
have separate optional width/height length values and presence flags. No
authored value is changed during resolution or rendering.

The property registry marks horizontal and vertical length consumers. The
shared key parser consults those flags instead of adding a property-name
lookup chain. The same typed parser handles static attributes and keys, with
positivity required only where the current static attribute requires it.
Existing mask animation overshoot is still clamped to zero after interpolation.

Only completed hosts gain `relative-length` capability forms. Extend the
generic key-value form inventory/checker so unrelated animation properties
cannot bypass version/capability diagnostics. Under proposal section 3.2,
relative forms on existing attributes require 1.1. New group width/height
attributes remain permitted on 1.0 elements, including relative spellings,
as required by the new-attribute exception. Existing token/paint form policy
is unchanged. Unsupported layout remains gated.

New named limits: 128 bytes for a relative-length spelling, absolute relative
coefficient at most 1e6, and absolute resolved relative values at most 1e12
pixels. Existing unitless paths retain their prior bounds. Reject non-finite
conversion/interpolation and out-of-range results with the host's source line,
element and attribute. A positive relative dimension that underflows to zero
also fails. Parser mutation and boundary tests cover every grammar branch.

## Animation without track copies

Conversion happens before interpolation, including additive bases, endpoint
extrapolation, offset-cycle deltas and the neighboring slopes of Catmull-Rom
and TCB. Spring clocks, temporal handles and normalized/local time mapping
retain the existing B1-1 semantics. Interpolating the raw coefficients and
then applying one unit would be incorrect for mixed units.

An internal helper selects at most six original key indices: global first
and last, the selected interpolation pair, and one neighbor on each side.
Sort/deduplicate this fixed set. Convert those key values and the additive
base into a stack-local track, retain all timing/curve parameters, and call
the existing evaluator with the original scene time. This preserves global
extrapolation endpoints and the actual neighbors needed for Hermite tangents.
It costs O(log keys) time and constant stack memory, without copying all keys,
allocating per frame or mutating a track. Zero/one/two-key tracks have explicit
boundary coverage. Relative-key tracks use the existing extended-animation
time/value/clock bounds, even with legacy linear curves. Finalization recomputes
the derived relative-key flag from current keys.

First introduce that helper in an isolated, output-neutral refactor. Share
the exact existing cyclic-time mapping between evaluation and neighborhood
selection, including decimal-cycle snapping and very large cycle counts.
Keep the ordinary in-range and hold/linear evaluator arithmetic intact.
The subset is evaluated with the original time and clock, not an already
mapped time, so clock mapping is applied exactly once per evaluation.

The neutral test compares complete tracks with their selected subsets for
every curve family, extrapolation mode, additive option and track clock,
including first/last boundaries, uneven key spacing, negative times and large
cycles. It also checks that all source keys remain unchanged. The byte oracle
and all existing goldens gate this refactor before unit-bearing code follows.

## One evaluated geometry table per frame

A new private lengths module builds a transient node-geometry table owned by
the compositor, indexed by the node's stable document order. Each entry holds
pixel x/y/anchors, resolved shape dimensions, the local box, and a slice of
evaluated mask x/y/width/height values. A bounded traversal propagates boxes
from parent to child before drawing. Values are evaluated once for the requested
time, then every draw/bounds/sort consumer reads the same table.

The loader records whether relative values occur. Without them, compositor
and physics retain their existing paths, with no geometry-table allocation or
extra scene traversal. New numeric group sizes alone require no render work
until a descendant or mask consumes a relative length. Render-owned buffers
grow on first need and are reused across frames. Reuse always recomputes values
for the current scene, time and output size; it is not a mutable scene cache.

Bound new storage/traversal with `SR_MAX_LENGTH_NODES=65536`,
`SR_MAX_LENGTH_MASKS=262144` and `SR_MAX_LENGTH_DEPTH=256`. Validate order indices,
counts and allocation sizes before use. Report the node that exceeds a bound.
Partial allocation/evaluation errors fail the render and leave buffers safe
to free or rebuild. The table is fully prepared before any drawing starts.

Normal traversal, isolated groups, leaf masks, card depth sorting, projective
card bounds, card content, deformation bounds and dimensions all use these
values. An inner card context borrows the outer table; its smaller raster
target never becomes a length reference. Queued worker operations retain
their existing numeric copies and must not hold pointers into mutable table
storage. Separate compositors have separate tables and can evaluate one
immutable scene independently.

Visibility, lifetime and opacity have distinct consumers. `sr_draw_children`
evaluates every direct card sibling's pivot before any of those checks.
Resolve x/y/anchors for these sort keys even for an inactive card under a drawn
parent. Preserve card runs and lighting flush grouping: an inactive card must
not be removed or assigned a substitute sort key.

`sr_content_bounds` skips invisible/out-of-lifetime subtrees, but traverses
zero-opacity descendants of a projective card, including their children.
Those transforms and dimensions still require evaluated geometry even though
drawing skips them. Masks are not used by this bounds traversal. Do not apply
an opacity-based subtree skip to geometry required for bounds. Box propagation
is independent of transforms and opacity. Frame-order tests reuse a compositor
across shuffled times and output sizes
and verify the authored units, keys and dimensions remain unchanged.

## Physics preparation and fingerprints

Preserve the engine's current base-pose policy: rigid bodies start from base
positions, collision dimensions use base shape/scale geometry, and soft-body
rest grids use base transforms. Animation does not become an alternative
integrator. Relative base values resolve against the preparation-time output
and parent boxes using the same box rules as drawing, with track evaluation
disabled. They are stored as pixel values in render/preparation-owned state,
never written into authored scene fields.

Rigid `BodyState` keeps the resolved initial dimensions. Soft state keeps the
resolved initial position/anchors/local dimensions. Existing multiplication
and integration order stays unchanged for unitless scenes. Drawing still
uses the precomputed physics pose where it already overrides node position;
length tracks do not override that pose. Soft grids retain their initial rest
geometry if a later evaluation box changes, matching their prepare-time role.

Implicit constraint rest lengths must also use resolved base positions.
The loader currently derives them from authored x/y coefficients; for scenes
using relative values, defer that derivation to preparation-owned constraint
storage. A pin derives its distance only when restLength is absent; spring
and distance constraints derive it when absent or explicitly zero, preserving
the existing distinction. Both elastic force and rigid-pin projection use
the prepared length. Keep the unitless loader path unchanged. Bound the new
array with SR_MAX_LENGTH_CONSTRAINTS=65536 and cover its allocation failure.
Do not overwrite the authored constraint when deriving the prepared value.

Fingerprint the new unit tags, relevant base values, ancestor group sizes and
output-frame dimensions. Keep the exact XML in the resume fingerprint. Add a
physics cache revision for the new geometry policy, and verify stale-cache
rejection plus cached/uncached and shuffled-frame identity. Include constraint
auto/explicit state and prepared rest lengths in the new policy signature. Include a viewport
case so preparation and rendering cannot accidentally choose different frame
references. No new external file or environment input is introduced.

## Validation and cost

- Typed parsing: all five units, signs, decimal boundaries, forbidden px and
  exponent-suffixed forms, positivity, counts, byte/value bounds, malformed and
  truncated inputs, fixed-seed mutations, source diagnostics and version gates.
- Independent numeric references: mixed-unit linear/Bezier/spring/Catmull/TCB
  keys, additive values, all extrapolations, normalized/local clocks, six-key
  neighborhood boundaries, and unitless versus converted-track equivalence.
- Box fixtures: nested sized/unsized/partially sized groups, masks on each host,
  all transform axes, anchors, shape sizes, viewport output versus panorama,
  and offscreen depth cards with effects/deformation. Include hidden,
  out-of-lifetime and zero-opacity cards among visible cards and 3D objects,
  comparing against literal geometry without changing sorting/flush groups.
  A tilted projective card with visible geometry and a large zero-opacity
  child/group verifies that invisible drawing still contributes to bounds.
  Group size alone must not clip or scale its contents.
- Physics: relative base positions/dimensions/anchors, rigid and soft bodies,
  implicit and explicit pin/spring/distance rest lengths, changed frame/group
  size or units invalidating caches, cached/uncached output,
  and unitless old-scene identity.
- A new small relative-length golden and literal reference comparisons at
  multiple frame sizes; shuffled/warm frame evaluation, 1/4-thread identity,
  all table-growth OOM paths and immutable scene snapshots.
- Full SDK Release/ASan/coverage, frame-order, old-scene oracle, and strict
  performance checks. Do not refresh old hashes or waive the open performance
  gate. Raise coverage floors after the verified milestone.

Cost for new scenes is a bounded O(nodes + masks) geometry pass plus O(log keys)
per animated length, with constant-size key conversion and reused buffers.
There is no new per-pixel work. The unchanged-scene median stage CPU budget
remains below 2%; measure rather than assuming the fast-path branch is free.

Implementation order: reviewed design; neutral timeline neighborhood helper;
typed values/parser and mixed-unit evaluation; box preparation and every draw
consumer; physics integration and fingerprints; full fixtures/documentation;
capability enablement with complete verification. Unfinished constructs retain
their unsupported diagnostics between commits.

## Pre-code review

The implementation agent checked the proposal, XSD, registry, compositor and
physics consumers; an independent read-only Codex reviewer completed the
second pass on 2026-09-26. The review identified the loader's implicit
constraint-rest derivation and the version-policy split between existing and
new attributes. Both are incorporated above. The follow-up found no remaining
design findings; the neutral timeline refactor may proceed with its planned
exact-evaluation tests and old-scene oracle.

An SDK xmllint probe confirmed that relative spellings reject surrounding
whitespace, plus signs and exponent notation, accept `.5` and `1.` decimal
forms, and allow zero only on the signed length type. Unitless exponent
numbers remain valid. No production changes are part of this design commit.

A subsequent pre-implementation consumer audit and read-only follow-up also
identified the inactive-card sorting and zero-opacity projective-bounds
exceptions above. Their evaluation requirements and literal-reference tests
are part of the geometry integration, not changes to the current draw path.
