# B1-3 resource-accounting audit

An independent read-only review mapped current allocation/work sites before
implementing the shared frame budget. The structural plan, named constants
and bounded mask-path parser do not yet enforce the complete render budget.
This audit is a follow-on implementation checklist, not a completion claim.

Use one ledger per outer render and share it with nested cards, captures,
effects and queues. Activate it from prepared compositing state, independently
of relative-length use. Preserve the legacy bypass without another scene walk.

## Integration sites

| Site | Live storage and work |
|---|---|
| `compositor.c`, `sr_pool_get` | Pool pointer capacity, descriptors and full RGBA allocations; clears/composites; nested plane compositors |
| Queue submission/flush | Queue/op/band-order capacities, copied mask chains; pixel work including masks/deformation/depth, plus band-by-op traversal |
| Deformation/particles | Parameter, mesh and soft-offset arrays; particle output capacity and preparation work independently of visible pixel area |
| Child/group/leaf drawing | Card-sort arrays, heap mask evaluations, mask-chain traversal and copying |
| Projective cards/scene entry | Simultaneous screen/plane surfaces, depth and clears, warp samples, card composites and DOF scratch; count shared depth once |
| `effects.c` | TLS scratch capacities, transfer tables, aligned padding, simultaneous low/high blur buffers and per-pass scratch |
| `lighting.c` | Pass state, transformed meshes, shadows, supersampled targets, depth and translucent fragments; private object captures |
| `compositing.c` | Prepared-plan/path retained capacity, aggregated during preparation so render entry needs no scene walk |
| `length_frame.c` | Evaluated geometry arrays and retained capacity |
| `vector_path.c` | Raster accumulator including guard columns and optional stroke indices |
| `renderer.c` | Explicit policy for composition/viewport float frames, consistent with direct-C caller-owned targets |

Packed output/writer/encoder/audio buffers and decoded media assets belong to
other subsystems. A generic process-wide allocator hook would accidentally
charge these unrelated owners and is not the implementation strategy.

## Required ownership and determinism decisions

- Persistent pool/queue/length capacities, TLS effect scratch and shared
  lighting scratch make a naive counter dependent on earlier frames. Ignoring
  retained allocations undercounts live storage; counting excess capacity
  without reclamation can make a warm frame fail when a fresh one passes.
  Choose deterministic reset or reclaim/trim unborrowed excess before rejection.
- Dirty rectangles also depend on prior frames. Charge a canonical clear-work
  bound; prior dirt must not change whether the current frame is admitted.
- Queue borrows outlive draw calls. Free mask copies only on flush/discard,
  and retain surfaces through the last borrowing operation. Particle operations
  share the first operation's owned mask copy: a new mid-emitter flush must
  invalidate/reset that shared pointer before it can be reused.
- Reserve before allocating or starting expensive loops, including old plus
  new buffers during growth. Roll back live reservations on OOM; resource
  failures remain distinct from OOM. Workers consume deterministic reservations
  made before dispatch, rather than racing to exhaust a shared counter.
- Band-order storage currently exists only for parallel dispatch, and lighting
  scratch ownership depends on lock acquisition. Thread count and trylock
  timing must not change resource admission. Use a canonical conservative
  reservation or a deterministic private scratch path for active B1-3 work.
- Specify renderer-owned but compositor-borrowed float target inclusion before
  coding, with the same accounting rule for direct-C targets. RGBA is four
  scalar-equivalent units per pixel; specify depth conversion too (an eight-byte
  depth value may consume two float-equivalent units), and charge full bytes.

Tests must exercise exact live-byte/pixel/work boundaries, old-plus-new growth,
partial cleanup, queue borrowing, repeated and shuffled frames, a larger prior
frame, one/four workers and runtime diagnostics. Limits apply to the already
implemented new color blends/skew as well as advanced masks/mattes/adjustments.
