# B1-3 structural preparation

This phase installs the reviewed prepare/invalidate lifecycle and the shared
2D node, mask and ancestry limits. XML checks authored structure before its
existing recursive resolution. Direct-C callers explicitly prepare before
evaluation and invalidate before authored edits. Preparation owns its arrays;
rendering only reads them. Invalidated/failed plans cannot be evaluated or
silently restored. Existing legacy scenes retain their bypass.

An iterative walk includes inactive content, rejects cycles and duplicate
child ownership, checks count/pointer consistency and bounds stack pushes.
The plan records node, parent, ancestry depth and aggregate mask offset.
A balanced AVL lookup uses integer indices across array growth. Lookup cost
is bounded independently of allocator layout; pointer ordering never controls
rendering or resource policy. Allocations check integer products and account
for old and replacement storage coexisting during growth.

This is structural preparation, not completion of B1-3's resource model.
Matte/adjustment dependency edges, capture plans, bounded mask paths and live
surface/work accounting remain required follow-on work. No new capability is
enabled by this phase, and no new external input needs fingerprinting.

## Tests and review

Six structural cases cover lifecycle failures, legacy bypass,
diagnostics, invalid ownership, exact count/depth boundaries, pointer lookup,
shuffled-time one/four-thread frames, immutable plan storage and XML opt-in.
The dedicated OOM replays cover all ten preparation/replacement allocations
and four partial invalid-tree allocations. Every injected failure returns
SR_ERR_MEMORY and releases all tracked storage. Focused compositing, skew,
color-blend, XML and OOM CTest suites pass 5/5.

Read-only review found an initial XML plan ordering inconsistency: preflight
ran before z sorting, while later explicit preparation saw the sorted tree.
A regression compares indices and mask offsets across an unchanged XML scene's
load and re-preparation. It failed five assertions before the fix; the log is
`/tmp/b1-compositing-preparation-order-repro.log`. The implementation retains
early unpublished structural preflight, frees its storage, then builds the
published plan from the sorted tree. The corrected six-case suite and the
four related/OOM suites pass (`/tmp/b1-compositing-preparation-order-fixed.log`).
Follow-up review closes the finding with no remaining actionable issues.

## Final verification

- Full SDK Release, ASan/UBSan and coverage each pass **76/76 CTests** on the
  corrected source, including all 21 golden frame-order checks, integration,
  OOM and the six-case structural suite. No sanitizer reports occur. The logs
  are `/tmp/b1-compositing-preparation-final-{release-oracle,asan,coverage}.log`.
- Coverage is **92.14% lines / 76.63% branches**. Raised floors
  **90.10% / 74.60%** pass on the same full-suite counters; the log is
  `/tmp/b1-compositing-preparation-coverage-raised.log`.
- The oracle against the preserved `caccb24` executable matches **309/309
  previews across 42 scenes, 3/3 encodes and 2/2 expected rejections**.
- A final formatting-only adjustment to test/build files was rebuilt in
  Release; compositing and OOM pass again, 2/2. Renderer source is unchanged.
  The log is `/tmp/b1-compositing-preparation-format-check.log`.
- Existing goldens, integration hashes and the owner baseline are unchanged.

The strict **2% baseline performance gate passes**: clear -8.4%, lighting
-22.0%, compositor -21.4%, viewport -22.8%, effects -22.1% and conversion
-18.9%. Stage-total CPU is 8.125 seconds versus baseline 10.437 (-22.2%).
Measurements are noisy and do not establish a stable speedup. Timing started
after all build/test/oracle jobs were terminal, and the process check found no
other scene-render job. The log is `/tmp/b1-compositing-preparation-perf.log`.
The baseline SHA-256 remains
`215804d165a2bf161c79e044dab98a01f64564995c1259c1b43e0b30d2a853b4`.
All verification jobs are terminal and the review conditions are satisfied.
