# B1-3 prepared vector paths

This neutral prerequisite exposes the existing flattened path as an owned
internal value. Parsing owns contour/point storage; raster calls borrow that
immutable storage and own their scratch buffers. Freeing resets the value,
and unsuccessful parsing releases partial storage before returning an empty
value. Public path checking and coverage wrap the same implementation.

The extraction preserves the legacy grammar, fixed quadratic/cubic
subdivisions, fill/stroke arithmetic and allocation order. It adds no new
descriptor allocation. Advanced-mask coordinate/count limits and coverage
origins remain separate work before enabling those consumers; this commit
does not change any feature capability or legacy acceptance policy.

## Review and focused verification

Read-only review found no actionable findings in ownership, OOM cleanup,
determinism, concurrent reuse or legacy behavior.

The path suite adds independent Bezier midpoint/end references, an exact
fractional-rectangle coverage reference, invalid-output checks and reuse at
different dimensions. Eight raster jobs share a prepared path across four
workers with disjoint outputs; a deep storage hash and descriptor snapshot
remain unchanged. OOM replay covers successful parse/raster allocation and
partial malformed-path cleanup. Focused path, vector and OOM CTests pass.

An initial test incorrectly expected argument rejection for dimensions whose
product actually fits in 64-bit size_t. That request correctly reached the
allocation and returned SR_ERR_MEMORY. The invalid test expectation was
removed before the sanitizer/full runs; no renderer fix was necessary.

## Full verification

- SDK Release, ASan/UBSan and coverage each pass **75/75 CTests**, including
  all 21 golden frame-order checks, integration and OOM replay. There are no
  sanitizer reports. Logs are `/tmp/b1-prepared-path-release-oracle.log`,
  `/tmp/b1-prepared-path-asan.log` and `/tmp/b1-prepared-path-coverage.log`.
- Coverage is **92.07% lines / 76.46% branches**. The line floor remains
  90.05%; the branch floor rises from 74.25% to 74.45%. Rechecking the same
  full-suite counters passes both floors; the log is
  `/tmp/b1-prepared-path-coverage-raised.log`.
- The oracle against the preserved `cef4400` executable matches **309/309
  previews across 42 scenes, 3/3 encodes and 2/2 expected rejections**.
- Existing goldens, the integration hash manifest and the performance
  baseline are unchanged. No new visible feature or golden is introduced.

The strict **2% baseline performance gate passes**: clear +1.5%, lighting
-14.6%, compositor -11.2%, viewport -13.1%, effects -11.1% and conversion
-11.3%. Stage-total CPU is 9.111 seconds versus baseline 10.437 (-12.7%).
The measurements are noisy and do not establish a stable speedup. Timing
ran after all build/test/oracle processes were terminal; the process check
found no other scene-render job. The log is `/tmp/b1-prepared-path-perf.log`.
The baseline SHA-256 remains
`215804d165a2bf161c79e044dab98a01f64564995c1259c1b43e0b30d2a853b4`.
All verification jobs are terminal, satisfying the review's remaining gates.
