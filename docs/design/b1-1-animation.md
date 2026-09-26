# B1-1 animation core

Scope: batch 1 proposal B1-1. The capability gate stays closed until each
construct has its parser, evaluation, tests and reference documentation.

## Data and migration

First introduce an immutable property registry in `property.c` with public
declarations in `property.h`. Each row identifies host kind, XML property
name, static attribute name, value type, byte offset, numeric bounds and
optional eligibility/activation flags. Types include number, point, colour,
paint, path and string; only types with a complete evaluator are registered.
An entry returns borrowed storage owned by its host. The registry allocates
nothing. Node subtypes have explicit host kinds and shared rows use host
bitmasks. Camera zoom remains eligible only when its static alias was set;
particle colorEnd keeps its activation flag. No lookup mutates a const host.

The first commit replaces both scene.c lookup chains and the XML host chains
without adding properties or changing accepted ranges. It preserves existing
lookup side effects at the caller boundary. XML key bounds come from the same
row. An oracle run against the pre-change binary gates that commit.

Feature commits extend `SrKeyframe` with bounded curve parameters and temporal
handles. `SrTrack` gains extrapolation, additive and clock configuration.
Zero initialization retains all legacy defaults. Colour channel tracks share
configuration, with interpolation continuing in the existing linear space.
Existing curves retain their exact arithmetic and operation order.

## Curve semantics

Use independent analytic functions for the Penner sine, polynomial, expo,
circular, back, elastic and bounce families. Back uses overshoot 1.70158;
elastic uses the conventional 0.3 period (0.45 for in-out). Exact endpoints
return exact key values, except step-start's deliberate initial jump.
`hold` aliases step. `steps` uses N equally spaced
jumps, with `start` jumping at the beginning and `end` at the end of each
interval. At the first or an interior key, the outgoing segment wins and a
step-start curve is right-continuous. The final key holds its exact value.
Other existing curves preserve their current endpoint comparisons.

Catmull-Rom and TCB evaluate a cubic Hermite segment using adjacent key values
and time-aware incoming/outgoing slopes. Missing neighbours use the segment's
secant slope. Zero T/C/B equals Catmull-Rom. T/C/B are per key; the segment
uses its left outgoing and right incoming tangent. Uneven key intervals get
dedicated reference tests.

Spring solves `m*x'' + c*x' + k*(x-1) = 0` with x(0)=x'(0)=0,
evaluated over the segment's elapsed seconds. Use separate underdamped,
critical and overdamped closed forms, including cancellation-safe roots for
strong damping. Return the target at the right key; there is no iterative
integrator, state, warm-up or dependence on previously rendered frames.

The XSD defines easeIn/easeOut as normalized influence/speed pairs. For the
outgoing pair (i,s), the Bezier handle is (i,i*s); for the next incoming
pair it is (1-i,1-i*s). Both values lie in [0,1]. Explicit `bezier` and
temporal handles on the same segment are a semantic error. Missing handles
use the existing default Bezier handle on that side. Spatial handles and
roving remain unsupported. Handle conversion happens at finalization.

