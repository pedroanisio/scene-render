# B1-3 shared compositor types

This neutral prerequisite moves six existing values into
`src/compositor_internal.h`: `SrClip`, `SrMaskEval`, `SrMaskLink`, `SrTarget`,
`SrCardTest` and `SrDrawContext`. Fields, order and qualifiers are unchanged.
The header describes borrowed storage, queue lifetimes and propagation of the
root lighting handoff from copied contexts. It includes its own dependencies.
No function, allocation, queue record, deformation implementation or render
loop changes. Callbacks will be added with the coverage/matte/adjustment
consumers that require them; this commit adds no speculative callable API.

Read-only review found no actionable issues. A declaration comparison checks
all six moved definitions verbatim, and a comment/whitespace-insensitive
comparison finds no other changed compositor tokens except the new include.
The new header compiles alone under the SDK C11 warning policy.

The rebuilt Release executable is byte-for-byte identical to the preserved
`a41f3c3` executable. Both SHA-256 values are
`abbf380c15ae3432146d4b4a595bea52e2001a5e113e5e78d248c19ce47009bd`.
This directly proves the extraction did not change executable behavior.

Final SDK Release, ASan/UBSan and coverage each pass **78/78 CTests**,
including all 21 golden frame-order checks, integration and OOM replay. No
sanitizer report appears. Coverage is **92.22% lines / 76.86% branches**;
existing floors **90.20% / 74.80%** pass. The oracle against `a41f3c3` matches
**309 previews across 42 scenes, three encodes and two expected rejections**.
Logs are `/tmp/b1-compositor-types-release-oracle.log`,
`/tmp/b1-compositor-types-asan.log` and `/tmp/b1-compositor-types-coverage.log`.

No new feature, input, allocation or parser is introduced; existing unit,
golden, frame-order, OOM and integration suites cover the change. No golden,
integration hash or baseline changes.

The strict **2% baseline performance gate passes**: clear -4.1%, lighting
-12.1%, compositor -13.8%, viewport -14.0%, effects -11.1% and conversion
-12.8%. Stage-total CPU is 9.160 seconds versus baseline 10.437 (-12.2%).
These noisy measurements do not establish a speedup; the executable is
identical to the preceding checkpoint. Timing ran after all other verification
jobs were terminal, with no other scene-render process found. The log is
`/tmp/b1-compositor-types-perf.log`. The baseline SHA-256 remains
`215804d165a2bf161c79e044dab98a01f64564995c1259c1b43e0b30d2a853b4`.
All verification jobs are terminal.

The next resource-accounting implementation must address the concrete
allocation and cache-history audit in `b1-compositing-resources.md`. That audit
records remaining work, not implemented consumers.
