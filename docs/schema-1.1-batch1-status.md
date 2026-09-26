# Batch 1 implementation evidence

The complete objective remains B1-0 through B1-6 in the accepted proposal.
This file records evidence and outstanding work; partial infrastructure is
not batch completion.

Loader worktree: `/tmp/scene-render-b1`, branch `b1-0-loader`.
Animation worktree: `/tmp/scene-render-b1-anim`, branch `b1-1-animation`.
B1-2 worktree: `/tmp/scene-render-b1-lengths`, branch `b1-2-lengths`.
Active B1-3 worktree: `/tmp/scene-render-b1-compositing`, branch `b1-3-compositing`.
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
| B1-0 embedded 1.1, capability and version checks, report CLI | Implemented and reviewed | Batch merge; latest cumulative baseline check passes |
| B1-0 root sections and multiple outputs | Still gated | Implement and enable alongside dependent items below |
| B1-1 property registry | Committed `0dd2068`, reviewed and verified | Batch merge; Release/ASan/coverage 65/65, oracle 309 previews + 3 encodes |
| B1-1 curves, handles, extrapolation, additive and timeBase | Implemented, reviewed and verified in animation worktree | Batch merge |
| B1-1 new animation hosts | Material/audio implemented, reviewed and verified | Batch merge; new-node hosts alongside B1-4/B1-5 |
| B1-2 relative lengths and parent-box evaluation | Implemented, enabled, reviewed and verified | Batch merge |
| B1-2 style tokens | Implemented, reviewed and verified | Batch merge |
| B1-2 metadata/container tags | Implemented, reviewed and verified | Batch merge |
| B1-3 blend modes, skew, mattes, masks, adjustment nodes | 22 new color modes, skew, structural preparation, bounded paths and shared types verified | Dependency graphs and surface/work accounting, dissolve/parent operators, advanced masks, mattes, adjustment nodes and remaining goldens |
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

## Typed length core

The core retains unit tags on bases/keys, parses all five relative suffixes
with bounded grammar, and converts a constant-size key neighborhood before
interpolation. Static conversion and mixed-unit curves use output-frame or
parent-box references, preserve authored values and allocate nothing. XML
capabilities remain gated until geometry and physics consumers are integrated.

Read-only review found a stale relative-track flag after re-finalization and
unbounded legacy-curve key times on new relative tracks. Both were reproduced
as failed tests and fixed; follow-up found no remaining issues. Final SDK
Release, ASan/UBSan and coverage pass 69/69 each, including all 18 golden
frame-order checks. Ten length cases include 320 parser mutations plus
truncations, bounds, failure outputs and independent literal-track comparisons.
The oracle against `7df7caa` matches 309 previews, three encodes and two expected
rejections. Coverage is 91.64% lines / 74.84% branches; floors 89.60% / 72.80%.

The latest cumulative strict 2% baseline check **passes**: clear +1.9%, total
-17.5%, all other material stages below baseline. Stages are still marked
noisy, so this does not establish a stable speedup. Earlier failed measurements
above are retained as history; this passing result supersedes their open
baseline-check status for the current tree. New visible features still need
their own performance verification. The original baseline and all old hashes
remain unchanged. See `docs/reviews/b1-length-core.md`.

The next geometry integration must preserve two reviewed consumer exceptions:
hidden card pivots still sort, and zero-opacity projective-card descendants
still contribute bounds. Their literal-reference and frame-order cases are
required by the updated relative-length design.

## Relative geometry integration checkpoint

The geometry integration slice added bounded per-frame node/mask geometry, typed XML fields,
and compositor consumers for drawing, sorting, masks and projective bounds.
Inner card contexts borrow the outer geometry table. At that checkpoint relative XML forms
remained gated, including animation keys; existing-attribute relative forms
require 1.1, while new group dimension forms retain the 1.0 minimum.

