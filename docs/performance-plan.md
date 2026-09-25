# Performance and quality plan

Research-backed improvement plan for scene-render 1.1. It is grounded in a
`gprof` profile of this codebase and in cited sources from the doc-ray corpus.

**Revision 2 (2026-09-25, after the libav/P5/text-engine merges, `b58fbac`)**
adds several things:

- measurements of the current code,
- **verified bugs**, which come first,
- corrections to revision 1,
- new sections on the 3D renderer, physics, robustness and media.

Sources for revision 2: four doc-ray research passes, an independent Codex
audit of `src/`, and direct checks by running code and reading it. Findings
that two sources reached independently are marked ⟂.

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

## Current-code measurements (revision 2)

These were measured on an Intel Core Ultra 7 155H with 22 threads, building
`b58fbac` with libav 62 (FFmpeg 8.1).

**`benchmarks/perf-scene.xml` against the pre-merge baseline**
(`make perf-check`, `--threads 4`):

| Stage | Before | Now | Change |
|---|---:|---:|---|
| 3D lighting | 1.50 | 2.36 CPU-s/run | +58% |
| Compositing | 5.07 | 5.97 | +18% |
| Viewport | 2.74 | 3.24 | +18% |
| Effects | 7.56 | 7.32 | −3% |
| Encoding | ≈0 | 1.12 | now in-process x264 |

The sample frames changed, as expected, because upstream regenerated its
goldens. **Refresh `benchmarks/baseline.json` with `--update` once the merge
is final.** `perf-check`'s noise floor should judge a stage when *either*
value is above the floor; today the encode line slips through.

**Single-thread profile of the same scene** (gprof):

| Share | Function |
|---:|---|
| 16% | `blur_worker` |
| 13% | `render_rows` |
| 12% | `sr_op_rows` |
| 10% | `merge_worker` |
| 8% | `sr_mat_point` |
| 7% | `grade_worker` |
| 6% | `sr_lighting_render` |
| 6% | `flare_worker` |

**`_tmp/douriva-margarina-30s.xml`** (flat 2D, 4K, 900 frames):

| Metric | Value |
|---|---|
| Wall time | 233 s (v1.1 before the merge: about 4 s/frame) |
| Engine CPU | 2,305 s |
| Compositing | 161 s wall / 2,078 s CPU |
| Encode | 54 s serial on the render thread (23% of wall) |
| Peak RSS | **3.8 GB** |

## Verified bugs — fix first

Each row was checked by reading the code. "Run" means it was also reproduced.

| Sev | Where | Bug | Evidence |
|---|---|---|---|
| **high** | `lighting.c:701` | `frame->width * (uint32_t)n` overflows 32 bits in supersampling; the resolve at `:665` then reads past the buffer | read; Codex |
| **high** | `mesh.c:77`, `lighting.c:576` | OBJ `sscanf("%lf")` accepts `nan`/`inf`, which reach unchecked double→int bound casts (UB) | read; Codex ⟂ research |
| **med** | `encoder.c:195` | `c->thread_count = threads` makes the x264 bitstream depend on `--threads`, contradicting the header's "identical bytes". x265 is pinned; x264 is not. | **run:** 2 vs 8 threads give different MP4 SHAs; same count gives identical SHAs. Decoded frames matched for that scene. |
| med | `lighting.c:584` | Depth and attributes use affine rather than perspective-correct interpolation, so occlusion is wrong | Codex ⟂ research |
| med | `lighting.c:546,570` | A triangle with any vertex outside near/far is dropped whole; there is no clipping | Codex ⟂ research |
| med | `lighting.c:519–522` | Specular is added when n·l ≤ 0, giving highlights on unlit faces | read |
| med | `lighting.c:585,648` | Transparent surfaces write opaque depth and hide geometry behind them | Codex |
| med | `physics.c:698,704,760` | Soft-body damping is applied twice: a dashpot `2ζ√(km)` *and* `exp(−ζh)` decay, treating a ratio as a rate | read |
| med | `physics.c:706` | Allocation failure silently skips soft-body steps, and the altered motion is then cached | Codex |
| med | `compositor.c:523`, `assets.c:191` | Reverse playback crosses the exclusive `clipOut` (frame 30 instead of 29) | Codex |
| med | `compositor.c:237,555` | Op bounds ignore non-grid deformation, so a wave-displaced image is clipped | Codex |
| med | `effects.c:285,357` | Group glow cannot extend into transparent pixels (no exterior halo) | Codex |
| med | `gpu.c:66` vs `color.c:296` | CPU LUT and GPU formula quantize differently, so the GPU fallback changes bytes | Codex |
| med | `resume.c:36` | The resume signature ignores font files, so a replaced `fontFile` reuses stale frames | Codex |
| med | `video.c:414` | The overflow check itself overflows (`INT64_MAX - origin` when origin < 0) | Codex |
| low | `resume.c:126,143`; `xml_physics.c:142` | `fclose` is skipped on short I/O; mesh-warp points leak on rejection | Codex |
| low | `audio.c:476` | A hard-panned *stereo* source is +3 dB on one side (the law is documented for mono) | read |

