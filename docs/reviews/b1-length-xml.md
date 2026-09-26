# B1-2 relative-length XML enablement

The XML feature completes the reviewed relative-length geometry and physics
contract in `docs/design/b1-2-relative-lengths.md`. Capability forms now cover
node x/y/anchors, shape and group dimensions, masks and length animation keys.
New group dimensions retain the new-attribute version exception. Masks on
particle emitters use the existing shared mask path and require version 1.1.
Deferred layout and layer box attributes remain gated.

The loader checks relative-scene node/order, aggregate mask, depth and
constraint limits before reference resolution. Geometry remains evaluated at
render/preparation time. Legacy scenes bypass the new traversal.

## Review and regressions

The independent read-only review found two issues, reproduced before fixes:

1. An oversized composition reported its new node-count error without a source
   line because the root node had no recorded composition line. A parsed XML
   regression reproduces the missing line using the synthetic count boundary;
   `/tmp/b1-length-composition-line-repro.log` records the failure. Composition
   dispatch now records the line. The regression restores the count before
   cleanup and verifies line 3 on the diagnostic.
2. The new feature benchmark compared stdout without checking completeness.
   A one-frame stub falsely passed as 24 matching frames, recorded in
   `/tmp/b1-length-benchmark-repro.log`. The verification-tool regression rejects
   empty, partial, duplicate and out-of-range frames, failed summaries and
   sanitizer reports. All seven rejected cases first failed the regression in
   `/tmp/b1-length-benchmark-test-repro.log`. The benchmark now requires exactly
   indices 0 through 23, a successful 24-frame summary and finite CPU data.
   Explicit checks work under optimized Python; `stage_total` describes the
   sum of measured per-frame stages, excluding setup and parsing.

Follow-up review closes both findings with no remaining issues. The reviewer
also checked the emitter-mask gate and actual clipping, independent fixture
and benchmark conversions, ownership, count bounds, fingerprints and legacy
bypass. The final verification gates below pass.

## Coverage of the feature

Seven XML cases cover all completed hosts, stored units/source lines,
partial and unsized scope, mixed keys, additive anchors, prepared physics,
shuffled warm rendering and exact pixel-reference identity at 1/4 threads.
They also cover grammar and byte bounds, version exceptions, deferred gates,
emitter-mask clipping, load-time count boundaries, runtime diagnostics,
source fingerprints and 288 fixed-seed valid/mutated/truncated inputs.

XML OOM replay covers 58 allocations, including every injected failure and
leak check. Existing geometry, compositor and physics OOM regressions remain.
The new 128x96 golden exercises group scopes, mixed-unit motion, masks on
all supported hosts, particles and physics. Frames 0, 12 and 23 were generated
and visually inspected. No old reference or integration hash was regenerated.

## Final verification

- SDK Release, ASan/UBSan and coverage: **72/72 CTests each**, after both review
  fixes. Integration and all 19 golden frame-order cases pass. The three new
  reference images match at 1/4 threads; all prior references are unchanged.
- Coverage: **91.90% lines / 75.76% branches**. The new XML lengths module has
  100% line / 92.86% branch coverage. Floors rise to 89.85% / 73.75% and pass
  against the same counters without rerunning or accumulating coverage tests.
- Final legacy oracle against preserved `f100ec1`: **309/309 previews over 42
  fixtures, 3/3 encodes, 2/2 expected rejections**. No old golden, integration
  hash or performance baseline changed.
- Strict 2% performance budget: **PASS**, after all verification jobs ended.
  Median stage changes: clear -1.1%, lighting -18.3%, composite -14.5%, viewport
  -20.1%, effects -18.1%, conversion -15.1%. Stage-total CPU is 8.536 seconds
  versus baseline 10.437 (-18.2%). The runs remain noisy and do not establish
  a stable speedup. Baseline SHA-256 remains
  `215804d165a2bf161c79e044dab98a01f64564995c1259c1b43e0b30d2a853b4`.
- New-feature cost, `python3 tools/length-benchmark.py`: 5,125 nodes, 5,120
  masks, 24 frames, five alternating measurements at four threads. Every
  frame hash matches the independently authored pixel scene at 1/4 threads
  and throughout timing runs. Median compositor CPU: 0.173182 seconds for
  pixels, 0.190701 for relative lengths (**+10.1%**); per-frame stage total:
  0.181982 versus 0.198233 (**+8.9%**). Compositor ranges are 0.158461–0.183784
  and 0.183792–0.208858 seconds respectively. This dense geometry stress case
  measures the extra traversal and conversion, not a universal cost bound;
  parsing/setup CPU is excluded from both stage totals.

Final logs: `/tmp/b1-length-xml-{release,asan,coverage}-final.log`,
`/tmp/b1-length-xml-oracle-final.log`, `/tmp/b1-length-xml-coverage-floors.log`,
`/tmp/b1-length-xml-perf.log` and `/tmp/b1-length-feature-perf.log`.
All jobs are terminal. B1-2 implementation is complete; batch merge and
B1-3 through B1-6 remain outstanding.
