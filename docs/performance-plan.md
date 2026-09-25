# Performance and quality plan

Research-backed improvement plan for scene-render 1.1. It is grounded in a
`gprof` profile of this codebase and in cited sources from the doc-ray corpus.

## Baseline (measured 2026-09-25)

Scene: `examples/archive-beacon.xml`. Workload: UHD viewport mode, frames
390–420 (the debris and light-burst section), GCC 13 `-O2`, `--threads 1`.

| Metric | Value |
|---|---|
| Render time, 1 thread | **6.0 s/frame** (unprofiled build) |
| Kernel time | 7% of CPU; ~120k minor page faults/frame (~470 MB freshly faulted per frame) |
| Encoder pipe wait | ~0.4 s/frame |

The table below is gprof self time. gprof saw ~98 of 237 CPU-s. The rest is in
uninstrumented libm (`sin`, `cos`, `atan2`, `asin`, `pow`), `memset`/`malloc`,
and the kernel.

| Function | Share | Stage |
|---|---:|---|
| `blur_worker` | 26% | bloom + glow box blur (2 effects × 2 passes) |
| `render_rows` | 21% (+ libm trig) | equirect → viewport extraction |
| `sr_op_rows` + `sr_mat_point` + `sr_texel` | 15% | 2D compositing |
| `flare_worker` | 7% | lens flare (touches every pixel) |
| `grade_worker`, `merge_worker`, `vignette_worker` | 13% | post effects |
| `lookup` | 5% (2.4 G calls) | transfer LUT, ~10 lookups per pixel per frame |
| `convert_rows`, `sr_clear_rows` | 4% | output colour conversion, frame clear |
| `shade` + `sr_lighting_render` | 3.5% | 3D shading |

**Scaling:** 22 threads give only a small speedup over 1. There are two causes:

- Threads are created and joined on every parallel call.
- Most compositing runs op by op, with a join barrier after each op.

**Scene bug found:** `titleGlow` ends at intensity 0.15, not 0. That runs a
full-frame blur on every frame after 5 s. The fix is a closing key at 0 in
`build_scene.py`.

## Rules for every change

- Frames must stay deterministic. Run the golden SHA tests at `--threads` 1,
  2, 7 and 22.
- Never add `-ffast-math`, `-Ofast` or `-funsafe-math-optimizations`. Keep
  `-ffp-contract=off` (see [C2], [C3]). Add a build check for this.
- Group byte-changing items (tier 2) into one golden-hash refresh.
- Known risk that already exists: glibc libm picks implementations by CPU at
  runtime. The same flags can therefore produce different bytes on different
  hosts [C1].

## Tier 0 — measure first (S–M effort)

1. **Per-stage timers in `sr_render_frame`.** Record wall time and thread CPU
   time for lighting, composite, viewport, each effect, colour conversion and
   pipe write. For parallel stages, also record slowest-worker vs mean. Add a
   `--metrics-trace` JSONL output with one row per frame, so you get p50/p95
   and outlier frames [M1][M5].
2. **Add `ru_utime`/`ru_stime` for self and children** to `--metrics`. The
   children figure is FFmpeg's real CPU cost [M2].
3. **`make profile`** (`-pg -fno-omit-frame-pointer`), plus a script that
   turns gprof output into a flame graph [M3].
4. **`make perf-check`**, kept outside `make test`. It renders a fixed 1080p
   30-frame scene 5 times and takes the median stage CPU time. It fails when
   the result is more than 15% over `benchmarks/baseline.json`, and it also
   asserts the golden SHA [M4][M6].

## Tier 1 — byte-identical speedups

