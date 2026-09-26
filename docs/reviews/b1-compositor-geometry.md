# B1-3 evaluated geometry resource accounting

This increment connects evaluated relative lengths, modifier parameters/mesh
points and sampled soft-body offsets to the shared compositor ledger. It does
not complete resource accounting or enable another schema construct. The
consumer-specific policy is in `../design/b1-compositing-resources.md`.

## Implemented

Evaluated geometry descriptors and growing node/mask arrays retain the same
ledger from first allocation through paired release. Growth charges old plus
replacement capacity, copied capacity and newly zeroed records; repeated
preparation also charges the explicit clearing pass. Node/mask traversal and
consumed length/opacity animation work are reserved before evaluation. The
existing scalar arithmetic is unchanged.

Deformation admission checks named modifier/grid limits and declared backing
arrays before parameter/grid loops. Static and keyed scalar work includes the
second amount evaluation in the existing bounds traversal. Cached rigid/soft
sampling validates finite clocks, backing pointers and checked declared extents
before conversion/indexing. Direct-C callers remain responsible for providing
arrays of their declared capacity; a bare pointer cannot prove allocation size.

Mesh and soft inverse work reserves the complete Newton/analytic fallback bound
for every clipped pixel before workers run, even for a zero grid or early
convergence. Parameters, mesh pointers/storage and sampled soft offsets are
charged and released around the existing immediate draw. Workers never mutate
the ledger. Legacy NULL-ledger paths preserve existing allocation ownership and
arithmetic.

## Review and regression evidence

Independent review found no production-code issues. It identified one test
coverage gap: the original 64×64 target could never exceed the 16,384-pixel
immediate parallel threshold. A new area assertion failed first, recorded in
`/tmp/b1-geometry-review-repro.log`. The history fixture alone now uses 320×256;
its half-width/half-height mesh+wave+soft draw exceeds the threshold. Legacy and
bounded one/four-thread pixels agree, and warmed/shuffled exact byte/pixel/work
quotas remain identical. Follow-up review closes the finding, subject to the
verification gates. The small allocation-failure fixture stays 64×64.

Five focused cases cover independently derived scalar/grid work, metadata limits,
relative array growth/reset, legacy-to-bounded ownership, thread/frame history,
exact and one-unit-short quotas, malformed sampled-physics metadata and cleanup.
A separate zero-mesh comparison independently derives the complete fallback
charge and proves a one-unit work shortfall rejects before any pixels are
written. Geometry OOM replay covers all **52 allocations**; every injected
failure returns `SR_ERR_MEMORY`, leaks nothing and leaves the compositor reusable.
Focused evidence: `/tmp/b1-geometry-focused-final.log` and
`/tmp/b1-geometry-review-fixed.log`.


SDK Release and ASan/UBSan each pass **80/80 CTests**, including all 21 golden
frame-order cases, integration and OOM. After the test-only parallel fixture
expansion, the changed suite passes again in both configurations. The final
coverage run passes **80/80**, with **92.27% lines / 77.15% branches**. Floors
are **90.25% / 75.10%** (branch floor raised), and a check against the same
counters passes. No sanitizer reports appear. The oracle against the preserved
`f18428d` executable matches **309 previews across 42 scenes, three encodes and
two expected rejections**. Production code is unchanged since that run.

Logs: `/tmp/b1-geometry-release-oracle.log`, `/tmp/b1-geometry-asan.log`,
`/tmp/b1-geometry-release-final.log`, `/tmp/b1-geometry-asan-final.log`,
`/tmp/b1-geometry-coverage-final.log` and `/tmp/b1-geometry-coverage-floors.log`.


The strict **2% baseline performance gate passes**: clear -10.9%, lighting
-20.1%, compositor -16.7%, viewport -20.7%, effects -18.7% and conversion
-17.0%. Stage-total CPU is 8.460 seconds versus 10.437 (-18.9%). These noisy
measurements do not establish a stable speedup. All build/test/oracle jobs were
terminal, and no competing renderer was present before timing. Log:
`/tmp/b1-geometry-perf.log`.

An alternating before/after run against `f18428d` measures the feature's own
cost, five runs per executable at four threads. A generated 320×180 scene has
40 relative shapes, each with a 3×3 mesh and wave, under an isolated skewed
group. Its compositor CPU is 0.131401 to 0.133684 seconds (+1.74%; overlapping
ranges 0.126331–0.141042 / 0.129604–0.139725), about **0.095 ms per frame**
additional engine CPU; stage total changes -0.21%. The existing skew fixture
measures 0.035390 to 0.036126 seconds (+2.08%; overlapping ranges
0.034667–0.037733 / 0.033953–0.037145); stage total changes +1.86%. These
small, overlapping measurements should not be treated as precise stable costs.
All 24 frames of both fixtures match before/after at one/four threads. Driver:
`/tmp/b1-geometry-benchmark.py`, using the validated metrics/hash reader in
`tools/length-benchmark.py`; log: `/tmp/b1-geometry-feature-perf.log`.

No golden, integration hash or baseline changed. Baseline SHA-256 remains
`215804d165a2bf161c79e044dab98a01f64564995c1259c1b43e0b30d2a853b4`.
The verified executable is preserved at
`/tmp/scene-render-b1-compositor-geometry-verified`, SHA-256
`385494932197310704402dfcb02bd0004cd73dc851d1f99bfd6f75becb872fc5`. All verification/timing jobs are terminal.

## Remaining integrations

Particle output/preparation, conservative cache capacity/cold-work accounting and
cache invalidation; other animation work, card sorting/bounds/warps; effects/DOF
and private scratch; lighting/shadow/fragment scratch; path raster scratch;
renderer composition/viewport/global-effect scope; aggregate new authored/path
ownership; and advanced masks/dependency captures remain incomplete. The full
byte/pixel/work contract must not be advertised as implemented. Other B1 items
remain open as recorded in the batch status.
