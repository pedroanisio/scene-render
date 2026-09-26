# B1-3 compositor resource ledger, first consumers

This increment implements the shared ledger and its first actual consumers.
It does **not** complete render-resource accounting or enable another schema
construct. The policy is `../design/b1-compositing-resources.md`.

## Implemented

The calling-thread ledger tracks live/peak bytes and scalar-equivalent pixels,
cumulative work and the first failure with its source location. Reservations
are atomic, arithmetic is checked, work is never refunded, and lower private
quotas cannot exceed the named limits. Private aligned allocation prefixes
count toward storage. Growth reserves old plus replacement storage even when
the allocator can extend in place. Allocation failure preserves the old block
and returns `SR_ERR_MEMORY`; resource rejection returns `SR_ERR_RENDER`.

Prepared compositor entry counts the structural plan, declared RGBA target and
any borrowed depth. Target dimensions and external depth layout are validated
before access. It reclaims prior compositor caches, preserves thread options
and borrowed depth, and releases frame-owned allocations on every exit. Nested
plane compositors and calls share the outer ledger. Legacy renders retain
their existing persistent caches and raw-allocation ownership contract.

Connected storage includes pool pointer capacities, descriptors, RGBA buffers,
queue records/operation arrays/band-order arrays, copied mask chains, heap mask
evaluations, card-sort arrays, nested plane compositor descriptors and owned
depth. Queue borrows survive until normal flush/discard; exhaustion introduces
no extra flush. Band-order capacity is reserved at one and multiple workers.

Connected work covers node/mask visits, inherited mask-chain traversal/copy,
allocation zero/copy capacity, ordinary pixel operations and their inherited
masks/depth samples, canonical full-buffer clears, depth initialization and
band-by-operation dispatch. Workers receive reservations before dispatch.

## Explicit remaining integrations

Deformation parameters/grids/soft offsets and inverse-warp work, particle
output/preparation and its cache accounting/lifecycle, evaluated relative
lengths, card sorting/bounds/warp work, animation-key evaluation work, effects
including DOF and TLS scratch, lighting including shadow/fragment scratch,
path raster scratch, and the renderer's composition/viewport/global-effect
outer scope remain unfinished. Prepared ownership must also aggregate new
authored/path data and particle caches as those consumers are integrated.
Dependency graphs/captures and advanced masks/mattes/adjustments remain gated.
The complete byte/pixel/work contract must not be advertised as implemented.

## Review and regression evidence

Independent review found two P2 issues. Both were reproduced before fixing:

- Reset discarded an external borrowed depth buffer, exposing an occluded
  card. The regression failed both pointer preservation and pixel comparison.
  Entry now preserves, preflights and reserves it; exit restores it. Invalid
  sample counts, dimensions and NULL data fail before access. A one-byte
  initial shortfall proves the external storage counts.
- Mask-chain allocation charged zeroing but omitted the subsequent copying.
  An independently derived work difference was **3366 vs expected 3532**.
  The fix separately reserves traversal and copied payload work. It retains
  the ordinary NULL-ledger path.

Reproduction: `/tmp/b1-resources-review-repro.log`; initial fixed regressions:
`/tmp/b1-resources-review-fixed.log`. Follow-up review closes both findings and
reports no other issues in this increment, subject to the verification gates.

Seven resource cases cover exact arithmetic limits without giant allocations,
atomic rejection/sticky errors, aligned ownership and old-plus-new growth,
actual exact/one-unit-short render quotas, one/four-thread pixels and accounting,
reuse after a larger legacy frame, target preflight/source diagnostics,
borrowed depth, independent copy-work derivation and nested projected cards.
The projected-card fixture includes a masked particle emitter sharing a queued
mask copy; bounded one/four-thread output matches the legacy reference render.

OOM replay covers **2** allocator-growth allocations, **12** ordinary
pool/queue/mask allocations and **20** nested projective-card/depth/shared-mask
allocations. Every injected failure returns memory error, retains correct
ownership and leaks nothing; the same compositor renders successfully afterward.
Focused evidence: `/tmp/b1-resources-focused.log`.

Final SDK Release, ASan/UBSan and coverage each pass **79/79 CTests**, including
all 21 golden frame-order cases, integration and OOM. No sanitizer reports
appear. Coverage is **92.26% lines / 76.95% branches**; floors are raised to
**90.25% / 74.90%** and pass against the same final counters. The oracle against
the preserved `6529d56` executable matches **309 previews across 42 scenes,
three encodes and two expected rejections**.

Logs: `/tmp/b1-resources-release-oracle-final.log`,
`/tmp/b1-resources-asan-final.log`, `/tmp/b1-resources-coverage-final.log` and
`/tmp/b1-resources-coverage-floors.log`.

The strict **2% baseline performance gate passes**: clear +1.0%, lighting
-15.7%, compositor -8.2%, viewport -12.4%, effects -12.0% and conversion
-10.7%. Stage-total CPU is 9.161 seconds versus 10.437 (-12.2%). Measurements
are noisy and do not establish a stable speedup. The run started after every
build/test/oracle job was terminal and no other renderer process was present.
Log: `/tmp/b1-resources-perf.log`.

An alternating before/after run against `6529d56` measures the new resource
path on the existing color/skew fixtures, five runs each at four threads.
Every one of their 24 frames matches before/after at one/four threads. Color
compositor CPU is 0.202071 to 0.202140 seconds (+0.03%; overlapping ranges
0.190685–0.211590 / 0.193171–0.220499); stage total changes -0.34%. Skew
compositor CPU is 0.024209 to 0.035820 seconds (+47.96%; ranges
0.023243–0.026828 / 0.034209–0.036241); stage total changes +31.14%. The latter
is a measured cost of this increment: about **0.48 ms of additional engine CPU
per frame** on that small card/group fixture. It must not be described as
zero-cost or hidden by the legacy-scene gate. Log:
`/tmp/b1-resources-feature-perf.log`; reproducible driver:
`/tmp/b1-resource-benchmark.py`, using `tools/length-benchmark.py`'s validated
24-frame metrics/hash reader. Both timing jobs are terminal.

No golden, integration hash or baseline is changed. Baseline SHA-256 remains
`215804d165a2bf161c79e044dab98a01f64564995c1259c1b43e0b30d2a853b4`.
The verified executable is preserved at
`/tmp/scene-render-b1-compositor-ledger-verified`, SHA-256
`914894ec6161d701fa77444c73363f58d54b62217d236249dd2233489f86f6bc`.