The first SDK Release build exposed an unused local after the compositor
refactor; removing it restored the build. All 12 targeted schema, length,
profile, XML, styles, property, compositor, depth, mask, group and deformation
checks pass. New profile tests cover every relative suffix on static x and
position, opacity and color keys in both versions. Logs are
`/tmp/b1-length-geometry-initial.log` and `/tmp/b1-length-geometry-build.log`.
These initial checks were followed by five geometry and seven physics cases,
plus a depth-sorting regression and complete OOM replay. Rigid/soft preparation
uses resolved base geometry and preparation-owned constraint defaults. Cache
version 5 fingerprints units, scopes and prepared constraints. New tests cover
literal pixels, actual viewport rendering, warm/shuffled times, 1/4 threads,
immutable authored values and hidden-card lighting flush boundaries.

Read-only review found two ignored-value evaluation errors: unrelated bases
were resolved during physics preparation, and position tracks were evaluated
before physics poses replaced them. Full-render/late-frame tests reproduced
both. The corrected consumer selection and position override pass follow-up
review with no new findings. OOM replay covers 9 geometry, 164 projective-card
and 23 physics allocations, with no leaks and recovery checks.

Full SDK Release, ASan/UBSan and coverage pass 71/71 each, including all 18
golden frame-order checks. Coverage is 91.78% lines / 75.50% branches; floors
rise to 89.75% / 73.50%. The oracle against `430c8f9` matches 309 previews
across 42 fixtures, three encodes and two expected rejections. Existing hashes
and the baseline are unchanged. Strict 2% baseline performance passes: clear
-6.7%, total -22.7%, all material stages below baseline. Measurements remain
noisy and do not establish a stable speedup; no baseline update was made.
See `docs/reviews/b1-length-geometry.md`. At that checkpoint public XML fixtures, semantic/fuzz coverage, reference
documentation and the relative-length golden remained before capability
enablement. The subsequent milestone below completes those requirements.


## Relative-length XML completion

B1-2 is implemented, enabled, reviewed and verified. Static lengths and mixed
animation keys work on all completed hosts; new group dimensions follow the
1.0 new-attribute exception. Emitter masks require 1.1. Load-time count checks,
source diagnostics, XML/pixel fixtures, 288 seeded valid/mutated/truncated
inputs and 58-allocation loader OOM replay cover the public surface. Three new
golden frames were visually reviewed; old references remain unchanged.

Two review findings were reproduced before fixing: missing composition source
lines on count failures and incomplete benchmark-output verification. Follow-up
review has no remaining findings. Final SDK Release, ASan/UBSan and coverage
pass 72/72 each, including all 19 golden frame-order cases. Coverage is 91.90%
lines / 75.76% branches, with raised floors 89.85% / 73.75%. The final oracle
against `f100ec1` matches 309 previews, three encodes and two expected rejections.

The cumulative strict 2% performance check passes on the final tree: clear
-1.1%, stage total -18.2%, every material stage below baseline. Measurements
remain noisy; this is not a stable speedup claim. The original baseline is
unchanged. A new 5,125-node mixed-unit stress benchmark matches every frame
against its literal reference at 1/4 threads and measures +10.1% compositor
CPU (+8.9% per-frame stage total), documented with ranges in
`docs/reviews/b1-length-xml.md`. This final cumulative check also closes the
historical open baseline checks for the preceding animation/metadata slices.

B1-3 through B1-6, dependent loader/animation hosts, batch merges and the final
completion audit remain. No merge or push is included in this milestone.

## Compositing design and shared randomness

The B1-3 design is reviewed and committed as `54c094d`. It defines the complete
blend, skew, mask, matte and adjustment contract, including bounded evaluation
dependencies, private source rendering and legacy fast paths. Review corrected
adjustment-card resampling identity and animated star contour ownership before
feature implementation. See `docs/reviews/b1-compositing-design.md`.

The first neutral prerequisite extracts stateless particle randomness into
the shared module, preserving the old APIs, arithmetic, historical id hash,
explicit seed behavior and hot-path inlining. Three fixed-vector tests pin
compatibility. The read-only code review found no issues. Release, ASan/UBSan
and coverage each pass 73/73 CTests, including all 19 frame-order cases.
Coverage is 91.88% lines / 75.77% branches, above unchanged floors; random.c
has full line/branch coverage. The `6eb035b` oracle matches all 309 previews,
three encodes and two expected rejections.

