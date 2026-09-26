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
