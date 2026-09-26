# B1-3 shared randomness prerequisite

The first implementation step extracts the existing particle mixer and seed
derivation into the shared random module required by dissolve. It pulls
forward the neutral part of B2-0 without adding gradient noise, fBm or new
seed rules. Public particle entry points remain compatible.

The shared private inline implementation retains the particle hot path's
inlining. `random.c` supplies the public helpers and owns the legacy seed
scan. The unsigned arithmetic, stream/index wraparound, high-53-bit conversion,
historical hash offset, unsigned id bytes, null-id behavior and explicit-seed
override remain unchanged. The XML reference now records those exact rules.

Three fixed-vector cases cover the mixer, particle values and seed derivation,
including maximum integers, stream values above seven, UTF-8 bytes and an
explicit zero seed. Existing particle tests cover frame/thread independence
and the renderer oracle covers old outputs. The change introduces no parser,
allocation, mutable state, external input or fingerprint policy; there is no
new allocating operation to add to OOM replay. Existing OOM checks still run.

The independent read-only diff review found no issues and verified both build
system registrations. The full design review is recorded separately in
`b1-compositing-design.md`. No B1-3 capability is enabled by this prerequisite.

## Verification

- SDK Release, ASan/UBSan and coverage each pass **73/73 CTests**, including
  integration, OOM, all prior goldens and all 19 golden frame-order cases.
  The new random suite contains three fixed-vector cases. No sanitizer
  report or failed test remains.
- Coverage is **91.88% lines / 75.77% branches**, above the unchanged
  89.85% / 73.75% floors. `random.c` has 100% line and branch coverage.
- The oracle against preserved `6eb035b` matches **309/309 previews across
  42 fixtures, 3/3 encodes and 2/2 expected rejections**. Old golden images
  and integration hashes are unchanged.
- The strict **2% performance gate passes**, after all other verification
  processes ended. Median changes are clear -1.1%, lighting -19.8%, composite
  -17.0%, viewport -17.8%, effects -17.0% and conversion -16.9%. Stage-total
  CPU is 8.548 seconds against 10.437 baseline (-18.1%). Measurements remain
  noisy and do not establish a stable speedup. The owner baseline is unchanged:
  SHA-256 `215804d165a2bf161c79e044dab98a01f64564995c1259c1b43e0b30d2a853b4`.

Logs are `/tmp/b1-compositing-random-{release,asan,coverage,oracle,perf}.log`.
All jobs are terminal. The independent review's verification conditions are
satisfied. Visible B1-3 features and the later batch items remain outstanding.