## Corrections to revision 1

- **Encoder overlap (#7):** there is no longer a pipe. Encoding (sws + x264)
  runs synchronously on the render thread; it was 23% of wall on the Douriva
  render. #7 still applies and is now worth more.
- **Effects fusion (#6):** fusing to a *single* encode/decode is not byte
  preserving. Keep the intermediate LUT quantization, or move it into the
  tier-2 golden refresh (Codex).
- **Tier 0:** implemented, but the libav merge removed the FFmpeg-child CPU
  fields because there are no child processes any more.

## New performance items

Each item gives its location, the change, the expected effect and whether
frame bytes change.

- **Lens flare** (`effects.c:291` `flare_worker`)
  - Change: scan only the 4 ghosts' bounding boxes, not the whole frame.
  - Effect: about 98% fewer pixels visited at UHD.
  - Bytes: unchanged.
- **Deformer animation** (`compositor.c:237` `sr_deform_inverse`)
  - Change: evaluate modifier animation once per draw, not per pixel.
  - Effect: work drops from pixels × modifiers to modifiers.
  - Bytes: unchanged.
- **Animated particle emission** (`particles.c:155`)
  - Change: cache prefix sums; today it re-integrates the whole timeline
    every frame.
  - Effect: total work goes from O(frames²) to O(frames).
  - Bytes: unchanged if the summation stays sequential.
- **Path strokes** (`vector_path.c:235` `stroke_coverage`)
  - Change: bin segments by bounds; today every pixel is tested against every
    segment.
  - Effect: orders of magnitude on complex paths. This also closes a
    denial-of-service hole.
  - Bytes: unchanged.
- **Shadow maps** (`lighting.c` `map_build`)
  - Change: rasterize them instead of ray-casting every texel against every
    triangle ⟂.
  - Effect: about 5–20× on the map build.
  - Bytes: change; include in the tier-2 refresh.
- **Static-scene shadow maps**
  - Change: cache the map when lights, casters and camera are unchanged.
  - Effect: large on static scenes.
  - Bytes: unchanged; the risk is invalidation.
- **3D supersampling** (`lighting.c:593,653`)
  - Change: shade once per pixel per primitive and weight by the coverage
    mask; work in row bands. Today everything is shaded per sample (16× at
    N=4), and at 4K the buffers take about 3 GB.
  - Effect: about N²× less shading and memory.
  - Bytes: change.
- **Video source cache** (`video.c:465`, `:20`, `:488`)
  - Change: reuse buffers, cache decoded YUV `AVFrame`s instead of 133 MB
    float frames, and use real keyframe positions from `scan()` instead of a
    48-frame seek rule.
  - Effect: large on reversed or remapped video.
  - Bytes: unchanged.
- **Colour conversion** (`encoder.c:518`)
  - Change: fuse float → Y'CbCr into one row-parallel pass with fixed dither;
    today it is float → RGBA16 → single-threaded `sws_scale`.
  - Effect: medium.
  - Bytes: change.

## 3D renderer (lighting.c)

Beyond the bugs above, in suggested order:

1. **Cheap wins:** Blinn-Phong gating (skip specular and PCF when n·l ≤ 0),
   per-light constants hoisted, bounding-sphere culling against the view and
   light frusta, backface culling for closed meshes [FCG 1673, 3020–3031].
2. **Correctness:** perspective-correct interpolation plus near-plane
   clipping [FCG 2799–2824, 2862–2867]. Also snap vertices to fixed point
   with a top-left fill rule, which gives watertight meshes with no shared
   edges drawn twice [FCG 2751–2758].
3. **Shadows:**
   - Rasterize the maps [FCG 4209–4210].
   - Fit the directional frustum to casters ∩ visible receivers; map spot
     lights to their cone only [CGPP 12271].
   - Use bilinear-weighted 4×4 PCF centred on the sample point [FCG 4215–4218].
   - Replace the large bias (up to 21.5 texels) with a normal offset [FCG 4212–4214].
4. **Supersampling:** shade at 1× with N² coverage, in row bands, as above.
5. **Visibility buffer:** write (object, triangle) ids first, then shade each
   pixel once [FCG 2920; CGPP 12248].
6. **Threading:** parallelize the 3D pass and the shadow build by row bands.
   Each band rasterizes all objects in scene order, so output is
   byte-identical.

**To check:** the coordinate frames may be mixed between directional
lights (y up) and point/spot lights and normals (world, y down), and sprite
depth ignores scale (research report B5/B6). Add small golden scenes first.

## Physics (physics.c)

The integrator is symplectic Euler. Contacts are O(n²) × 3 passes with a
full positional snap. There is no rotational dynamics; soft bodies use
auto-substeps capped at 256.

1. **Sequential-impulse solver:** one contact list per step, 8–10
   iterations, clamped accumulated impulses, warm starting keyed by the
   sorted (i, j) pair, Baumgarte/split correction with slop, and restitution
   only above an approach-speed threshold [GK; a4b286db 658–663]. This fixes
   jitter and sinking stacks.
2. **Broadphase:** sweep-and-prune with a stable index tie-break, plus a
   node→index map in place of the linear `state_for` [a4b286db 3961–4024].
3. **Stability:** compute ω per constraint and substep to keep ωh < 2;
   report when the soft-body 256-substep cap is hit [FCG 7094–7098].
4. **Soft bodies:**
   - Apply damping once.
   - Add bend springs.
   - Hoist the per-step `malloc`.
   - Add grid-vs-rigid collision [CGPP 17132].
5. **Rotational dynamics:** inertia, contact points by face clipping, and
   torque [a4b286db 4058].
6. **Tunneling:** auto-substep when |v|·dt exceeds half the smallest extent.
7. **Cache signature:** add endianness and a libm probe; bump
   `PHYSICS_CACHE_VERSION` after any solver change.

Corpus coverage is thin here. Items marked [GK] are general knowledge.

## Robustness and testing

1. **An `SrLimits` struct enforced at parse time.** Caps would cover:
   - dimensions (~16384),
   - frame count,
   - XML depth (~256; today the stack is unbounded, and each isolated group
     level allocates a full-frame buffer),
   - path bytes and segments,
   - OBJ vertices and faces, with finite values only.

   This treats denial of service as a threat category [a2165db8 620].
2. **`sr_alloc_array(n, size)`** with checked multiply at every size site,
   including `effects.c:216,422`, `video.c:636` and the supersample bug above
   [2e83fe48 3154–3164].
3. **Fuzz harnesses** (libFuzzer or AFL) for the path parser, OBJ loader,
   XML loader and text layout. Loaders need `(buf, len)` entry points.
   - Seed from `tests/` and `examples/`.
   - Add an XML dictionary.
   - Run under ASan and UBSan with RSS and timeout limits.
   - Run nightly, plus a short smoke run per merge request.

   Sources: 891a79be 1968–1979; 49b31c3a 11128–11250.
4. **Sanitizer matrix:** add TSan (the renderer is threaded) and MSan builds
   of the unit suites, and use `-fno-sanitize-recover=all` in CI.
5. **Fault injection for `sr_alloc`** (fail after N), iterated over load and
   render under ASan. Today only libav faults are injected [220822c0 2323–2325].
6. **Metamorphic tests:**
   - thread-count invariance (1, 2, 4, 22) on *every* example, where today
     it covers three scenes;
   - identity groups change nothing;
   - integer translation of a path shifts its coverage.

   Commit fuzz crashers as regression inputs [39e3e917 1953; 95bff2e0 4276].
7. There is **no CI config** in the repo; add one that runs the above.

## Media and audio

- **x264:** pin its thread count to fix the determinism bug above. Also
  budget cores between the render pool and the encoder, since today they
  oversubscribe [Gregg 1575, 8754–8757].
- **Chroma flag:** `SWS_FULL_CHR_H_INT` is an output-side flag. For RGB
  input the relevant one is `SWS_FULL_CHR_H_INP`, and no dither is set.
  Verify this [GK].
- **Dither:** apply fixed, position-keyed dither before quantizing to 8-bit
  video and integer audio [514012eb 2239–2246; FCG 1280].
  - Offer `yuv420p10le` as the banding-free default [514012eb 2036].
- **Audio mixing:**
  - One deterministic look-ahead limiter; remove the second clip in the
    encoder [220822c0 9280; EBU R128].
  - Attenuate-only balance for stereo sources.
  - Cubic interpolation for varispeed, and dB-shaped fades [220822c0 1934, 3989–3993].
- **Wide-gamut and HDR sources:**
  - Decode above 8 bits (today every source goes through RGBA8).
  - Soft gamut compression instead of per-channel clamping [CGPP 9526–9529].
  - Tag Rec.2020 as HDR only with PQ or HLG.

## Rules for every change

- Frames must stay deterministic. Run the golden SHA tests at `--threads` 1,
  2, 7 and 22.
- Never add `-ffast-math`, `-Ofast` or `-funsafe-math-optimizations`. Keep
  `-ffp-contract=off` (see [C2], [C3]). Add a build check for this.
- Group byte-changing items (tier 2) into one golden-hash refresh.
- Known risk that already exists: glibc libm picks implementations by CPU at
  runtime. The same flags can therefore produce different bytes on different
  hosts [C1].

## Tier 0 — measure first (S–M effort) — implemented

Done: stage timers and `--metrics-trace`, CPU for the engine and children,
`make profile`, and `make perf-check`. See [benchmark.md](benchmark.md#measurement-tools).

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

Revision 2 order:

1. **Safety:** the two high bugs (supersample overflow, NaN OBJ geometry),
   `SrLimits`, and `sr_alloc_array`. These are small and close real memory
   holes.
2. **Determinism:** pin x264 threads, and give the CPU and GPU colour paths
   the same quantization. Add thread-count invariance tests over every
   example.
3. **Byte-identical speed:** Tier 1 #2 (scratch arena), #3 (blur), #1
   (thread pool), the flare bounding box, per-draw modifier evaluation,
   particle prefix sums, and #7 (encoder overlap, now worth 20–25% of wall).
   Refresh the `perf-check` baseline first.
4. **Correctness with byte changes, in one golden refresh:**
   - 3D perspective interpolation, clipping, specular gating, fill rule;
   - soft-body damping;
   - reverse `clipOut`;
   - Tier 2 #9–#11;
   - rasterized shadow maps;
   - fused Y'CbCr conversion with dither.
5. **Architecture:** tile-parallel compositing (Tier 3 #12), 3D row bands,
   and a sequential-impulse solver with a broadphase.
6. **Continuous:** fuzzing, the sanitizer matrix and CI; then the quality
   and media tracks.

Re-profile with `make profile` and `make perf-check` after each step.

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

- **R2** — revision 2 sources:
  - [FCG] *Fundamentals of Computer Graphics 5e* (`363e9ddb`): the 3D items above
  - [CGPP] *Computer Graphics: Principles and Practice 2e* (`8bea5cf5`)
  - `a4b286db` *Game Programming in C++* (Madhav): fixed steps, SAT, sweep-and-prune, CCD
  - `a2165db8` Threat modeling (EoP)
  - `2e83fe48` checked arithmetic
  - `891a79be` SDL and fuzzing
  - `39e3e917` metamorphic testing
  - `220822c0` *The Audio Programming Book*: pan law, clipping, interpolation, malloc fault injection
  - `514012eb` Gersho, *Vector Quantization*: dither, 6 dB per bit
  - Codex audit transcript kept outside the repo
  - [GK] = general knowledge, not found in the corpus

**Corpus caveat:** in doc-ray, the bodies of *Modern C* (`3d57d415`) and
*Optimized C++* (`c9022c82`) hold text from other books. Only their tables of
contents are usable.
