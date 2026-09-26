# B1-3 shared render resources

This supplements `b1-3-compositing.md` and implements the open obligations in
`../reviews/b1-compositing-resources.md`. It does not relax the reviewed byte,
pixel or work bounds, and applies to all active B1-3 features, including color
blends and skew. Legacy renders retain their existing cache and allocation path.

## Scope and accounting

One render-owned ledger spans a complete frame evaluation. Nested groups,
cards, private captures, masks, prefixes, effects and lighting share it.
Direct compositor calls create their own outer scope. The renderer owns the
outer scope when composition, viewport and global-effect stages are combined.
No ledger is stored in, or mutates, the scene.

The ledger starts with prepared compositing ownership, aggregated during
preparation, and all declared input/output float targets that coexist during
evaluation. A renderer composition and its distinct viewport target both
count. A direct-C target counts its declared width x height x four floats,
regardless of who allocated it. A shared target/depth allocation counts once
within this scope; an alias is a borrow, not another allocation. The caller's
unobservable excess capacity is outside this declared-target contract.

Include full allocated capacities of compositor-owned descriptors, pointer
arrays, surfaces, depth, queues, mask copies, deformation/particle results,
evaluated geometry and scratch, including alignment/padding and allocator
bookkeeping introduced by this subsystem. Stack descriptors do not consume
the heap-byte budget. New B1-3 authored data/strings and prepared geometry count in aggregate
prepared ownership even when released by the scene rather than the plan;
existing asset/media storage belongs to asset ownership.
Packed output, encoder/audio buffers and writer copies remain in their own
subsystems. Do not intercept process-wide allocations.

RGBA counts as four scalar-equivalent pixels. A float coverage/shadow sample
counts as one; an eight-byte depth sample counts as two. Non-image metadata
consumes bytes without scalar pixels. Every allocation consumes its full byte
size independently of this scalar convention. Products and additions are
checked before reservations, allocations, loops or coordinate casts. Active
B1-3 float targets are preflighted against the reviewed dimension bound before
conversion to integer clips or any pixel access.

Byte/pixel reservations represent conservative live upper bounds. A reservation
may cover a known scratch bundle or a potential parallel-path buffer even when
the selected implementation uses less. These bounds must cover every actual
owned allocation, be documented at the consumer and be independent of worker
count, addresses, lock acquisition and earlier frames. Reservations are never
discounted because an allocator happens to resize in place.

## Cache and queue lifetime

At the start of an active B1-3 outer evaluation, reclaim unused persistent
compositor pools, queue capacities and evaluated-length storage. At the end,
discard or finish all borrows, then release every frame-owned allocation.
Preserve caller options such as thread count. Nested compositor contexts share
the outer ledger and do not reset it. A later legacy render can build its usual
persistent caches again. Neither stale capacities nor old dirty rectangles
may determine admission of a new frame.

Bounded effect and lighting work uses deterministic private scratch lifetimes
instead of inheriting unrelated TLS/shared-cache capacity or a trylock outcome.
All allocations used by that private path are covered by reservations made on
the calling thread. Existing global caches not borrowed by this path remain
outside its ownership. Their legacy users keep their existing behavior.

The existing scene-owned particle-rate cache retains its synchronized ownership;
rendering never resets it. Preparation accounts a conservative full cache-capacity
reservation from the authored emitter/track, regardless of cache readiness.
Frame work admission charges a conservative cold-cache evaluation bound for
that emitter at the requested time, including rate integration, key evaluation,
checkpoint copies, emission candidates and output collection. It never discounts
this charge for cached totals/checkpoints. Fresh, warmed-at-a-later-time and
shuffled evaluation must therefore receive the same budget decision. The cache
capacity remains part of aggregate prepared ownership throughout the frame.

Compositing invalidation/preparation also invalidates particle-rate caches
outside rendering before edited authored inputs can be evaluated. Invalidation
uses the existing prepared node inventory; successful initial preparation also
clears caches on newly admitted nodes before publishing their new bounds.
Consequently an invalidate/edit/reprepare cycle cannot reuse either old rate
integration or an old larger cache allocation. Existing asset/physics cache
preparation contracts remain separate. Add a warm/change-and-shrink/reprepare
comparison with a fresh equivalent emitter.

