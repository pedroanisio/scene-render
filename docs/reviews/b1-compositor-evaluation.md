# B1-3 node and card evaluation accounting

This increment connects remaining compositor-side track evaluation, card-list
sorting, recursive content bounds and projective preparation/sampling to the
shared frame ledger. Effects, lighting scratch and renderer-wide ownership
remain separate integrations. No additional schema capability is enabled here.

## Implemented policy

Scalar/keyed admission precedes opacity, transform/skew, mask, source-time,
shape-color, card-pivot/tilt and selected-camera evaluation. Repeated sampling
for sorting, content bounds and drawing is charged each time. Relative geometry
that was already evaluated is not sampled again. Finalized key counts/storage
are checked before access. Entry bounds camera/object counts and storage, and
accepts finite clocks with absolute value at most SR_MAX_DURATION. Visited
nodes, masks and cameras own failure diagnostics even when called from a
parent's sort or bounds walk.

Child/list/handoff scans reserve linear work. Active card runs use in-place
heapsort with the same far-depth/unique-document-order comparator, avoiding
hidden library sort allocation; legacy calls retain qsort. The documented
bound covers heap construction/extraction, comparisons and record copies and
is reserved before mutation. The list uses existing paired ledger ownership.

Projective preparation reserves 1024 units per card, covering six bounded
12-vertex clipping walks, guarded appends/intersections, final projections and
16 plane-size attempts. Finite checks protect clipping intermediates, projected
coordinates and plane extent/area. Every append checks the named stack capacity.
Screen coordinates are clamped to the declared target before conversion, and
plane dimensions/quantization are checked before integer conversion.

Warp work reserves 512 units per clipped screen pixel before dispatch: four
outer depth/plane queries, up to sixteen taps with homography, texel clear,
four RGBA gathers and accumulation, loop overhead and final channel stores.
The charge does not depend on selected samples or worker count. These logical
loop/copy units are not CPU instruction counts. The 2^30 work ceiling permits
at most 2097152 warped pixels before other frame work: a full HD warp leaves
12058624 units and may fail once its other consumers are included; a full 4K
warp always exceeds this conservative bound. Affine cards do not pay warp work.

## Review findings and reproductions

The design review found unchecked double-to-int screen bounds. A prepared
direct-C scene with a UINT32_MAX-wide camera space, a declared 64x64 target
and a finite tilted card reproduced a float-cast-overflow report at the old
compositor.c:1894 while returning success. Evidence:
`/tmp/b1-evaluation-cast-repro.c` and `/tmp/b1-evaluation-cast-repro.log`.
The bounded path now clamps finite coordinates before conversion. The same
instrumented probe reports no sanitizer error and still succeeds with an empty
offscreen target: `/tmp/b1-evaluation-focused-final.log`.

Diff review found finite plane extents whose product overflowed before the
area-based scale cap, turning the scale into zero and silently skipping the
card. A finite extreme camera/plane probe returned success instead of render
failure: `/tmp/b1-evaluation-area-repro.c` and
`/tmp/b1-evaluation-area-repro.log`. The checked path now rejects nonfinite
plane area before division/sqrt; a regression uses those finite inputs.

Two verification findings were reproduced before fixing. A keyed alpha channel
with an unkeyed red channel never entered the color evaluator; the probe kept
alpha 1 despite a requested .25. The corrected test establishes a keyed additive
red baseline and visible stroke, then independently adds the consumed alpha
track and checks the 63-unit increment and identical output. Evidence:
`/tmp/b1-evaluation-alpha-repro.log`.

The entry-admission test's camera had an invalid default fov. A diagnostic
probe showed the untouched baseline failing with camera @fov rather than
rendering successfully: `/tmp/b1-evaluation-camera-repro.log`. Every malformed
case now first asserts successful rendering with a valid camera, then checks
ledger failure status, cleanup and the correct composition/camera source.
Follow-up review closes both test findings and the plane-area finding; no
remaining review findings, conditional on final verification.

## Verification

Seven new cases cover independently calculated sort work and unchanged input
on rejection; ascending/descending/equal/infinite-key order; per-consumed-track
work differences; invalid track storage with heap-mask cleanup; independent
clock/count/camera failures; full card history and exact/short quotas; checked
projection/area arithmetic; and the documented resolution ceiling.