References: [Penner's equations](https://robertpenner.com/easing/),
[CSS step timing](https://www.w3.org/TR/css-easing-1/), and
[Eberly's time-aware TCB derivation](https://www.geometrictools.com/Documentation/KBSplines.pdf).
The implementation is written for this repository; no dependency is vendored.

## Track evaluation and clocks

Hold preserves the endpoint. Linear extrapolation continues the boundary
secant. Loop wraps into the first/last key interval; ping-pong alternates its
direction. Offset wraps and adds the endpoint value difference per cycle.
Negative times use floor-based cycles, never truncation toward zero. A
one-key track holds its value for all extrapolation modes. Additive adds the
static base after interpolation/extrapolation, including colour's linear
channel bases.

The original closed key interval takes priority over extrapolation. Outside
it, loop/offset map exact cycle boundaries to the first key (and apply the
offset cycle count); ping-pong maps alternating boundaries to the first/last
key. A mapped first key uses step-start's outgoing jump; a mapped last key
holds its value. Tests cover either side of every boundary and negative cycles.

Composition time is project seconds. Local time is elapsed host time, including
ancestor group clocks; normalized time divides local time by the host span.
Hosts without a node interval use the composition span. Store a resolved
affine clock per track so evaluation remains a pure function of scene time.
The resolver assigns clocks after marker and sequence resolution when B1-5
lands. An omitted node end uses the project end transformed into the host's
parent clock; it never uses the in-memory infinity sentinel as a duration.
An explicitly normalized zero-length host is a load error. Global host times
are never transformed both in the draw context and again in a track.

For node start s and group parameters offset o, scale q, the child clock is
`childSeconds = (parentSeconds - s) * q + o`; the default q=1,o=0.
Compose these affine transforms from ancestors during resolution. A node's
local track sees its own elapsed seconds. Its masks and modifiers inherit
that clock and span. Audio-track local time starts at the track's start;
shared materials, shared effects, cameras, lights, force fields and project
paints use the project span because they have no unique owning node.
Normalized key positions convert through that span, but spring elapsed time
uses local seconds: `(normalizedTime - leftKeyTime) * localSpanSeconds`.

Already supported hosts retain their properties. New animate contexts on
supported hosts acquire registry entries and evaluators in feature commits
(including material and audio-track properties). Gradient/stop and new-node
hosts land with their B1-4/B1-5 owners. Entirely deferred hosts remain gated;
their animation children cannot make the parent implemented. Expression,
link and motionPath stay unsupported.

Material `baseColor`/`emissive` use `SrAnimColor`; `metallic`/`roughness`
use `SrAnimValue`. Each object evaluates its shared material once into its
render-owned frame data. Metallic and roughness clamp to [0,1] when animated;
static arithmetic remains unchanged. Audio volume/pan use `SrAnimValue` and
the mixer borrows their immutable tracks. It evaluates absolute sample time,
then existing equal-power pan and fades, with no cursor or block dependency.
Volume clamps to [0,1] and pan to [-1,1]. Audio normalized time spans its start
through the project end; clip length and playback settings affect source
sampling, independently of this automation clock. Host arrays are limited to
4096 materials and 4096 audio tracks; existing key limits apply to both.

## Limits, ownership and determinism

Limits: `SR_MAX_TRACK_KEYS=65536`, `SR_MAX_SCENE_KEYS=1048576`,
`SR_MAX_CURVE_STEPS=1000000`; adjacent finalized key times differ by at least
1e-12 (the existing uniqueness threshold). Extended tracks require finite
key/base magnitude <=1e12, key times within +/-1e6, and Bezier handle magnitude
<=1e6; existing legacy-only tracks retain their existing accepted range.
Spring mass and stiffness lie in [1e-6,1e6], damping in [0,1e6]. T/C/B lie in
[-1,1], normalized influences/speeds in [0,1]. Resolved clock coefficients
must be finite with magnitude <=1e12; normalized span must be >=1e-12.
These bounds keep interpolation/extrapolation finite throughout the bounded
project interval. Validate derived constants and clock endpoint values at
finalization, reporting a load error if they are nonfinite. Test the bounds
and nearly critical/strongly overdamped cases with floating-point sanitizers.

Preserve established rendering clamps (for example opacity in [0,1] and
nonnegative radii). New bounded properties define explicit evaluation clamps
in their registry rows; overshooting position values remain free to overshoot.
Key-value validation retains each property's load-time limits. No generic
clamp changes arithmetic on legacy animation paths. Check growth before
multiplication and allocation. Report limits/semantic failures at the source
key and attribute. Direct C callers must supply finite evaluation time;
checked preparation rejects a malformed in-memory track before rendering.
Every new allocation is scene-owned and released by the existing track-free
path, including partial colour-track construction. No render-time allocation
or mutable cursor is introduced for curve evaluation. No external files are
added, so scene XML remains the complete new resume input.

## Existing consumers and caches

Audit every direct key/track access, not only `sr_anim_eval`. The legacy
particle-rate cache assumes constant values outside the key span; its bounds
and endpoint rates are currently read directly from keys. Retain that exact
path for legacy tracks. Extended tracks integrate the existing fixed time
grid over the bounded active emitter interval using evaluated rates and the
existing synchronized checkpoint mechanism. Clock, additive and extrapolation
settings must affect every rate sample; raw key times cannot delimit a
looping or transformed track. The named rate-cell budget still applies and
oversized work fails explicitly.

Particle lifetime culling needs a conservative finite upper bound, including
Bezier, Penner overshoot, Hermite extrema, spring response, additive base and
extrapolation over the active interval. Preserve the legacy bound arithmetic;
use analytic extrema or documented conservative envelopes on the new path.
If a finite bound cannot be proved, use the finite elapsed emitter interval
as the lookback, subject to the existing particle-work limit. Tests must show
that early-born particles remain visible under overshooting lifetimes.

`physics.c:hash_anim` hashes track content independently of the source XML.
Extend that signature with every curve parameter, temporal handle, track
mode and resolved clock, and advance the physics cache format/signature
version. Tests change one parameter at a time and compare cached/uncached
simulation. Resume still hashes the complete XML; neither cache may reuse
analysis from a different effective animation.

## Verification and cost

Registry tests cover each host, aliases, wrong host/type, eligibility and
bounds. Curve tests compare known reference values for every enumeration,
endpoints, discontinuities, all damping cases, uneven TCB intervals, handle
conversion, negative extrapolation and one-key tracks. Include rate
loop/offset/additive/local-time comparisons, lifetime overshoot retention,
and physics cache invalidation for new animation fields. Parser mutations and
named-limit cases run under ASan/UBSan; OOM replays every growing track.

Add a <=320x180, <=24-frame curves golden, thread invariance and shuffled,
sliced and interrupted/resumed frame-order checks. Full Release, sanitizer,
coverage, existing goldens and integration hashes must pass. Run the oracle
for the registry refactor and again at the item boundary. CPU-stage regression
budget on the unchanged perf scene is <2%; new curves remain O(log keys) per
evaluation with a constant-cost analytic function and no integration steps.
