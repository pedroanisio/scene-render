# Batch 1 implementation evidence

The complete objective remains B1-0 through B1-6 in the accepted proposal.
This file records evidence and outstanding work; partial infrastructure is
not batch completion.

Loader worktree: `/tmp/scene-render-b1`, branch `b1-0-loader`.
Animation worktree: `/tmp/scene-render-b1-anim`, branch `b1-1-animation`.
Active B1-2 worktree: `/tmp/scene-render-b1-lengths`, branch `b1-2-lengths`.
Authoritative pre-batch base: `5b7dca1` (main advanced during initial setup).
Preserved reference executable: `/tmp/scene-render-b1-reference`.
Builds and tests run only in Flatpak `org.freedesktop.Sdk//25.08`.

## Prerequisites

- Schema errata E1–E9: `d9e7919`; all 42 1.0-valid fixtures validate as 1.1.
- Owner-machine baseline: `65538bd`, recorded from the pre-batch renderer.
- Checked-in equivalence/frame-order tools: `5fc5e41`, hardened in `b11d3d2`.
- Dispatcher refactor: `b11d3d2`, with read-only review in
  `docs/reviews/b1-dispatch.md`.
- Pre-feature verification: Release and ASan/UBSan 56/56 CTests;
  309/309 RGBA previews at 1/3/22 threads, 3/3 full encodes, unchanged goldens.
  The 42 XSD-valid fixtures include two deliberate rejection tests; the
  oracle checks their exit codes and diagnostics instead of rendering them.

## Requirements and remaining work

| Item | State | Evidence still needed |
|---|---|---|
| B1-0 table dispatcher, E_PARTICLES | Implemented and reviewed | Batch merge |
| B1-0 embedded 1.1, capability and version checks, report CLI | Implemented and reviewed | Performance check and batch merge |
| B1-0 root sections and multiple outputs | Still gated | Implement and enable alongside dependent items below |
| B1-1 property registry | Committed `0dd2068`, reviewed and verified | Batch merge; Release/ASan/coverage 65/65, oracle 309 previews + 3 encodes |
| B1-1 curves, handles, extrapolation, additive and timeBase | Implemented, reviewed and verified in animation worktree | Performance gate and batch merge |
| B1-1 new animation hosts | Material/audio implemented, reviewed and verified | Performance and merge; new-node hosts alongside B1-4/B1-5 |
| B1-2 relative lengths and parent-box evaluation | Design and neutral timeline prerequisite reviewed; feature in progress | All required hosts, per-frame evaluation, scoped percentages, docs and goldens |
| B1-2 style tokens | Implemented, reviewed and verified | Batch merge |
| B1-2 metadata/container tags | Implemented, reviewed and verified | Performance gate and batch merge |
| B1-3 blend modes, skew, mattes, masks, adjustment nodes | Pending | Every listed mode/parameter, cycle checks, numerical tests, three new goldens |
| B1-4 shapes, stroke styles, trims, vector constructors | Pending | Geometry/arc lengths/coverage, all listed styles and shapes, golden |
| B1-4 gradients and paints | Pending | Coordinates, focal/aspect/spread/rotation/stops, interpolation, dither, every paint host, golden |
| B1-5 markers/beatGrid/snapping, group timing, sequence, names/tags | Pending | Generated ID resolution, timing tests and sequence-markers golden |
| B1-6 multiple passes/shared encoders/ranges | Pending | Render sharing at equal dimensions/FPS; distinct passes otherwise |
| B1-6 codecs, container/options, poster/thumbnail | Pending | All requested SDK codecs, 1/4-thread container identity, lossless still goldens, PNG/JPEG stills |

## Loader profile verification

- SDK Release CTest: 64/64, including all 14 original goldens and frame order.
- SDK ASan/UBSan CTest: 64/64, `ASAN_OPTIONS=detect_leaks=0`.
- Coverage CTest: 64/64; 90.74% lines / 72.11% branches, floors unchanged
  at 88.30% / 69.29%.
- Byte oracle: 309/309 RGBA previews at threads 1/3/22 across 42 old-schema
  fixtures; 3/3 full encodes; two expected-rejection diagnostics match.