The strict 2% performance check passes with an unchanged baseline and output
hashes: clear -1.1%, stage total -18.1%, all material stages below baseline.
No stable speedup is claimed from these noisy measurements. See
`docs/reviews/b1-compositing-random.md`. At that checkpoint all B1-3 visible
features remained gated. The next milestone below enables the color modes.

## Compositing color modes

The 22 new color modes are implemented and enabled for groups, layers,
shapes and particle emitters in 1.1. The six original modes preserve their
enum values and arithmetic. Pure kernels allocate nothing and retain no
frame/thread state. Twelve test cases cover independent numerical vectors,
nonseparable invariants, all XML hosts, parser mutations, immutable scenes,
warm/shuffled times and one/four-thread rendering. Three new blend-sheet
references were visually reviewed; old references remain unchanged.

Independent review closed two reproduced findings: the design now explicitly
documents plus-lighter's zero-alpha no-op on HDR backdrops, and the shared
benchmark helper requires fresh metrics for each run. Six verification-tool
tests pass after the helper fix. Full Release, ASan/UBSan and coverage pass
74/74 each, including all 20 golden frame-order cases. Coverage is 91.97%
lines / 76.07% branches, with raised floors 89.95% / 74.05%; the color kernel
module has full line/branch coverage. The oracle against `66fda60` matches
309 previews, three encodes and two expected rejections.

The cumulative strict 2% performance gate passes: clear -4.9%, compositor
-25.4%, stage total -25.0%, all material stages below baseline. The original
baseline is unchanged and the noisy results do not establish a stable
speedup. The feature fixture matches every frame at one/four threads and
across five alternating pairs; its overlapping 22-mode stack costs +65.1%
compositor CPU versus normal blending (+62.8% stage total). Detailed evidence
and timing ranges are in `docs/reviews/b1-compositing-colors.md`.
At the color-blend checkpoint skew, masks, dissolve/parent operators, mattes
and adjustments remained in B1-3. The next milestone below implements skew;
B1-4 through B1-6 and the final batch gates are still outstanding.

## Skew transforms

Static skew attributes and animated skew properties are implemented on all
four completed 2D node hosts. Static attributes follow the new-attribute
exception for 1.0; animation names require 1.1. Ordered Kx/Ky matrices feed
ordinary drawing, inherited masks, card sorting/bounds/projection and soft
rest poses. Zero axes preserve old operations; invalid evaluated skew and
unusable skewed transforms fail explicitly. Physics cache version 6 includes
both bases; rigid collision samples keep their existing behavior.

Eight cases cover independent matrix/pixel/free-fall references, 96 XML
mutations, parser/version/runtime errors, cache invalidation, explicit-zero
identity, immutable scenes and warm/shuffled one/four-thread renders. Three
new skew goldens were visually reviewed. Review reproduced and closed lost
root-lighting state, missing projection diagnostics and animation version
gating. The follow-up review has no remaining actionable findings.

All 75 tests have passing evidence in Release, ASan/UBSan and coverage:
each full run passed 74/75; a test-fixture camera correction then passed the
eight-case skew suite in all three configurations without renderer changes.
All 21 golden frame-order cases, integration and OOM passed in the full runs.
Coverage is 92.06% lines / 76.32% branches; raised floors 90.05% / 74.25% pass.
The `7f02ec7` oracle matches 309 previews, three encodes and two rejections.
The strict 2% baseline gate passes (clear -1.8%, compositor -20.5%, total
-20.5%); the original baseline remains unchanged and noisy measurements do
not establish a stable speedup. Feature cost is +24.8% compositor CPU versus
zero skew (+25.4% stage total), with each variant matching all hashes at
one/four threads and five alternating timing pairs. Evidence and ranges are
recorded in `docs/reviews/b1-compositing-skew.md`.

