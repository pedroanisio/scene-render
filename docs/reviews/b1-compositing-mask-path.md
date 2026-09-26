# B1-3 bounded mask-path preparation

This prerequisite adds `sr_prepared_mask_path_parse` to the existing prepared
path module. Both entries use the same SVG subset and fixed 16-piece cubic /
12-piece quadratic subdivisions. The existing path entry retains its grammar,
arithmetic and allocation sequence. No advanced-mask capability is enabled;
scene ownership and coverage integration remain subsequent B1-3 work.

The bounded entry checks the input length before parsing, then enforces the
reviewed command, contour, aggregate flattened-point and coordinate limits.
Implicit repetitions, explicit close commands and closing points count.
Literal values and relative-derived controls/endpoints are checked before
flattening. The caller supplies its remaining live-byte quota; every growth
checks both old and new storage before allocation, and successful parsing
reports retained heap capacity. Failure releases all partial storage and
reports an error category with a zero-based byte offset. Resource rejection
returns SR_ERR_ASSET, separately from SR_ERR_MEMORY.

## Tests and review

Six boundary/geometry cases cover all named parser limits at and beyond their
boundaries, implicit commands, aggregate points across contours, closure,
relative controls, nonfinite values, byte offsets, cleanup, exact storage
quotas and transient growth. Accepted ordinary geometry and coverage match
the legacy entry byte for byte; legacy coordinates above the new limit still
parse. Four fixed seeds produce 256 generated valid paths and 1,024 mutated,
truncated, overflowing-coordinate or reduced-quota variants. Repeated parsing
checks deterministic errors/geometry, all accepted geometry matches the
legacy parser, and owned capacities/counts/ranges are checked independently.
Allocation-failure replay covers all eight allocations on success, malformed
partial input and derived-coordinate rejection, and all four allocations
before storage rejection. Every injected failure returns SR_ERR_MEMORY with
no retained storage or leaks.

The independent reviewer found one P2: a quadratic with all X controls exactly
1e9 can round a sampled X to 1000000000.0000002 and be incorrectly rejected.
Eight regression inputs cover both axes, both signs and absolute/relative
commands. They produced 16 failing assertions before correction in the SDK
ASan build: `/tmp/b1-mask-path-boundary-repro.log`.

The correction clamps only finite bounded Bezier samples at the coordinate
limit, after strict control validation establishes the mathematical convex
hull. Generic points and relative sums retain strict rejection, and legacy
flattening is unchanged. All eight inputs now pass; the follow-up review
closes the finding with no additional actionable issues.

## Verification

- Final SDK Release, ASan/UBSan and coverage each pass **78/78 CTests**,
  including all 21 golden frame-order checks, integration and OOM replay.
  No sanitizer report appears. Logs are
  `/tmp/b1-mask-path-final-release-oracle.log`, `/tmp/b1-mask-path-asan.log`
  and `/tmp/b1-mask-path-coverage.log`.
- Coverage is **92.22% lines / 76.84% branches**. Floors rise from
  90.10% / 74.60% to **90.20% / 74.80%**. Rechecking the same full counters
  passes, in `/tmp/b1-mask-path-coverage-raised.log`.
- The original focused run passed 5/5. A pre-correction Release run passed
  78/78; its subsequent oracle was stopped once the review correction made
  it obsolete. The final Release run above rebuilds the corrected source.

- The final oracle against the preserved `636ce5e` executable matches
  **309/309 previews across 42 scenes, 3/3 encodes and 2/2 expected
  rejections**. Existing goldens and the integration hash manifest are
  unchanged. No new visible feature or golden is introduced.

The strict **2% baseline performance gate passes**: clear +1.9%, lighting
-8.5%, compositor +0.0%, viewport -3.9%, effects -7.6% and conversion -6.8%.
Stage-total CPU is 9.852 seconds versus baseline 10.437 (-5.6%). Measurements
are noisy and do not establish a stable speedup. The log is
`/tmp/b1-mask-path-perf.log`. Timing began only after all build/test/oracle
jobs were terminal and a process check found no other scene-render job.
The original owner-machine baseline SHA-256 remains
`215804d165a2bf161c79e044dab98a01f64564995c1259c1b43e0b30d2a853b4`.
All verification jobs are terminal.
