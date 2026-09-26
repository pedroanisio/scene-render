# B1-2 relative geometry integration

Base: `430c8f9`. This slice adds evaluated node/mask geometry, compositor
consumers and physics preparation. Relative XML remains gated until the
public XML fixtures, reference documentation and golden are complete.

## Scope and invariants

The compositor owns bounded, reusable per-frame geometry. Drawing, shape
deformation, masks, card pivots and projective bounds consume the same
resolved values. Temporary card buffers borrow the outer table. Unitless
scenes retain their existing scalar path without the geometry allocation.

Physics stores resolved initial positions/dimensions and soft-body anchors
in preparation-owned state. Implicit constraint lengths are derived into
a separate bounded array; authored constraints remain unchanged. Cache
version 5 fingerprints geometry units, scopes, output dimensions and
prepared constraint defaults.

## Review and reproduced findings

The independent reviewer inspected the current diff against the B1-2 design
and guide section 2, read-only. Two P2 findings were reproduced before fixes:

1. Physics preparation evaluated unused bases on unrelated nodes, including
   hidden nodes and bases replaced by nonadditive tracks. Two full-render
   failures are recorded in `/tmp/b1-length-physics-scope-repro.log`.
   Preparation now indexes the bounded tree and marks actual consumers and
   ancestor scopes before resolving values. Rigid bodies do not evaluate
   unused base anchors; soft bodies retain their required anchors.
2. Drawing evaluated animated x/y before the prepared physics pose replaced
   them, allowing an ignored overflowing offset track to fail rendering.
   `/tmp/b1-length-physics-overrides-repro.log` records that failure. The
   geometry pass now obtains the physics position first and skips those
   overridden tracks, while evaluating anchors normally.

Both fixes pass focused Release and ASan/UBSan length-frame, length-physics,
physics and OOM suites. The reviewer found no other early-audit blockers;
follow-up closed both findings and reported no additional issues.

Dedicated geometry tests also exposed a null-diagnostics crash on the new
length error path. `/tmp/b1-length-frame-asan.log` identifies the failing
call; optional diagnostic guards fix it. A projective reference mismatch
came from negating an unsigned test width; correcting that test expression
made the independently authored pixel and relative renders identical.

## Current test evidence

- Five geometry cases cover all mask hosts, sized/unsized/partial scopes,
  animation/base poses, inactive consumers, table bounds/reuse, immutable
  authored state and pixel-reference renders across output dimensions.
- Seven physics cases cover rigid/soft pixel references, base-pose policy,
  every constraint kind and absent/zero/explicit defaults, cache changes,
  cache version rejection, actual viewport rendering, constraint limits and
  both review regressions. Viewport references include nonempty output,
  shuffled/warm frames and 1/4-thread identity.
- The depth suite compares hidden, expired and zero-opacity card pivots among
  visible cards and crossing translucent 3D objects, at shuffled times and
  1/4 threads. Substituting an inactive pivot changes the reference output,
  verifying the fixture exercises sorting and lighting flush boundaries.
- OOM replay covers 9 geometry allocations, 164 projective-card allocations
  and 23 rigid/soft/constraint allocations. Each failure returns
  `SR_ERR_MEMORY` without leaks; geometry and compositor recovery is checked.
- Full SDK Release, ASan/UBSan and coverage pass 71/71 CTests each, including
  integration and frame order for all 18 goldens. Coverage is 91.78% lines /
  75.50% branches; the geometry module is 97.60% / 86.64% and physics is
  95.98% / 79.25%. Floors rise to 89.75% / 73.50% and pass against the same
  counters without rerunning or accumulating coverage tests.
- The legacy oracle against preserved `430c8f9` matches 309/309 previews
  across 42 old-schema fixtures, three encodes and two expected rejections.
  No existing golden, integration hash or performance baseline has changed.
- Strict 2% baseline performance passes after all other verification jobs
  finished: clear -6.7%, lighting -23.0%, composite -21.5%, viewport -23.7%,
  effects -21.5%, conversion -18.2%; total 8.065 versus 10.437 CPU seconds
  (-22.7%). Frame hashes match. Stages remain noisy, so these measurements
  do not establish a stable speedup. The baseline is unchanged; no merge or
  push is included. The new-feature benchmark follows XML enablement.

Full verification logs: `/tmp/b1-length-geometry-{release,asan,coverage}-full.log`,
`/tmp/b1-length-geometry-coverage-floors.log` and
`/tmp/b1-length-geometry-oracle.log` and `/tmp/b1-length-geometry-perf.log`.