The render fixture has separated card runs, recursive known bounds, nine masks
per leaf, animated camera/tilt/color tracks, affine/projective paths and a
relative-coordinate variant with a 3D sphere to exercise object-key interleave.
A first-card-only render proves a non-affine pose and more than 16384 nonzero
warped pixels, crossing the actual parallel-dispatch threshold. Its one/four
thread outputs match, as do full legacy/bounded and shuffled-history renders.
Expanded OOM replay covers the card lists, pools, plane compositors, masks and
queued copies, with failure cleanup and reuse after success.

Initial SDK Release and ASan/UBSan each pass 82/82 CTests, including all golden
frame-order cases. Logs: `/tmp/b1-evaluation-release-oracle.log` and
`/tmp/b1-evaluation-asan.log`. Final review fixes require targeted reruns and
final compatibility verification. These are now complete: all six affected
Release suites and both affected sanitizer suites pass after rebuilding the
final review fixes. The final oracle against `8832f14` matches 309 previews
across 42 scenes, three encodes and two expected rejections. Logs:
`/tmp/b1-evaluation-release-final.log` and `/tmp/b1-evaluation-asan-final.log`.
The expanded card operation replays **25 allocations**; every injected failure
returns MEMORY, leaks nothing and leaves the compositor reusable.

Final coverage passes **82/82 tests**, at **92.25% lines / 77.25% branches**.
The line floor stays 90.30%; the branch floor rises to 75.25%, both checked
against the same counters. Logs: `/tmp/b1-evaluation-coverage.log` and
`/tmp/b1-evaluation-coverage-floors.log`. No sanitizer reports appear.

One fixed nine-run strict 2% baseline check on the final executable passes:
clear -1.3%, lighting -15.0%, compositor -12.2%, viewport -14.5%, effects -11.9%,
conversion -10.8%, total stage CPU 9.129 versus 10.437 seconds (-12.5%). These
noisy measurements do not establish a stable speedup. The particle checkpoint's
prior failures remain recorded in `b1-compositor-particles.md`; this is a new
cumulative passing result, not a deletion or reinterpretation of those misses.
Log: `/tmp/b1-evaluation-perf.log`. All preceding build/test/oracle jobs had
finished before timing; no concurrent engine timing runs were used.

The feature benchmark compares `8832f14` against this increment with five
alternating four-thread runs of 24 frames. Four animated projective cards with
nested groups/masks, separated runs and animated camera measure compositor
CPU 0.234253 to 0.240588 seconds (+2.70%, about 0.264 ms/frame), total +2.50%.
Compositor ranges are 0.222105–0.281885 / 0.231864–0.255016; total ranges are
0.226803–0.287194 / 0.236080–0.259369. The color-blend control measures
compositor 0.261124 to 0.257896 (-1.24%), total -1.22%; compositor ranges are
0.238887–0.302558 / 0.242900–0.299650, total ranges
0.245058–0.310215 / 0.248953–0.306904. Overlapping ranges do not establish a
stable change. All 24 before/after frame hashes match at one/four threads for
both fixtures. Driver `/tmp/b1-evaluation-benchmark.py`; log
`/tmp/b1-evaluation-feature-perf.log`.

The checked executable is preserved at
`/tmp/scene-render-b1-compositor-evaluation-verified`, SHA-256
`4565d99e3b26605ba247f5349d9d18b1cb28c0173a5671227976951d1c10d82d`.
The owner baseline remains unchanged, SHA-256
`215804d165a2bf161c79e044dab98a01f64564995c1259c1b43e0b30d2a853b4`.
No golden or integration hash changed. All verification jobs are terminal.

## Remaining work

Private effects/DOF and lighting/shadow/fragment scratch/work, renderer outer
composition/viewport/global-effect scope, path raster scratch and aggregate
new authored/path ownership remain unconnected. Advanced mask rendering/XML/
animation, seven remaining blend operators, dependency captures, mattes and
adjustments are still required, followed by B1-4 through B1-6, dependent loader/
animation hosts, batch merges and the complete requirements audit.