Shared graph/surface limits remain a required B1-3 preparation step for all
new compositing features, including direct-C scenes. Advanced masks, parent
operators/dissolve, track mattes and adjustment nodes remain, followed by the
later batch items and final merge/completion gates.

## Prepared vector-path prerequisite

Path parsing now exposes owned immutable flattened geometry for reuse by
advanced-mask preparation. Existing public path functions use the same
parser, subdivision and coverage calculations; no capability is enabled by
this extraction. Independent geometry, four-thread reuse, immutable-storage
and allocation-failure tests pass. Read-only review found no issues.

Full SDK Release, ASan/UBSan and coverage each pass 75/75 CTests, including
all 21 golden frame-order checks. Coverage is 92.07% lines / 76.46% branches;
floors 90.05% / 74.45% pass. The oracle against `cef4400` matches 309 previews,
three encodes and two expected rejections. Existing image hashes and the
baseline are unchanged. The strict 2% performance gate passes: clear +1.5%,
compositor -11.2% and stage total -12.7%; measurements are noisy and do not
establish a stable speedup. See `docs/reviews/b1-compositing-prepared-path.md`.

The next preparation phase has a reviewed explicit prepare/invalidate
contract for programmatic scenes. It avoids stale plans after direct field
edits without adding a per-frame traversal to legacy scenes. XML prepares
automatically; direct-C callers must invalidate before authored edits and
prepare afterwards. The lifecycle is now implemented with bounded structural
preparation, as described below; the remaining shared limits still need their
consumers. See `docs/reviews/b1-compositing-design.md`.

## Structural compositing preparation

The scene-owned immutable plan validates all authored 2D nodes, including
inactive content, with limits of 65,536 nodes, 262,144 aggregate masks and
256 ancestry levels. A bounded iterative walk rejects cycles, duplicate
ownership and missing backing arrays. XML preflights before recursive
resolution; explicit C preparation follows the same validated lifecycle.
Invalidation disables renderer, compositor and physics entry until a new
preparation succeeds. Legacy scenes retain their existing bypass.

Six new cases include exact boundaries, direct-C lifecycle diagnostics,
immutable frame/thread reuse and XML publication/re-preparation consistency.
Ten preparation/replacement and four invalid-ownership allocation failures
all return SR_ERR_MEMORY without leaks. One review finding was reproduced
before correction: the published XML plan now follows the finalized sorted
tree. Follow-up review found no remaining actionable issues. Final SDK
Release, ASan/UBSan and coverage each pass 76/76 CTests, including all 21
golden frame-order checks. Coverage is 92.14% / 76.63%, and raised floors
90.10% / 74.60% pass. The oracle matches 309 previews, three encodes and two
expected rejections against `caccb24`. The strict 2% performance gate passes:
clear -8.4%, compositor -21.4% and stage total -22.2%; the noisy measurements
do not establish a stable speedup. Existing image hashes and the owner
baseline are unchanged. See `docs/reviews/b1-compositing-preparation.md`.

Dependency capture graphs, mask integration and live surface/work accounting
remain required B1-3 work, along with advanced masks, remaining blend operators,
mattes and adjustments. This phase does not enable another capability.


## Bounded mask-path parsing

The new internal path entry enforces input bytes, explicit/implicit commands,
contours, aggregate flattened points, coordinates and caller-supplied remaining
storage. Growth checks include both old and replacement buffers; failed parses
free partial ownership and report an error category plus byte offset. Legacy
path grammar, flattening and allocation order remain intact. Advanced masks
are still gated until their rendering and scene-ownership integration finish.