Queued operations borrow immutable surfaces until flush/discard. Copied mask
chains remain charged until their owning operation is released, including
copies shared by particles. Budget exhaustion fails the render; it does not
introduce a mid-emitter flush, evict a borrowed surface or silently skip work.
Normal existing ordering/overlap flushes remain valid. Charge a canonical
clear-work bound independent of prior dirty rectangles.

## Ledger and allocation contract

The private ledger tracks current/peak bytes and pixels, cumulative work, the
first failure and its owning source location. Its limits cannot exceed the
named compositing constants. Smaller private limits support remaining parent
reservations and inexpensive boundary tests; they are not a new user option.

Reserving bytes/pixels checks both dimensions atomically and leaves current
ownership unchanged on rejection. Work reservations are monotonic for the
frame and occur before expensive work. Release cannot underflow. Subtraction
of a completed ownership reservation never refunds work already attempted.
The first failure remains sticky while cleanup releases owned storage.

Allocating helpers may use an aligned private prefix to record the charged
capacity of new frame-owned blocks. The prefix itself counts toward bytes.
They preserve the ordinary zero-initialization/alignment contract, use the
project allocators, and always have a paired release using the same ledger.
Zero/copy allocation work is conservatively charged in four-byte units,
rounded up and including bookkeeping. Legacy calls bypass this prefix and use
the existing allocators. Cross-module
objects with existing release APIs must either carry the ledger through their
ownership contract or use an explicitly reserved raw-allocation bundle; never
pass a prefixed payload to an ordinary raw free.

Growth reserves the complete replacement alongside the old allocation before
calling the allocator. On OOM, release only the replacement reservation and
leave the old block valid. On success, retire the old reservation and publish
the new capacity. Count limits and overflow checks precede growth arithmetic.

Resource exhaustion returns SR_ERR_RENDER with the owning element/attribute
and source line. Real allocation failure returns SR_ERR_MEMORY. A helper's
NULL pointer must not cause a resource failure to be mislabeled as OOM.
Ownership scopes preserve the original failure location while unwinding.

## Work and parallelism

Consumers reserve checked upper bounds for their loops, including node/mask
and queue traversal, band-by-operation dispatch, geometry/particle preparation,
path edges and guard cells, raster samples, depth samples, effect passes,
clears, capture/replacement and morphology/blur work. The consumer documents
the relation between its bound and its nested loops; a pixel-area-only charge
does not cover a hidden per-mask/per-edge/per-fragment loop.

Workers consume pre-established deterministic reservations. They do not race
to exhaust a shared work or memory counter. Parallel-only order arrays use the
same conservative reservation at one and multiple workers. Scratch layouts or
bundle bounds must likewise cover all supported dispatch choices consistently.
Dynamic fragment growth needs a pre-established conservative capacity/work
bound before raster workers start; it cannot use timing-dependent admission.

### Evaluated lengths and deformation

Length frames carry the same borrowed ledger from their first allocation until
paired free. The descriptor, node/mask capacity and old-plus-new growth count;
growth zeroing and reset zeroing are separate from the allocator's copy charge.
Bounded compositor entry reclaims any earlier raw length arrays before attaching
a ledger. Physics preparation and other legacy callers retain NULL ownership.

Scalar animation reserves 64 units for a keyed track (at most 16 binary-search
steps plus 24 Bezier iterations and constant curve/clock work), one for a static
value. Relative-length tracks additionally reserve a second search, up to six
key copies/conversions and a temporary track copy. Key count/storage is checked
before evaluation. This consumer integration includes length-walk opacity and
deformation parameters/grid points; other animation consumers are still listed
as remaining work until connected.