| # | Change | Where | Evidence | Expected |
|---|---|---|---|---|
| 1 | Persistent worker pool with a generation barrier. Keep the same static partitions. | `parallel.c`; `camera.c` stops using its own spawner | pthread create/join per call ×22 per op [P1][P2] | High for node-heavy scenes |
| 2 | Scratch arena sized once per render: blur ping-pong buffers (2 × 133 MB), depth buffer, transfer LUT, job arrays | `effects.c:107`, `lighting.c:217`, `effects.c:273`, `camera.c` | 120k faults/frame; mmap'd allocations fault in fresh pages every time [H1][H2] | ~0.3–0.5 s/frame |
| 3 | Vertical blur over column blocks (4–16 columns, one running sum each) or transpose → horizontal → transpose. Partition on 64 B boundaries. | `effects.c` `blur_worker` | #1 hotspot. Column walk has a 61 KB stride, and neighbouring threads share lines [H3][H4][H5][P3] | High (blur ≈ 26%) |
| 4 | Split `blur_worker` into horizontal and vertical versions, handle edges in head/tail loops, keep sums in scalars | `effects.c` | scalar replacement [H6] | Medium |
| 5 | `restrict` on row pointers. Copy `op` fields into locals before the x-loop. Make `sr_blend_px` `static inline` (or LTO). Add a fast path with no mask and normal blend. | `compositor.c` `sr_op_rows`, `raster.c` | Aliasing forces reloads. Switches and calls block vectorization [C4][C5][C6][C7] | Medium–high on compositing |
| 6 | Fuse grade + vignette (+ merge where legal) into one per-row pass that does a single encode/decode | `effects.c` | 2.4 G `lookup` calls; each pass streams 266 MB [H7][H1] | Medium |
| 7 | Writer thread with a bounded 2–3 slot queue, so encoding frame N overlaps rendering frame N+1. Keep strict frame order. | `renderer.c`, `encoder.c` | pipeline model [P4][P5] | Up to the encode wait (~7%) |
| 8 | `-flto=auto`, and try `-O3` / `-march=x86-64-v3`. Add `make vec-report` (`-fopt-info-vec-missed`). | `Makefile`, `CMakeLists.txt` | cross-TU inlining; vectorizer diagnostics [C6][C8] | Low–medium; check the golden hashes |

## Tier 2 — deterministic, but bytes change once

| # | Change | Where | Evidence | Expected |
|---|---|---|---|---|
| 9 | One rotation matrix per frame instead of `rotate_z/x/y` per pixel (6 trig calls/pixel). Later: per-row incremental rays or a cached direction table. | `camera.c` `render_rows` | loop-invariant code motion [C9] | High (viewport ≈ 21% + libm) |
| 10 | Hoist per-light invariants (`exponent`, directional/spot trig, `sr_anim_eval`) out of the pixel loop. Skip `pow` when n·h ≤ 0. Use integer-exponent squaring. | `lighting.c` `shade` | [C9][G1] | Medium on 3D-heavy frames |
| 11 | Incremental edge functions per row with 1/area precomputed. Hoist rotation trig out of the sphere/box loops. | `lighting.c` | bounding-box rasterization [G2] | Medium on meshes |

## Tier 3 — architecture

12. **Tile-parallel compositing.**
    - Walk the tree serially into an ordered op list, bin ops into 64–128 px
      tiles, and have workers claim tiles from an atomic counter.
    - Each pixel still sees the same op sequence, so output is byte-identical.
    - Hard part: isolated group buffers. Their dirty bounds (`sr_buffer_mark`)
      would race.
    - Evidence: [P2][P3][P6].
    - Expected: about 8–15× on compositing, until memory bandwidth caps it.
13. **Several frames in flight.** Do this only if serial time remains after
    #12.
    - Memory cost is about 0.7–1 GB per frame in flight at 4K.
    - First audit shared mutable state: pools, video decoders, the GPU context.

## Quality track (separate from speed)

- **Mipmaps and trilinear filtering** for image minification and for the
  panorama. Choose the level from the existing `op.aa` footprint. This removes
  shimmer when zoomed out [G3][G4].
- **Anti-aliased 3D edges.** Silhouette coverage for spheres, edge distance
  for triangles. Write depth only when coverage ≥ 0.5 [G5].
- **3 box passes ≈ Gaussian** for bloom and blur (radius r/√3 each). Do this
  after #3 makes blur cheap [G6].
- **Optional Catmull-Rom** for magnification, clamped to the premultiplied
  range [G7].