Six boundary/geometry cases, 1,280 seeded valid/adversarial inputs and complete
allocation-failure replay pass. One review finding was reproduced before fixing:
finite Bezier sample roundoff at exactly +/-1e9 is corrected only in bounded
parsing, after strict control validation. Follow-up review has no remaining
findings. SDK Release, ASan/UBSan and coverage each pass 78/78, including all
21 golden frame-order checks, integration and OOM. Coverage is 92.22% lines /
76.84% branches; raised floors 90.20% / 74.80% pass. The oracle matches
309 previews, three encodes and two expected rejections against `636ce5e`.
The strict 2% performance gate passes: clear +1.9%, compositor +0.0% and total
-5.6%; the noisy measurements do not establish a stable speedup. Existing
image hashes and the owner baseline remain unchanged. See
`docs/reviews/b1-compositing-mask-path.md`.

Remaining B1-3 work includes live surface/work accounting, mask integration
and rendering, remaining blend operators,
dependency capture graphs, mattes and adjustments. B1-4 through B1-6 and the
batch completion audit also remain required.


## Shared compositor types

Six existing frame-local types now live in `src/compositor_internal.h`, with
unchanged fields, order and qualifiers. The shared header supplies its own
dependencies and documents borrowed storage, queue lifetime and root lighting
handoff. No function or arithmetic changed, and the rebuilt Release executable
is byte-for-byte identical to `a41f3c3`.

Read-only review found no issues. SDK Release, ASan/UBSan and coverage each
pass 78/78 CTests; coverage is 92.22% / 76.86%, with floors 90.20% / 74.80%
unchanged. The oracle matches 309 previews, three encodes and two rejections.
The strict 2% baseline gate passes: clear -4.1%, compositor -13.8%, total
-12.2%; the noisy measurements do not establish a speedup for an identical
executable. No golden, integration hash or baseline changed. See
`docs/reviews/b1-compositor-types.md`.

The follow-on resource audit in `docs/reviews/b1-compositing-resources.md`
identifies actual allocation/work sites and cache-history, thread-count and
queue-borrowing traps. It is an implementation checklist; the shared live
budget and advanced masks remain incomplete, along with the other B1 work.

## First shared-ledger consumers

The reviewed resource policy is now concrete in
`docs/design/b1-compositing-resources.md`. The first implementation increment
adds the ledger, target/prepared-plan/borrowed-depth admission, deterministic
compositor cache reclamation, paired allocation accounting for pools/planes,
queues/mask copies/heap masks/card-sort arrays/depth, and ordinary raster,
mask-copy, clear/depth and band-dispatch work. Seven focused cases and OOM
replay exercise success, exact/short quotas, partial cleanup, thread invariance,
larger prior frames and nested projected cards. Both reproduced review findings
are fixed, with no remaining review findings. Final SDK Release, ASan/UBSan and
coverage pass 79/79 each; coverage is 92.26% / 76.95%, with floors raised to
90.25% / 74.90%. The oracle matches 309 previews, three encodes and two
rejections. The strict 2% baseline performance gate passes (clear +1.0%,
compositor -8.2%, total -12.2%; noisy). No golden, integration hash or baseline
changes. Evidence and explicit remaining consumers are in
`docs/reviews/b1-compositor-ledger.md`.

This is partial accounting: deformation/particles, lengths, geometry/animation
work, effects/lighting, path raster scratch and the wider renderer scope still
need integration. Advanced masks and the other remaining B1 requirements are
unchanged. No complete shared-budget claim or additional capability is enabled.


## Evaluated geometry accounting

The next shared-ledger increment covers relative node/mask arrays, evaluated
modifier parameters/mesh grids and sampled soft offsets, including growth,
clearing, consumed animation work and the full inverse-deformation fallback
before worker dispatch. Cached rigid/soft sample clocks and declared extents
are checked before indexing. Legacy arithmetic and ownership remain unchanged.

Five geometry cases and 52-allocation failure replay cover exact/short quotas,
repeated growth, thread/frame history and clean recovery. Review found no
production-code issues; the sole verification finding was reproduced and closed
by enlarging the deformation fixture to exercise actual parallel dispatch.
Release and ASan/UBSan pass 80/80 tests each, with the expanded test rerun in
both builds; final coverage passes 80/80 at 92.27% lines / 77.15% branches.
The raised 90.25% / 75.10% floors pass. The oracle matches 309 previews,
three encodes and two rejections. The strict 2% baseline performance gate
passes; alternating geometry compositor cost is +1.74% with overlapping ranges.
No golden, integration hash or baseline changes. Full evidence is recorded in
`docs/reviews/b1-compositor-geometry.md`.

