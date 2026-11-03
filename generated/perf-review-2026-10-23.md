---
title: Performance review — CPU, GPU and memory
date: 2026-10-23
commit: 74a2fae
generated_by: Claude Fable 5.1 via Claude Code (four read-only crate reviewers, findings spot-checked against the code; measurements from scene-render's own --bench/--stats)
---

# Performance review: where scene-render spends CPU, GPU and memory

Scope: the workspace at 74a2fae, measured on a 12-thread host with an RTX 3050 (NVIDIA driver, Vulkan) and lavapipe
(Mesa 25.0.7). Every finding below was read from the code at the cited lines; the impact ranking leans on the
measurements first and on code reading where nothing was measured. `perf` is not installed on this host and
`perf_event_paranoid=4` blocks sampling, so the CPU split comes from the renderer's own `--bench` breakdown and
`--stats` counters.

## Measurements

The five documented perf scenarios (`tools/perf_*.py`) plus the 02-fracture showcase, `render --bench --stats`,
120 frames on the GPU (24 on lavapipe, 12 for fracture; physics from frame 60). "serial" runs without `--pipelined`
so GPU timestamps are reported; "pipelined" is the delivery mode. Times are medians per frame; RSS is the peak of the
process.

| Scenario | GPU serial: frame / GPU work / submit / eval | GPU pipelined frame | GPU RSS | lavapipe frame | lavapipe RSS |
|---|---|---|---|---|---|
| layers (200 blended image layers) | 14.3 / 12.7 / 1.5 / 0.1 ms | 12.4 ms | 545 MB | 384 ms | 355 MB |
| text (50 animated text layers + karaoke captions) | 2.6 / 0.4 / 2.1 / 0.0 ms | 2.1 ms | 534 MB | 6.4 ms | 244 MB |
| effects (8 effects, crossfade, 16-sample motion blur) | 3.4 / 2.8 / 0.5 / 0.0 ms | 2.6 ms | 527 MB | 82 ms | 894 MB |
| 3d (208k triangles, 4 shadowed spots, DoF) | 4.2 / 3.1 / 0.8 / 0.0 ms | 2.9 ms | 536 MB | 591 ms | 517 MB |
| physics (200 bodies, jelly, cloth, 12k particles) | 2.7 / 0.9 / 1.7 / **4.8** ms | 1.8 ms | 540 MB | 49 ms | 298 MB |
| 02-fracture (path traced, 8 spp, debris) | 49.3 / 33.8 / 7.0 / 1.5 ms (p95 102) | 34.5 ms (p95 90) | 816 MB | 202 ms (frames 0–12) | 1201 MB |

Per-frame `--stats` on the GPU runs, averaged over the run:

- physics: `sim_rigid` 3.5 ms + `sim_particles` 6.5 ms per frame — the simulation is the whole frame; the GPU draws
  it in 0.9 ms.
- fracture: `pt_pack` 7.7 ms + `pt_bvh` 1.5 ms (12.4 + 4.3 ms at frame 80) + `pt_assemble` 2.0 ms, `sim` 3.5 ms,
  `vector` 2.5 ms per frame of CPU work before the GPU traces anything.
- layers: 201 draws and **174 backdrop copies** per frame; the GPU time (12.7 ms) is the copies, not the draws.
- text: `vector_seconds` 3.9 ms per frame of CPU rasterisation — the frame is CPU-bound.
- Texture churn is nil: `textures_created` is 0 per frame on every GPU run (2 on effects' sub-frame steps); the pools
  do their job.

Memory floor: an empty composition is 387 MB on the NVIDIA GPU and 229 MB on lavapipe; `--version` and `eval` are
10 MB. The floor is the Vulkan device (loader, driver, llvmpipe's LLVM JIT, pipelines), not our heap. What we own is
the difference between a scene and that floor: fracture +430 MB on the GPU and +970 MB on lavapipe, effects +670 MB on
lavapipe. On a software adapter every GPU buffer is host memory, so per-frame GPU allocations show up as RSS.

sr-eval's own scenario (`crates/sr-eval/tests/perf.rs`, 1,020 nodes, 5,500 slots, 500 expressions) ran at a frame
median of 1.383 ms, but under a load average of 15–20 from concurrent builds; the README records 0.97 ms. The low-load
re-run is in the addendum below.

## Ranked opportunities

Ranked by measured impact where there is a measurement, then by code evidence. Sizes are complexity classes.

### 1. Path tracer: per-frame CPU rebuild of the scene it traces — M/L

Measured: 10–19 ms of CPU per fracture frame on the GPU before tracing starts, and the main reason the post-impact
frames take 29–49 ms with only 34 ms of GPU work.

- `pathtrace/cache.rs:26` keys the geometry cache on every draw's model matrix bits. `pathtrace.rs:469-491` expands
  any mesh drawn once (`repetitions[k] == 1`) into world-space triangles, and `:543` rebuilds the whole top-level BVH.
  After the impact every debris piece is unique and moving, so the cache misses every frame and the whole scene is
  re-expanded and re-BVH'd.
- `pathtrace.rs:2452-2466`: `accum`, `guide`, `ping`, `pong` are fresh zeroed buffers of `pixels × 32` bytes every
  render (≈118 MB per 720p frame allocated and freed; on lavapipe that is host RSS), plus new `mats`/`lights` storage
  buffers from `data.mats.clone()`/`data.lights.clone()`, a per-tile uniform buffer and per-tile/per-denoise-pass
  bind groups. `atlas_key()` (`:177-183`) clones every atlas word list per frame to build a key.
- `pathtrace.rs:2537` → `pathtrace/rays.rs:80-190`: with hardware ray queries, `rays::build` re-expands the geometry,
  creates new input buffers and new BLAS/TLAS and calls `build_acceleration_structures` every frame, even when the
  packed buffers came from the cache.
- `pathtrace/cpu.rs:184-196` (software adapters): `Dense::new` rebuilds the SAH BVH and `atlas_pixels()` per pass, and
  `EnvMap` clones mip0 (`e.2.clone()`, ~32 MB for a 2K map) per pass; each tile uploads through a fresh
  `create_buffer_init` staging buffer and a copy instead of `queue.write_buffer`.

Change: treat every cached rigid mesh whose transform changes as an instance of a cached prototype (the prototype path
already exists for `repetitions > 1`), and rebuild only the top level over instance bounds; keep grow-only scratch
buffers and the denoise bind-group pair in `PtGpu`; cache `Rays` and `Dense` by geometry ticket; hold the env map by
`Arc`. Measure: `pt_pack_seconds`, `pt_bvh_seconds`, `timing.geometry_reused` and frame median on fracture frames
60–120, GPU and lavapipe; RSS on lavapipe.

### 2. Simulation: collisions and sub-steps do whole-scene work — S to L

Measured: physics scenario 10 ms/frame of simulation on a 1.8 ms frame; fracture 3.5 ms/frame.

- `sr-sim/src/physics.rs:216-230` (`surface`) tests every collider with `project_point`, no AABB pre-check and no
  spatial index; called from `particles.rs:291` for every live colliding particle every step, and from the soft-body
  `collide` (`:629-632`). perf_physics: ~1,200 live sparks × ~205 colliders ≈ 250k projections per step. S/M.
- `sr-eval/src/sim.rs:573-590` (`PDriver::enabled/kinematic/fields`) and `sim3d.rs:1429-1436`: every sub-step calls
  `graphs.at(t)`, which on a miss runs a full `eval::evaluate` of the whole document just to read a few bodies'
  transforms (`sim.rs:258-283`, an 8-entry cache keyed by exact time). `Evaluator::evaluate` (`lib.rs:133-139`) never
  puts the frame's own base graph into that cache, so it can be computed twice. L for a pose-only evaluation; XS to
  seed the cache.
- `sr-eval/src/sim.rs:285` `index_of` is a linear scan by id string, used per body per step (`apply_bodies`
  `:2080-2091`, `sim3d.rs:1635-1704`, twice in the emitter state `:1729,1734`), and `resolve_in` (`eval.rs:1594-1611`)
  formats a key per scope level then scans, inside loops in `sim3d.rs:136`, `render_three.rs:3212`, `vector.rs:668`,
  `render_fx.rs:1965`. One lazily built id→index map per FrameGraph removes the O(bodies × nodes). S.
- `sr-sim/src/physics3d.rs:1582-1611`: `driver.surface()` is asked for every enabled body every step, including
  dynamic debris for which `Some` is an error anyway; `sync_visibility`/`apply_fractures` run twice per step. XS.
- `sr-sim/src/pyro.rs:1461` `working_copy` copies 5 full grids, then `advect` (`:1674-1679`) takes them and allocates
  fresh ones, so the copies are wasted; `project` (`:1917-1989`) allocates ~10 more full-grid f64 vectors per step;
  obstacle voxelisation (`:705-739`) is serial and redone every step. M.
- `particles3d.rs:531` reserves `max_particles` (~184 B each) on every step and rebuilds a death heap over all live
  particles. S.

Measure: `sim_*_seconds` in `--stats` on perf_physics and fracture; perf_physics with sparks `collide` on vs off.

### 3. 3D raster and vector raster: resources rebuilt per render — M

Measured: text scenario CPU-bound at 2.1 ms submit (`vector_seconds` 3.9 ms/frame); 3D at 0.8 ms submit for 18 draws.

- `sr-gpu/src/three.rs:2372-2383, 2548-2558`: `frame_buf`, `light_buf`, `tile_buf`, `smat_buf`, `obj_buf`,
  `mat_buf`, `index_buf` are new `create_buffer_init`s every render; because `mat_buf` is new, `self.mat_binds.clear()`
  discards every material bind group and `mat_bind` recreates them. The comment explains why (several 3D passes may be
  recorded before one submit, so `write_buffer` into a shared buffer would be wrong); a ring of grow-only buffer sets,
  one slot per 3D pass between submits, keeps that property and the bind groups.
- `three.rs:2495` skinned/deformed meshes upload with `create_buffer_init` per draw per frame; `:3050-3060` every splat
  re-sort allocates four n-sized buffers; `:2007` the IES atlas is re-created whenever any light has IES;
  `particles.rs:241` one instance buffer per batch per frame; `render_three.rs:3502` under f32 working precision each
  3D layer creates an unpooled f16 texture per frame because `Pool::put` (`resources.rs:195`) rejects other formats.
- `sr-gpu/src/raster.rs:306-319, 462-489`: each vector/text job creates one uniform and 4–5 storage buffers plus a bind
  group; `storage()` does `bytes.to_vec()` and then `create_buffer_init`, copying the data twice. Appending a frame's
  jobs into frame-level grow-only buffers with offsets (as `Renderer::upload`/`FrameBufs` already does for the
  compositor) removes both.
- `sr-3d/src/anim.rs:191-272`: skinning and morphs clone every vertex (96 B each) and blend serially on the CPU per
  frame; GPU skinning (joint palette + existing `joints`/`weights`) is the real fix, rayon + buffer reuse the interim.

Measure: `submit_ms` and `vector_seconds` on perf_text; `ThreeEngine::render` CPU time and `create_bind_group` count on
a many-material scene; frame time on a skinned glTF.

### 4. Delivery: things that serialise CPU, GPU and encoder — S to M

Not measured here (they are off in the perf scenarios); verified in the code.

- `sr-deliver/src/pipeline.rs:498-500` → `sr-gpu/src/render_access.rs:507,587`: with `<accessibility/>` declared,
  the flash check submits a reduction and blocks on `read` **every frame**, before `submit_placed`, so frame N's GPU
  work must finish before the CPU continues: CPU and GPU take turns. It also creates a 48×27 texture and a staging
  buffer per frame. Fix: `begin_map_read` kept beside the frame's `Pending`, collected one frame later; or reduce
  inside the pack pass. M (S for the async half).
- `sr-deliver/src/overlay.rs:133-214`: the overlay contrast probe re-renders the overlay 1+N (up to 1+3N) times per
  frame with full-frame blocking reads (~33 MB f32 at 1080p, decoded on the CPU), with no budget, while composition
  text has `CONTRAST_BUDGET`. M.
- `pipeline.rs:350-353` / `render.rs:5609`: shutter sub-frames are evaluated synchronously on the render thread (up to
  256 at `final`), and `subframe_seconds` is measured but never reaches the `Report`, so the cost is invisible.
  Surface it, then evaluate shutter times in parallel on the helper thread. M.
- `pipeline.rs:232-252`, `sr-media/src/encode.rs:686-696,1403-1410`: ProRes, DNxHR, alpha and any pixel format other
  than yuv420p/p010 read back `Rgba16` and let FFmpeg's `scale` convert in software; x265/SVT-AV1/VP9 get NV12 but
  `encoder_pix_fmt` passes `nv12` through only for libx264/NVENC, so swscale converts nv12→yuv420p. GPU packs for
  I420, yuv422p10le, yuv444p(10), yuva444p10 halve readback and pipe bytes. M.
- `pipeline.rs:2476-2530`: two-pass and `maxFileSize` relay every frame through two FFmpeg processes and Rust per
  pass; the pass encoder can read the FFV1 intermediate directly. S/M.
- `sr-media/src/decode.rs:218-219`: video sources decode at full resolution regardless of displayed size. M.
- `scene-render/src/main.rs:1645-1709`: `render` to a PNG sequence does a blocking f32 readback, per-pixel f64 sRGB
  conversion on one thread, and PNG encode on the render thread; the encode path's `OutputStage` already does the
  conversion on the GPU. S/M.

Measure: an encode of perf_layers with and without `<accessibility/>` (addendum); `stage_seconds` and "GPU/readback
blocked" in `encode --json`; ffmpeg CPU% on a ProRes output.

### 5. Evaluation: per-frame allocation and whole-document walks — S each

Measured: 0.97–1.4 ms per frame on perf.rs; small next to the items above, but it runs for every frame of every film
and for every shutter sample.

- `sr-eval/src/eval.rs:1164-1166` builds a `Props` Vec per node (~1,020) with an `Arc<str>` clone per slot (5,500) per
  frame; `Out { nodes: Vec::new() }` (`:1244`) has no `with_capacity`.
- `rig.rs:698-711` walks every node's children through dynamic `visit` every frame to find constraints that most nodes
  do not have, and formats a key per constraint; precompute the constrained set at build time.
- `eval.rs:1219-1230, 1448`: fresh scratch Vecs and a memo HashMap every frame.
- `eval.rs:846-884`: keyframe channels of off-screen nodes are still evaluated (only expressions use `idle`).
- `vm.rs:1294-1311` wiggle allocates three Vecs per call even for scalars; `property_channel` CRC-hashes the property
  name on every call for non-built-in properties.
- `eval.rs:756-771` → `geo.rs:73-95`: `geo()` in an expression stats files and builds strings per call per frame.

Measure: perf.rs median; `eval FILE --bench`.

### 6. Compositor: backdrop copies and hashing — M, design-limited

Measured: layers 12.7 ms of GPU work for 201 draws, 174 backdrop copies.

- Only add and linear-dodge use the fixed-function blender; every other blend mode copies the backdrop under the draw's
  bounds (README, effects section). With 200 layers cycling through 33 modes that is a copy per draw. wgpu has no
  programmable blending; the options are grouping consecutive same-mode draws into one copy, or a per-tile approach.
  L; measure `backdrop_copies` and GPU ms on perf_layers.
- `sr-gpu/src/render.rs:2626-2698, 2744-2876`: change-detection hashing re-runs `subtree_hash` from every root and
  again per isolated group/matte/effect source; `node_hash` calls `glyph_depth` twice (`format!` + children scan each)
  and collects all particle positions into a fresh `Vec<u64>` per call; `element_state` scans `ctx.el` linearly per
  id. Memoise per-node hashes per frame. M.
- `render.rs:5066-5078, 5326-5327`, `fx.rs:842-916`: the `pairs` bind-group map and `dummy_pair` are rebuilt every
  frame; coverage draws create a bind group per draw; each `fx.record` makes its own params buffer. S.
- `fx.rs:776-814`, `shader.rs:143-162`: looped (`iterate`) shaders create a uniform buffer and bind group per step
  and check convergence with a full synchronous readback per persistent texture; a GPU max-abs-diff reduction reads
  back 4 bytes. M.
- `video.rs:371-379, 458-500`: each decoded video frame gets a fresh full-resolution texture, uniform, bind group and
  its own `queue.submit`; evicted frames are dropped instead of returned to the pool; `flow()` creates 18 textures per
  call. S/M.

### 7. Memory: what grows and what is duplicated — XS to M

- Unbounded: 2D world checkpoints (`sr-sim/src/physics.rs:649-651`) and 2D emitter checkpoints
  (`particles.rs:318-320`) insert a full state clone every simulated second and never thin, unlike `timeline.rs:67-74`,
  `World3::save_checkpoint` and particles3d — memory grows linearly with duration (~1 MB/s for 12k particles plus the
  rigid state). S. `ThreeEngine.meshes`/`.textures` (`three.rs:803-808`) have no general eviction; the process-global
  geo caches (`sr-eval/src/geo.rs:33,73,182,226`) never evict and key on file mtime, so a long preview session keeps
  every stale entry; `voxel_cut.rs:403` grows per revision. XS–S each.
- Over-charged: `physics3d.rs:1460-1491` charges each shared `Arc` collider shape in full per checkpoint and walks
  `voxels().count()` per checkpoint, so the 256 MB budget fills early on decomposed debris and checkpoint spacing
  doubles — longer replays on seeks and shutter samples. S.
- Duplicated: `sr-3d/src/sequence.rs:262-358` clones the whole model including texture bytes per blended frame and
  re-validates immutable `Arc` models per sample; audio decode (`decode.rs:410-424`, `sr-audio/src/mix.rs:32-35`)
  holds three copies of a decoded file at peak; `render_three.rs:1155` clears the entire volume cache on budget
  overflow. S–M.
- Trade-offs to keep: `MeshGpu.cpu` keeps a host copy of every mesh for the path tracer's BVH; checkpoint-and-replay
  determinism; the extra fractional `advance` in particles3d.

### 8. Threads on lavapipe — S, pending measurement

`scene-render/src/main.rs:2532-2537`, `sr-vector/src/lib.rs:41`, `sr-media/src/decode.rs:212-219`,
`encode.rs:1146`, `scene-render/src/arena.rs`: lavapipe's pool (all cores), the sr-vector pool (cores, ≤16), rayon's
global pool, libx264 at 8 threads and every decoder's FFmpeg run at once, with all Rust allocations in one glibc
arena. The sweep of `LP_NUM_THREADS` and `MALLOC_ARENA_MAX` on fracture frames 80–86 is in the addendum.

## Already well optimised (do not re-investigate)

Output readback (GPU NV12/P010 packing, 3-slot staging ring, non-blocking map, waits only on its own submission);
grow-only compositor frame buffers; texture and target pools with trimming (confirmed by `textures_created` = 0);
shadow maps kept by key, tiled light culling, light-signature specialisation; splat sort cache under an LRU budget;
path-tracer geometry and packed storage cached by ticket (for static or instanced meshes); mask coverage cache
bounded; lazily compiled pipelines; mid-frame submits only past 512 MiB of transients; picture caches evicted after
`CACHE_KEEP`; expressions compiled to register bytecode with `prop()`/curve/marker references resolved at compile
time; binary-search keyframes; `Arc` payloads in FrameNode; emitters sampled on the frame grid; World3 and particles3d
checkpoint budgets with thinning; Rapier's BVH broad phase; rayon throughout pyro with a multigrid-preconditioned CG;
byte-bounded encoder queue on its own thread; decoders recycling plane buffers; cheap progress reporting.

## Suggested order

1. Path tracer per-frame rebuild (item 1): the largest measured cost, on both adapters, and it compounds with the
   02-fracture 3× work already done. Govern lane, three passes: instance cache for moving rigid meshes; persistent
   scratch and bind groups; `Rays`/`Dense`/env-map caching.
2. Simulation (item 2): AABB pre-check + per-step collider index; id→index map per graph; seed `Graphs` with the base
   graph; pyro ping-pong buffers. Measured on perf_physics and fracture.
3. Raster resources (item 3): 3D buffer ring + persistent material binds; frame-level vector job buffers.
4. Delivery stalls (item 4): async flash grid; `subframe_seconds` in the report, then parallel shutter samples;
   GPU packs for the remaining pixel formats.
5. Memory bounds (item 7): thin 2D checkpoints; charge shared shapes once; evict `ThreeEngine` assets and geo caches.
6. Evaluation allocations (item 5) as a single short pass guarded by perf.rs.

Each item names the stat or scenario that proves it; a pass that cannot move its number is reverted, as with the two
path-tracer experiments dropped during the 02-fracture work.

## Addendum: measurements taken after the ranking

- **perf.rs at low load (load average 3.8):** frame median 0.837 / 0.837 / 0.841 ms over three runs, p95 0.89–0.90 ms,
  compile 47–48 ms. Better than the README's recorded 0.97 ms; item 5 stays last in the order.
- **Threads and arenas on lavapipe (item 8): no change needed.** Fracture frames 80–86, serial bench, median per
  frame: default 1960 ms; `LP_NUM_THREADS` 6 / 8 / 12: 2075 / 2039 / 2030 ms; `MALLOC_ARENA_MAX` 2 / 8: 2021 /
  2036 ms — all within noise of each other. Layers (raster-bound, 24 frames): default 364 ms; `LP_NUM_THREADS` 6 / 8:
  430 / 418 ms; `MALLOC_ARENA_MAX=8`: 383 ms. The current defaults (lavapipe on all cores, one arena) are the best of
  the sweep on both workloads; item 8 is closed.
- **Fracture memory on lavapipe, frames 80–83, serial:** CPU tracer on: 980 MB peak RSS, 1928 ms per frame; CPU
  tracer off (`SR_PT_CPU=0`): 914 MB, 4604 ms. The CPU tracer's own copies (`Dense`, the env-map clone) cost 66 MB;
  the rest of the +685 MB over the 229 MB floor is shared by both paths — the per-frame path-tracer scratch of item 1
  and the scene's assets. The 1201 MB seen earlier was the pipelined run (two frames in flight).
- **Flash check stall (item 4, first entry), measured:** `encode` of perf_layers, 120 frames at 1080p60 on the GPU,
  two runs each. `<accessibility/>` declared: 2.56 / 2.60 s, 47 frames/s. Without it: 2.19 / 2.18 s, 56 frames/s —
  17 % slower. The stage split shows the mechanism: "GPU/readback" drops from 1.32 s to 0.06 s and "render" rises from
  0.35 s to 2.0 s, because the blocking `flash_grid` read now sits inside the render stage and the GPU no longer
  overlaps the CPU. On a document with heavier GPU frames the loss scales with the GPU time per frame.
