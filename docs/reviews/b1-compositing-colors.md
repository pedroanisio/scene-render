# B1-3 color blend modes

This milestone implements and enables 22 additional color modes on groups,
layers, shapes and particle emitters. The existing six enum values and
their float arithmetic remain unchanged. New modes use pure, bounded
three-channel kernels with double color intermediates and float output;
they add no allocation, mutable state or external input. Existing raster,
masked, deformed, isolated-group, particle and projective-card paths share
the dispatcher. The seven parent/dissolve operators remain gated.

The XML reference records formulas, endpoint rules, tiny-alpha handling,
HDR contributions, whole-color ties and the explicit transparent-source
exception for plus-lighter. The four nonseparable modes follow W3C
SetSat/SetLum/ClipColor, rather than an HSL conversion. Capabilities require
1.1 on all four hosts; no unrelated construct is enabled.

## Review

The independent read-only review checked determinism, frame order, thread
safety, limits, allocation ownership, fingerprinting and legacy arithmetic.
No C implementation defect was found. Two findings were resolved:

1. The design's unconditional plus-lighter formula conflicted with the
   renderer's transparent-source no-op. A failing regression first showed
   the difference for an HDR backdrop. The contract now explicitly preserves
   the backdrop at zero source alpha, consistent with the drawing paths;
   every positive source alpha, including tiny alpha, sums and clamps.
   The test pins both cases. This clarification was re-reviewed and accepted.
2. The new benchmark reused the length benchmark's metrics validator, which
   could accept an old trace after a renderer emitted complete hashes without
   writing new metrics. A failing regression reproduced the stale result.
   The helper now removes the old trace before each invocation and requires
   fresh, nonempty, valid summary JSON. Regression cases cover no write,
   empty/malformed/non-object output and fresh timing acceptance. All six
   verification-tool tests pass. Follow-up review closed the finding.

There are no remaining actionable findings. The first reproducer is
`/tmp/b1-colors-transparent-contract-repro.log`; the second is
`/tmp/b1-blend-stale-metrics-repro.log`. The fixed verification-tool run is
`/tmp/b1-blend-metrics-fixed.log`.

## Verification

- Full SDK Release, ASan/UBSan and coverage each pass **74/74 CTests**,
  including integration, existing OOM replay and all 20 golden frame-order
  cases. No sanitizer report remains. The Python validator change followed
  those full runs and passed its six focused tests separately.
- The 12-case blend suite includes fixed high-precision scalar and RGB
  vectors, 62,500 nonseparable invariant combinations, transparent/HDR and
  threshold cases, all four XML hosts, version/pending-mode diagnostics,
  96 seeded valid/mutated/truncated XML inputs, immutable scene snapshots
  and warm/shuffled one/four-thread full renders.
- Three new 320x180 golden frames were visually reviewed at frames 0, 12
  and 23. They show all 28 color modes against matching translucent
  backdrops, with animated opacity/rotation. No old reference was refreshed.
- Coverage is **91.97% lines / 76.07% branches**, with `raster.c` at
  **100% lines and branches**. Floors rise to 89.95% / 74.05%.
- The oracle against preserved `66fda60` matches **309/309 previews across
  42 fixtures, 3/3 encodes and 2/2 expected rejections**. Existing golden
  images and integration hashes remain unchanged.
- The strict **2% performance gate passes** after all other render/build
  jobs ended. Median stage changes are clear -4.9%, lighting -23.9%,
  composite -25.4%, viewport -27.0%, effects -24.2% and conversion -23.2%.
  Stage-total CPU is 7.828 seconds against 10.437 baseline (-25.0%). These
  noisy measurements do not establish a stable speedup. The original
  baseline remains unchanged: SHA-256
  `215804d165a2bf161c79e044dab98a01f64564995c1259c1b43e0b30d2a853b4`.

Full logs: `/tmp/b1-colors-{release,asan,coverage,oracle,perf}.log`.
The new feature-cost benchmark is `tools/blend-benchmark.py`; it compares
the same 24-frame stack with only its 22 blend attributes replaced by
normal, verifies each variant independently at one/four threads, then
checks every hash in five alternating timing pairs.

The feature benchmark passed all hashes and measured median compositor CPU
of **0.182702 seconds versus 0.110646 (+65.1%)** for normal blending over
24 frames at four threads. Ranges were 0.179051–0.188997 and
0.104037–0.118891 seconds respectively. Per-frame stage-total CPU was
0.187617 versus 0.115221 seconds (+62.8%), with ranges
0.184196–0.194051 and 0.108005–0.123797. This 192x128 deliberately overlapping
stack includes all new modes, masks, deformations, particles, isolated groups
and projective cards; its cost is workload-dependent, not a per-mode constant.
The log is `/tmp/b1-colors-feature-perf.log`. All measurement jobs are terminal.

B1-3 still requires skew, advanced masks, dissolve/parent operators,
track mattes and adjustment nodes. Later batch items and merges remain.