Accounting remains incomplete for particles/cache lifecycle, other animation
and card geometry work, effects/lighting, path raster scratch and the wider
renderer scope. Advanced masks, remaining blends, dependency captures, mattes,
adjustments, B1-4 through B1-6 and batch completion remain required. No new
capability is enabled by this increment.


## Particle resource accounting

The next increment extends the shared ledger to particle output growth and
keyed-walk scratch. Preparation aggregates authored rate-cache capacity;
invalidation clears the old prepared emitter inventory and final preparation
clears newly admitted caches before publication. Cold integration and actual
deterministic candidate work are charged independently of cache warmth.
Closed-form emission-index overflow now fails explicitly on bounded paths.
Legacy evaluation keeps its arithmetic and behavior.

Focused tests cover thread/frame/cache history, extended animation, independent
work bounds, exact/short quotas, edit-and-shrink recovery, aggregate capacity,
invalid metadata and diagnostics. Nineteen particle allocation failures return
memory errors with complete cleanup. Release, ASan/UBSan and coverage pass
81/81 tests each; coverage is 92.33% lines / 77.23% branches, with raised
90.30% / 75.20% floors. The oracle matches 309 previews, three encodes and
two expected rejections. Review findings are closed.

The strict 2% performance gate remains **open**: the final 31-run check fails
on clear (+3.5%, noisy), despite lower total stage CPU (-10.7%). The particle
feature benchmark measures +2.89% compositor CPU, about 0.029 ms per frame,
with overlapping ranges and identical before/after one/four-thread output.
This functional checkpoint is not ready for merge. The baseline, goldens and
integration hashes are unchanged. All results, including the earlier failed
timing runs and the passing reference control, are retained in
`docs/reviews/b1-compositor-particles.md`.

Remaining accounting includes other animation/card geometry, effects/lighting,
path scratch, the wider renderer scope and aggregate new authored/path ownership.
Advanced masks, remaining blends, dependency captures, mattes, adjustments and
B1-4 through B1-6 remain required. No additional capability is enabled here.


## Node and card evaluation accounting

Compositor track evaluation, child/card-list traversal, bounded in-place sorting,
recursive content bounds and projective preparation/warp work now use the shared
ledger. Scene clocks and camera/object counts are checked before evaluation;
consumed finalized tracks have checked storage, with source ownership preserved
through sort/bounds helpers. Projective clipping checks finite intermediates and
fixed vertex capacity; screen/plane conversions and plane-area multiplication
are checked before use. The legacy path retains its arithmetic and qsort.

Seven focused cases and 25-allocation failure replay cover independent work
bounds, exact/short quotas, unchanged sorting on rejection, real parallel warp
execution, repeated/shuffled histories, relative geometry, 3D-object interleave
and cleanup. Review findings were reproduced before fixes and are all closed.
Initial Release and ASan/UBSan pass 82/82 tests each; final affected-suite reruns
pass in both builds. Final coverage passes 82/82 at 92.25% lines / 77.25%
branches, with floors 90.30% / 75.25%. The final oracle matches 309 previews,
three encodes and two rejections. The final nine-run strict 2% performance gate
passes (clear -1.3%, total -12.5%; noisy). The feature benchmark measures +2.70%
card compositor CPU with overlapping ranges and identical frames at 1/4 threads.
Earlier failed performance runs remain recorded. Full evidence:
`docs/reviews/b1-compositor-evaluation.md`.

B1-3 remains incomplete: effects/lighting and renderer/path ownership/work,
advanced masks, seven remaining blend modes, captures, mattes and adjustments
are still required. B1-4 through B1-6, dependent loader/animation hosts, batch
merges and the full completion audit also remain. The next feature increment
is the remaining blend operators; final resource integration remains required
before B1-3 completion.