- **Depth prepass, then shade each pixel once** [G8].

Already done, so skip: premultiplied alpha, anti-aliased 2D shape and particle
edges, table-based sRGB in effects and output.

## Suggested order

Tier 0 → Tier 1 #2, #3, #1, #5 → #6, #7 → Tier 2 (single golden refresh) →
Tier 3 #12 → quality track. Re-profile after each step.

## Sources (doc-ray document : sentence ordinals)

- **C** — compiler and C: LLVM Docs r7 (`49b31c3a`); Muchnick, *Advanced Compiler Design* (`f9fdeeca`); LLVM IR Quick Reference (`89a4587d`).
  - [C1] libm dispatch — general knowledge, not from the corpus
  - [C2] fast-math breaks IEEE: IR QR 1146–1147
  - [C3] FMA contraction: LLVM 2600–2601, 9099
  - [C4] C aliasing: Muchnick 3078, 3199
  - [C5] noalias/restrict and runtime overlap checks: LLVM 410, 414, 13642–13644
  - [C6] inlining and LTO: Muchnick 250, 4493; LLVM 12826–12833
  - [C7] control flow blocks vectorization, unswitching: LLVM 13633, 13637; Muchnick 5648
  - [C8] vectorizer remarks: LLVM 13635
  - [C9] loop-invariant code motion: Muchnick 3924
- **H** — memory hierarchy: Gregg, *Systems Performance 2e* (`1088abf3`); Muchnick ch. 20; *C++ High Performance 2e* (`efa8e5f5`).
  - [H1] ~240-cycle memory latency: Gregg 3109–3110
  - [H2] large allocations use mmap; huge pages: Gregg 4154, 4047
  - [H3] tiling: Muchnick 6643, 6651, 6664–6667
  - [H4] stride-1 access: CHP 1069
  - [H5] copy into contiguous buffers: Muchnick 6425
  - [H6] scalar replacement: Muchnick 6522, 6732
  - [H7] loop fusion: Muchnick 6543, 6617–6618
- **P** — parallelism: CHP; *Rust High Performance* (`96ec0bba`); *Distributed and Parallel Computing* (`b1f90a1e`).
  - [P1] thread creation cost: CHP 3043; RHP 2505–2510
  - [P2] Amdahl: CHP 4104; Gregg 1109–1114
  - [P3] false sharing: CHP 3381–3388
  - [P4] bounded producer/consumer: CHP 3137, 3325
  - [P5] pipeline model: DPC 1414–1415
  - [P6] load balancing: CHP 4142–4172; DPC 1648–1654
- **M** — measurement: Gregg; *BPF Performance Tools* (`ea116f42`); *Foundations of Software and System Performance Engineering* (`ce2e64d1`).
  - [M1] drill-down: Gregg 983–985
  - [M2] USE method: Gregg 872, 891
  - [M3] flame graphs and frame pointers: Gregg 2537–2540; BPF 634, 4366
  - [M4] active benchmarking and statistics: Gregg 7448–7819
  - [M5] workload characterization: Gregg 953–956
  - [M6] repeatable automated tests: FSSPE 1450–4123
- **G** — graphics: *Fundamentals of Computer Graphics 5e* (`363e9ddb`); *Computer Graphics: Principles and Practice 2e in C* (`8bea5cf5`).
  - [G1] Blinn-Phong: FCG 1673
  - [G2] incremental edge functions: FCG 2732–2737
  - [G3] filter sized to the output: FCG 3566, 4104
  - [G4] mipmaps and trilinear: FCG 4110–4127
  - [G5] coverage compositing: FCG 1351–1360
  - [G6] box³ ≈ Gaussian: FCG 3338, 3347
  - [G7] cubic filters: FCG 4094–4103
  - [G8] shade only visible pixels: FCG 2920; CGPP 10877

**Corpus caveat:** in doc-ray, the bodies of *Modern C* (`3d57d415`) and
*Optimized C++* (`c9022c82`) hold text from other books. Only their tables of
contents are usable.
