# Batch 1 implementation evidence

The complete objective remains B1-0 through B1-6 in the accepted proposal.
This file records evidence and outstanding work; partial infrastructure is
not batch completion.

Worktree: `/tmp/scene-render-b1`, branch `b1-0-loader`.
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
| B1-1 property registry | Design reviewed; implementation pending | Host/type/offset/bounds table replaces all duplicated mappings; refactor oracle |
| B1-1 curves, handles, extrapolation, additive, timeBase, new animation hosts | Pending | Closed-form unit references, curve golden, OOM and frame-order evidence |
| B1-2 relative lengths and parent-box evaluation | Pending | All required hosts, per-frame evaluation, scoped percentages, docs and goldens |
| B1-2 tokens and metadata/container tags | Pending | Load-time resolution, unknown-token errors, tags and embedMetadata tests |
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
- Performance measurement pending; no golden or integration hash was refreshed.

Independent verification fixes: `985de3f` keeps pixel kernels shared in
unoptimized builds, restoring meaningful branch coverage; `f26a8cf` corrects
the SDK x265's uninitialized DTS on one/two-frame flushes. Both have read-only
review and regression evidence. Release kernel arithmetic is unchanged.

All final batch gates remain open: every item merged, full new-feature test
matrix, all goldens/frame-order checks, coverage, performance budgets, and
requirement-by-requirement completion audit.
