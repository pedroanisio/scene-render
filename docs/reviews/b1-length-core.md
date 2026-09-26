# B1-2 typed length and animation core

Base: `7df7caa`. This implements the data/parser/evaluation portion of the
reviewed relative-length design. XML capability forms remain gated until
per-frame geometry, all rendering consumers and physics integration are done.

The parser preserves unitless finite-double behavior and retains relative
coefficients and units. Relative spellings follow the XSD grammar, are capped
at 128 bytes and absolute coefficient 1e6, and accept no px suffix. Conversion
uses output-frame dimensions or the caller's parent-box axis and bounds
relative results to absolute 1e12 pixels. Positive dimensions reject zero,
negative values and underflow. Failed parsing/evaluation preserves the caller's
output; these operations allocate no heap memory.

Scalar bases and keys retain a zero-default pixel unit. A track flag records
relative keys. The length evaluator converts a constant-size neighborhood and
an additive base before calling the existing track evaluator with original
scene time. It retains endpoint deltas, Hermite neighbors and normalized spring
seconds. Source values, tracks and keys remain unchanged. Ordinary unitless
rendering still calls the existing scalar evaluator.

## Review and verification

The read-only foundation review found no parser/resolver defects. The core
review found two gaps; both were reproduced as failing tests before fixes.
Finalization now recomputes the relative-key flag, so editing a relative key
back to pixels cannot retain the stricter relative-result bound. Relative-key
tracks also take the existing extended time/value/clock bounds, preventing
an overflowing time span from silently producing a finite wrong answer.
The regression confirms the existing additive-base bounds and preserves
ordinary unitless tracks' old accepted time range. The failing harness had
five assertions; the same harness passes after the two fixes. The follow-up
review confirms both findings are closed and reports no new issues. Final-source
Release, ASan/UBSan, coverage, byte-equivalence and strict baseline
performance verification pass.

- Standalone SDK Release foundation: five cases pass, including 320 seeded
  parser mutations and every seed truncation. No new allocating operation.
- Integrated SDK Release timeline/length/curves/physics/anim_color: 5/5 CTests.
- Ten length unit cases cover grammar, byte/value bounds, positive underflow,
  finite arithmetic, preserved failure outputs, track units, zero/one/two-key
  tracks, ignored non-additive bases and old unbounded unitless values.
- Mixed-unit animation compares exact doubles against independently specified
  literal pixel keys at two output/parent box sizes, across all 40 curves,
  five extrapolation modes, additive options and three clocks. Descending
  time samples and alternating boxes retain source snapshots unchanged.
- Final SDK Release, ASan/UBSan and coverage: 69/69 CTests each, including
  integration, OOM replay and frame order for all 18 goldens. Coverage is
  91.64% lines / 74.84% branches, with the length module at 98.84% lines /
  97.22% branches. Floors rise to 89.60% / 72.80%; the stricter floors pass
  without rerunning or accumulating coverage counters.
- The oracle against preserved `7df7caa` matches 309/309 previews across 42
  old fixtures, three encodes and two expected rejections. Existing goldens,
  integration hashes and the original performance baseline are unchanged.
- Strict 2% performance check passes: clear 0.201 versus 0.197 CPU seconds
  (+1.9%); lighting -18.5%, composite -17.1%, viewport -19.4%, effects -16.1%,
  conversion -15.9%; total 8.605 versus 10.437 (-17.5%). Frame hashes match.
  The prior user render and our other verification jobs had finished before
  this run. Stages remain marked noisy; this is a passing measurement, not
  evidence of a stable speedup. No baseline update, merge or push is made.

Review reproduction logs: `/tmp/b1-length-core-review-{repro,fixed}.log`.

Final verification logs: `/tmp/b1-length-core-{release,asan,coverage}-final.log`,
`/tmp/b1-length-core-oracle.log`, `/tmp/b1-length-core-coverage-floors.log` and
`/tmp/b1-length-core-perf.log`.
