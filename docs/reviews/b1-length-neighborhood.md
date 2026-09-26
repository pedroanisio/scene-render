# B1-2 timeline neighborhood prerequisite

Base: `0834161`. This is the output-neutral prerequisite from the reviewed
relative-length design. Relative-length syntax remains unsupported while the
rest of that feature is implemented.

A private helper returns at most six sorted, unique indices from a finalized
track: its global endpoints, the interpolation pair for the requested scene
time, and the pair's tangent neighbors. The caller can copy and convert this
constant-size subset while retaining the original clocks, curve parameters
and extrapolation options. Selection is O(log keys), uses no allocation and
never modifies the track. The ordinary in-range, hold and linear evaluator
paths retain their existing arithmetic. Only cyclic time mapping is extracted
into a helper shared with selection; its arithmetic and boundary snapping
are unchanged.

The implementation agent audited the subset against every curve consumer.
The independent read-only reviewer found no defects in the diff, bounded
index selection, exact arithmetic or test coverage. The full correctness matrix below also passes; the performance gate
remains open. No schema or capability changes are included.

## Verification

- SDK Release focused timeline/curves CTests: 2/2 pass.
- New exact-double comparisons cover all 40 curve families, all 25 before/after
  extrapolation pairs, additive/non-additive tracks and five clock mappings.
  Inputs include nonuniform key spacing, temporal handles, steps at boundaries,
  zero/one/two/three-key tracks, negative times, decimal cycle boundaries,
  neighboring representable times, enormous cycles and 65,536-key tracks.
  Source track/key snapshots are unchanged. New production code allocates
  nothing, so no new OOM operation is introduced.
- An additional SDK-compiled comparison links the exact pre-refactor timeline
  source (public symbols renamed) beside the new evaluator. The same 2.18
  million curve/mode/clock/time samples compare old full-track evaluation
  against new selected-track evaluation with zero bit differences. Local
  harness: `/tmp/b1-neighborhood-oracle.c`; source: `/tmp/b1-timeline-before.c`;
  log: `/tmp/b1-length-neighborhood-direct-oracle.log`.
- Full SDK Release, ASan/UBSan and coverage: 68/68 CTests each, including
  integration, allocation-failure replay and all 18 golden frame-order checks.
  Coverage is 91.57% lines / 74.54% branches; floors are 89.55% / 72.50%.
  The stricter branch floor passes without rerunning or accumulating counters.
- Byte oracle against the preserved `d106925` executable: 309/309 previews
  across 42 legacy fixtures, three full encodes and two expected rejections
  match. No old golden or integration hash changed.
- Strict 2% performance check: failed on clear-stage CPU, 0.206 seconds
  versus 0.197 (+4.5%). Total CPU was 8.672 versus 10.437 seconds (-16.9%);
  all material stages were marked noisy. The separate user render had exited
  by the final process check; this measurement does not establish a cause.
  This does not clear the gate. Frame hashes
  match and the original baseline is unchanged; no merge or push is made.

Local logs: `/tmp/b1-length-neighborhood-{release,asan,coverage,oracle}.log`,
`/tmp/b1-length-neighborhood-coverage-floors.log` and
`/tmp/b1-length-neighborhood-perf.log`.