Deformation state owns parameter, mesh-pointer, evaluated-grid and soft-offset
arrays through the ledger. Active compositing accepts at most 65536 modifiers
per node and grid sides in [2,16], matching the existing XML grid range. Check
those bounds before products or pointer access. Declared direct-C backing arrays
must contain rows*cols*2 points, sample_count*rows*cols*2 soft doubles or
sample_count rigid records; a bare pointer cannot expose its actual capacity.
Cached physics sampling also requires finite time and finite positive fixedStep
before floor-to-index conversion, and checked sample-offset products.

Each grid inverse reserves `16 + 8*(rows+1)*(cols+1)` units per clipped pixel,
plus one dispatch per modifier. This covers initial/final field checks, twelve
Newton iterations, and the analytic fallback's exterior/interior patches,
outer row dispatch, four hull corners and up to two roots/considerations per
patch. Reserve the full bound even for a zero grid or an early-converging pixel.
Mesh/soft extent scans and output preparation have separate reservations;
`modifier.amount` is charged again for its existing bounds evaluation.
Parameters and arithmetic remain unchanged on the legacy path.


### Particle consumers

The public particle evaluator retains raw ownership and legacy arithmetic. A
private compositor entry carries the ledger through collector growth, keyed
walk totals, segment marks and the fixed 1025-double regeneration buffer;
returned particles use paired ledger release after their draw operations have
copied their values. Existing queued mask borrows keep their normal lifetime.

Preparation adds every emitter's conservative rate-cache allocation capacity
to aggregate prepared bytes, independent of cache readiness. Compute it from
the authored finalized rate track's slot and checkpoint layout. The scene-owned
cache remains one raw allocation protected by its existing mutex. Invalidation
clears caches through the old prepared inventory before freeing the plan;
successful preparation clears newly admitted emitters after all bounds pass and
before publishing the plan. Structural XML preflight remains separate from
final preparation after track resolution. A failed preparation publishes nothing.

Bounded particle tracks must provide finalized backing storage with at most
65536 keys; maxParticles is at most the existing XML limit 10000000. Rate-key
and emitter clock coordinates used by grid-index correction stay within the
existing animation/project time limits. The private path rejects invalid
metadata before indexing or candidate work. Seed hashing scans at most 1 MiB
of emitter id bytes, with a named limit and separate scan/hash work; an explicit
particle seed avoids that scan. Legacy callers keep their contract.

For a keyed evaluation let C be the grid span from the first cache cell to
min(the newest requested birth cell, the last cache cell), clamped below at
zero. Validate C against the existing maximum rate-cell count before work.
Reserve four times (C + slot_count + 1) times 80 work units, plus cache-capacity
zeroing. This covers cold boundary integration, checkpoint integration/copy,
block regeneration, reverse cell visits and key/segment dispatch. Each cell's
scalar rate evaluation costs at most 64 units; the extra factor covers starting
samples and all three possible passes. Never discount for warmed checkpoints.

Lifetime upper-bound scans reserve 64 units per key plus 128 for endpoint
sampling before evaluation. Each candidate deterministically reserves scalar
birth-parameter and color-channel evaluation plus fixed particle math before
being offered, regardless of whether it survives. Candidate visits are
independent of cache warmth and charged before work; the bounded path fails if
the existing 20000000-candidate ceiling is exhausted, rather than silently
truncating. Every closed-form path (static, before-first, after-last) rejects
nonfinite or greater-than-9e15 emission indices on the bounded path. Rejection
diagnostics omit unbounded emitter ids. Collector copies/growth, final reversal
and per-particle draw
preparation are charged separately. Actual allocation failure remains MEMORY;
work/storage/candidate rejection is RENDER. Frame history and worker count must
produce identical resource decisions and output.

### Node and card evaluation

Charge every consumed scalar track before sampling: opacity, transform and
skew, non-relative mask coordinates/radius, media source time, shape fill and
stroke channels, card pivot/tilt and active-camera values. Repeated evaluations
for sort keys, bounds and drawing each count; evaluated length coordinates are
not sampled or charged again. A shared track helper validates finalized storage
and applies the existing static/keyed work rule to color channels too. Fixed
matrix/corner/pose work and mask clipping have separate conservative charges.
Failed admission unwinds masks, lists and pools through their existing owners.

