# B1-1 animation curves and track options

This slice implements curves, temporal handles and track options from the
reviewed B1-1 design. Material/audio animation hosts follow separately;
gradient/new-node hosts and resolved group clocks accompany B1-4/B1-5.
It introduces no file inputs or mutable render cursor.

## Regression evidence

- Particle-rate integration previously read raw endpoint values and stopped
  sampling after the key span. A looping additive ramp regression failed
  with the pre-change particle consumer and passes with evaluated rates on
  the fixed integration grid. A spring lifetime regression retains an early
  particle whose lifetime exceeds every key value.
- Physics previously omitted extended key/track parameters from its cache
  signature. Fourteen one-parameter changes reproduced stale cache hits and
  simulation differences. The version-4 signature includes every effective
  new input; cached and uncached simulations now match.
- Nearly critical spring damping exposed cancellation against the critical
  closed form. Stable exponential/trigonometric forms pass the reference
  values for underdamped, critical, overdamped and strongly damped cases.
- Malformed direct-C tracks previously accepted invalid enums/nonfinite
  clocks and crashed on missing key storage. Finalization rejects those
  configurations before sorting or evaluation. The failing sanitizer report
  is `/tmp/b1-anim-config-before.log`.
  A final direct-C regression also rejects a finite normalized spring span
  whose products with the spring frequencies would overflow, before any
  evaluation can produce a nonfinite result.
- Review: incoming handles on the first key and outgoing handles on the
  last key were silently unused. Regressions reproduced successful loads;
  both now fail with key/attribute diagnostics. Evidence:
  `/tmp/b1-anim-handles-before.log`.
- Review: a legal Catmull-Rom rate with keys `(0,0)`, `(1e-12,1e12)`,
  `(1,1e12)` overshot the integer emission-index range. The test reproduced
  a float-to-integer sanitizer error. Grid walks now check cumulative
  indices before integer conversion or decrements, and fail rendering above
  `SR_MAX_PARTICLE_INDEX=9e15`. Evidence:
  `/tmp/b1-anim-particle-limit-before.log`.
- Review: a loop spanning `3*2^-40` sampled at `2^18` lost its phase through
  subtraction of a huge cycle product; ping-pong also lost cycle parity.
  Four positive/negative regressions failed before switching to remainder
  reduction by the span or twice the span. Evidence:
  `/tmp/b1-anim-cycle-before.log`.
- Follow-up review: decimal spans could disagree with the separately
  rounded offset cycle count, and large rounded products could falsely snap
  an actual phase to a boundary. Decimal `0.1`-span and exact-binary
  `3*2^-40`-span cases at positive/negative times reproduce both issues.
  Offset derives its quotient consistently with the remainder; boundary
  snapping also bounds the remainder's distance to the span edge by elapsed
  time roundoff, capped at 1e-7 of the span. Decimal boundaries at 10 and
  1000 seconds also have regressions. Evidence: `/tmp/b1-anim-followup-before.log` and
  `/tmp/b1-anim-snap-before.log`.
- Follow-up review: a valid outgoing-only handle was rejected when the
  default Bezier curve reached the final key. Extended tracks now allow the
  unused final outgoing controls, and tests cover both single-sided forms.
  Extended ordinate-limit errors also identify the source key and `bezier`
  attribute instead of the closing animate tag.

The fixed focused sanitizer run passed curves, particles, OOM and golden
suites (`/tmp/b1-anim-review-fixes.log`). Parser mutation seeds are 7, 4711
and 0x51a8a234, with 48 valid/mutated/truncated cases per seed. New XML and
render fixtures replay all 24 and 12 allocation failures respectively;
failures return the expected status and leak nothing. The final targeted
ASan curve run passes all 16 cases (`/tmp/b1-anim-curves-review-final.log`).

The `tracks.xml` golden adds all extrapolation modes, additive scalar/colour
tracks, local/normalized clocks, looping emission and spring lifetimes to
the normal thread-invariance and frame-order checks. All three new frames
were visually reviewed.

## Verification

- SDK Release, ASan/UBSan and coverage builds each passed all 66 CTests,
  including the 16 golden scenes and full frame-order checks.
- After the final boundary and direct-C preparation corrections, the eight
  affected suites passed again in all three configurations. Both new golden
  scenes also passed sequential, shuffled, sliced and interrupted/resumed
  checks at one and four threads in every configuration.
- Coverage of the changed timeline object was reset after rebuilding;
  coverage from unchanged objects was retained from the full suite. Final
  coverage is 91.06% lines / 73.11% branches. Floors rise from
  88.30% / 69.29% to 89.00% / 70.90%.
- The registry-reference byte oracle passed 309/309 previews across 42
  legacy scenes at 1/3/22 threads, 3/3 full encodes and both expected
  rejection cases. The subsequent boundary/preparation changes affect only
  extended tracks, which none of these legacy scenes use.
- Read-only re-review reports no remaining findings, including the final
  spring preparation guard. All findings were reproduced before verifying
  their fixes.
- Existing golden images and integration hashes are unchanged; six PNGs for
  the two new goldens were reviewed and added.

Final logs: `/tmp/b1-anim-curves-release-final2.log`,
`/tmp/b1-anim-curves-asan-final2.log`,
`/tmp/b1-anim-curves-coverage-final.log`,
`/tmp/b1-anim-release-verified.log`, `/tmp/b1-anim-asan-verified.log`,
`/tmp/b1-anim-coverage-verified.log`, `/tmp/b1-anim-curves-oracle.log`.
Performance remains an open merge gate. The strict 2% fixed-baseline check
reported total +103.7% with noisy stage measurements while another renderer
was active in the main checkout. Five alternating registry-reference/current
pairs still exceeded budget: median paired total +6.73%, clear +3.93%,
lighting +4.44%, composite +7.82%, viewport +10.27%, effects +5.95%, convert
+1.29%. Absolute timings varied sharply within the run (for example,
reference viewport CPU ranged from 2.72 to 5.15 seconds), so these results
do not establish the change's isolated cost. They also do not satisfy the
gate. Logs: `/tmp/b1-anim-curves-perf.log`,
`/tmp/b1-curves-paired-perf.log`, `/tmp/b1-curves-paired-perf.json`.
The recorded baseline has not been replaced; no merge is claimed.