- Additional short H.265 container exactly matches the pre-batch binary.
- Loader review: three findings reproduced as failures, fixed and re-reviewed;
  no remaining findings. See `docs/reviews/b1-loader-profile.md`.
- The fixed-baseline 2% performance check failed with noisy stage deltas and
  total +19.5%. Nine alternating reference/current measurements found that
  the reference also slowed (12.755 CPU seconds versus baseline 10.437).
  Median paired total difference was +0.65%; composite +0.06%, lighting
  +0.81%, viewport +0.24%, effects -0.29%. Clear +3.63% and convert +2.28%
  remain outside the per-stage budget, so the gate stays open. The machine
  was running a desktop media process at about one CPU core. The original
  checked-in baseline is unchanged.
- No golden or integration hash was refreshed.

Independent verification fixes: `985de3f` keeps pixel kernels shared in
unoptimized builds, restoring meaningful branch coverage; `f26a8cf` corrects
the SDK x265's uninitialized DTS on one/two-frame flushes. Both have read-only
review and regression evidence. Release kernel arithmetic is unchanged.

All final batch gates remain open: every item merged, full new-feature test
matrix, all goldens/frame-order checks, coverage, performance budgets, and
requirement-by-requirement completion audit.

## Animation curves and track options

The worktree adds all proposed interpolation families, temporal handles,
extrapolation modes, additive values and affine track clocks. The original
curve arithmetic remains intact. A new 320x180, 24-frame curve sheet has
three visually reviewed references; original references remain unchanged.

Full Release, ASan/UBSan and coverage runs passed 66/66 CTests each. After
the final review corrections, all eight affected suites and both new golden
frame-order checks passed again in every configuration. Curves have 16
unit cases, including numerical references, parser mutations, cache inputs,
large-cycle precision and input bounds. Final coverage is 91.06% lines /
73.11% branches; floors rise to 89.00% / 70.90%. The byte oracle matches all
309 previews, three encodes and two rejection cases against the registry
reference. See `docs/reviews/b1-animation-curves.md` for review findings and
exact verification scope. Performance remains open; this is not completion
of B1-1 or the batch.

The curves fixed-baseline performance run failed at total +103.7% during
concurrent rendering. Five alternating reference/current pairs measured
total +6.73% and several stages above the 2% budget, with strongly varying
absolute times. This does not clear the gate; retain the original baseline
and remeasure/profile under stable conditions before merging.

## Material and audio animation

The host slice animates material baseColor, emissive, metallic
and roughness, plus audio volume/pan. Shared material values live in
render-owned object frames. Audio automation uses absolute sample time and
has no mutable cursor. The new 4096-host limits, key bounds and normalized
audio span have parser boundary/rejection tests and 216 seeded mutations.

Focused Release and ASan suites passed, including independent numeric
material/audio references, overshoot clamps, shuffled audio blocks, and
allocation-failure replay. The host fixture exercises 40 XML allocations,
21 preview-render allocations and both mixer allocations; every injected
failure is clean and leak-free. Three new material goldens were visually
reviewed; 1/4-thread and warm-frame comparisons pass. Integration confirms
identical FFV1/PCM files at 1/4 threads, 96000 audio samples and unchanged
prior preview hashes. The read-only review found no implementation defects.
Full Release, ASan/UBSan and coverage passed 66/66 CTests each, including
frame order for all 17 goldens. The oracle against `0735d72` matched 309
previews, three encodes and two expected rejections. Final coverage is
91.21% lines / 73.50% branches; floors rise to 89.20% / 71.40%. Performance
remains open; see `docs/reviews/b1-animation-hosts.md`.

The host slice's strict performance check failed at total +126.1%, with all
stages noisy during a separate user render using roughly eight CPU cores.
No baseline was updated and no merge gate was waived.

## Style tokens

The B1-2 root-section design is committed in `7a0adb6`. Its checked color
parser prerequisite is committed in `160b8c1`, with full 66/66 Release,
ASan and coverage runs and an unchanged byte oracle. See
`docs/design/b1-2-styles-metadata.md` and `docs/reviews/b1-color-status.md`.