Before scene evaluation, bound camera and 3D-object counts at 65536 each and
require their declared arrays; reject nonfinite render clocks or
`abs(time) > SR_MAX_DURATION`, preserving the existing negative-time envelope.
Camera selection charges both its admission scan and the unchanged
view-construction scan, and validates only the selected camera's consumed
tracks before lighting or card code can sample them. Lighting's own repeated
evaluation and private scratch remain a separate integration. Object sort-key
evaluation charges its consumed position tracks and camera scan/transform;
the direct-C finalized-array contract continues to apply.

Child discovery, list construction, drawing and final handoff scans reserve
linear work before traversal. Prepared node/mask bounds still govern those
arrays. Card-run sorting on the bounded path uses in-place heapsort with the
existing total ordering (far depth, then unique document order), eliminating
unobservable library sort scratch. Legacy calls retain qsort. Each heap level
uses at most two comparisons and one three-record swap; reserve
`2*N*(ceil(log2(N))+1)*(4+3*ceil(sizeof(item)/4))` before mutation. The two
passes have fewer than 2*N sifts including root extraction. No allocation or
worker-dependent admission is introduced. The combined list is bounded by
the sum of the node and object ceilings before size addition.

Recursive content bounds charge each visited node, child edge and four-corner
projection, including visits that return unknown bounds. Transform work is
charged at each actual call. Projective clipping, magnification and the fixed
16-attempt size search reserve 1024 units per card before those loops. A convex
quad clipped by six half-planes has at most 39 input-edge visits, 45 emitted
vertex records, 12 crossings and 10 final projected vertices. The active path
checks finite clipping state and a named 12-vertex stack capacity before every
append. The 1024-unit reserve covers even six full 12-edge walks with bounded
appends, rejected intersections and twelve final projections, without relying
on exact convexity after floating-point rounding. Projected screen coordinates must be finite and are clamped to the
target before integer conversion; plane-size and quantization conversions are
range-checked. Mathematical clipping is not a substitute for those checks.
Visited nodes/masks/cameras temporarily own their admission diagnostics even
when the caller is sorting or walking a parent's bounds. The warp
reserves 512 units per clipped screen pixel before dispatch: four depth/plane
queries, up to sixteen taps (each including its homography query, four-channel
clear, four RGBA texels and accumulation), sample-loop overhead and the final
four-channel store. These are logical loop/copy units, not CPU instruction
counts. Reserve the same bound
even when a pixel rejects early or takes one sample; workers do not update the
ledger. At 512 units per pixel, the 2^30 frame-work ceiling admits at most
2097152 warped pixels before any other work. A full 1920x1080 warp leaves
12058624 work units for its other consumers, so many such cards will fail; a
full 4K projective warp always exceeds this conservative bound. Affine cards
do not pay warp work. Effects and DOF remain separately listed until their private scratch
and pass work are connected.

Tests compare bounded sorting with the existing comparator across ascending,
descending, equal-depth and mixed runs, and check an independently calculated
work ceiling and rejection before mutation. Render fixtures cover animated
transform/mask/color/camera tracks, multiple separated card runs, affine and
projective cards, recursive bounds, relative coordinates, exact/short quotas,
larger-prior/shuffled histories and actual one/four-worker dispatch. Malformed
track storage and clock/count limits fail before indexing with source owners;
existing OOM replay continues to cover all changed cleanup paths.

## Integration and verification

Integrate the ledger into actual consumers in reviewable increments. An
increment does not claim the full budget is enforced while effects, lighting,
renderer targets or other audited sites remain unconnected. All such consumers
must be connected before B1-3 is complete or the limit contract is documented
as fully implemented for users.

Tests cover exact named arithmetic limits without giant allocations, actual
render failures under smaller remaining budgets, old-plus-new growth, OOM and
partial cleanup, nested/shared targets, queue borrowing, target preflight before
memory access, and source diagnostics. Compare fresh, repeated, shuffled and
larger-prior-frame histories at one/four workers, including keyed-rate particle
emitters whose shared cache was warmed at a later time. Preserve scene immutability,
legacy golden identity, the oracle and the performance budgets.