Style tokens now resolve at load time, including forward project-background
references, aliases and animated color keys. All 13 existing color-consumer
contexts use the shared helper. Names, values, counts and alias depth are
bounded; missing references, cycles and duplicates fail with source positions.
The original XML remains the resume fingerprint input. Rendering performs no
token lookups or mutations.

Targeted tests cover every color host, exact limits, declaration-order
independence, 216 seeded parser mutations and warm/shuffled literal-versus-token
renders at 1/4 threads. OOM replay covers 120 allocations with no leaks.
Three new token golden references are byte-identical copies of the existing
material-animation references, visually reviewed at frames 0, 12 and 20.
The reviewer found one frame-order tool issue: inserting an output before
styles violated schema ordering. The failing regression was reproduced and
fixed; follow-up review found no remaining issues. Release, ASan/UBSan and
coverage each passed 67/67 CTests, including frame order for all 18 goldens.
The byte oracle matched all 309 previews, three encodes and two expected
rejections against `160b8c1`. Coverage reached 91.50% lines / 74.11% branches;
floors rise to 89.50% / 72.10%. See `docs/reviews/b1-styles.md`.

Style tokens are committed as `c0df479`. Relative lengths remain required
B1-2 work. The batch's strict performance gate remains open, with its original
baseline unchanged.

## Metadata and codec worker policy

The metadata slice retains the ten schema attributes and custom entries with
bounded scene ownership. Authored tags round-trip through MP4, MOV and Matroska,
including empty, Unicode and multiline values. Container-specific reserved keys
and Matroska canonical collisions fail before output or resume work. Disabled
embedding retains entries without submitting tags. Internal segments omit tags;
the final full/resumed output applies them from the scene and checks that the
muxer retained each value.

Cross-thread tests exposed an existing encoder gap: previous bit-exact tests
repeated one codec thread count. Version 1.1 now pins video codec workers and
x265 pools to one while preserving requested render/scaler threads; version 1.0
retains its original settings. A 1.1-only resume policy marker invalidates old
segments. Failing regressions for byte identity, stale segment reuse and x265
option OOM now pass. The read-only reviewer found no remaining production issues.

Focused schema/metadata/encode/fault/OOM/resume CTests passed 6/6. Metadata load
replay covers 56 allocations without leaks. Tests also cover 216 seeded parser
mutations, all limits, final-path overrides, full/resumed tags, XML fingerprint
changes and 256x256/24-frame codec identity at 1/4/automatic thread counts.
The new integration preview hash independently matches a metadata-free 1.0 scene
rendered by the preserved style-token binary. Release, ASan/UBSan and coverage
each passed 68/68 CTests, including integration and frame order for all 18
goldens. Coverage is 91.56% lines / 74.44% branches, with floors raised to
89.55% / 72.40%. The oracle matched 309 previews, three encodes and two expected
rejections against `c0df479`. See `docs/reviews/b1-metadata.md` for scope and
the outstanding performance gate.

The metadata slice's strict 2% performance run failed at total +24.9%
(13.034 versus 10.437 CPU seconds), with every material stage marked noisy
while a separate user render used roughly eight CPU cores. Frame hashes match.
The result does not clear the gate; the original baseline remains unchanged.

## Relative-length prerequisite

The reviewed design is committed as `0834161`, including scoped boxes,
output-frame viewport units, bounded immutable geometry and preparation-owned
physics constraint defaults. A private six-key timeline neighborhood helper
and shared cyclic-time mapping provide constant-size mixed-unit evaluation
without copying complete tracks. Relative-length capability entries remain
unsupported pending the parser, all geometry consumers and physics integration.

The neutral prerequisite passes SDK Release, ASan/UBSan and coverage 68/68
each, all 18 golden frame-order checks, and the byte oracle against `d106925`
(309 previews, three encodes, two expected rejections). A direct comparison
against the exact old evaluator finds zero bit differences across 2.18 million
samples. Coverage is 91.57% lines / 74.54% branches, floors 89.55% / 72.50%.
Read-only review found no issues. Strict performance still fails: clear +4.5%,
total -16.9%, with noisy stages. The separate user render had exited by the
final process check. The baseline and old image hashes are unchanged. See `docs/reviews/b1-length-neighborhood.md`.
