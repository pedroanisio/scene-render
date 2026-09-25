# Incremental delivery report

All measurements below were produced in the supplied Ubuntu 24.04 x86-64
container with GCC 13.3.0, Expat 2.6.1, and FFmpeg 6.1.1. Small phase artifacts
are committed under `artifacts/`; SHA-256 values are in `artifacts/SHA256SUMS`.

## Phase 1 — XML, timeline, image layers, 4K output

Architecture: Expat callbacks build an owned scene graph; the absolute-frame
timeline evaluates six curve types; a scalar CPU compositor performs nested
transforms, rectangular masks, alpha, six blends, and optional linear-light
math; raw frames stream to FFmpeg.

Verified:

- strict C17 build and semantic XML diagnostics;
- image source sharing, stable z-order, groups, masks, and 10 simultaneous
  layers;
- H.264/H.265/FFV1 output and a full 300-frame 3840×2160 render;
- deterministic golden frame.

Artifacts: `phase1-verification.mp4`, `phase1-4k-frame.png`, and
`phase1-10s-4k.mp4`. The measured 10-second result is detailed in
`benchmark.md`.

Boundary at delivery: image-only timeline; later phases retained the same
scene/compositor contracts.

## Phase 2 — video, audio, time manipulation, resume

Architecture: video frames are indexed by declared rational FPS and decoded
lazily into a four-frame per-source LRU. Audio decodes to temporary float disk
streams and mixes in 4096-frame blocks. A content signature namespaces atomic
raw-frame resume entries.

Verified:

- three independently timed instances share one video asset;
- trim, finite loop, reverse, speed/stretch, and explicit source-time remap;
- stereo volume/pan mix and synchronized H.264/AAC streams, both exactly
  1.000 seconds in the verification file;
- resume rerun reuses all selected cached frames.

Artifact: `phase2-media-audio.mp4` (160×90, 12 frames, H.264 + AAC). Measured
wall time: 1.793 s; peak self/child RSS: 47,388 KiB.

Boundary: each LRU miss starts a supervised FFmpeg decode process, favoring a
small C dependency surface over high-throughput decode. Widely divergent
remaps can thrash the four-frame cache.

## Phase 3 — 360° and camera

Architecture: the panorama remains equirectangular; perspective rays apply
animated roll, pitch, and yaw, then bilinearly sample longitude with seam wrap
and clamped latitude. POSIX workers render disjoint row ranges.

Verified:

- FFV1 equirectangular output;
- animated viewport orientation and FOV;
- byte-identical viewport golden output at one and four threads;
- standard perspective/orthographic projection for built-in 3D objects.

Artifacts: `phase3-equirect.mkv`, `phase3-viewport.png`, and its lossless PPM.
The viewport preview measured 0.0237 s; six equirectangular frames measured
0.1181 s.

Boundary: mono 2:1 equirectangular scenes only. Translation is intentionally
irrelevant for an infinitely distant panorama.

## Phase 4 — lighting, particles, post-processing

Architecture: a CPU primitive renderer shades material colors with ambient,
directional, point, and spot lights. The compositor generates particles from
stateless integer hashes. Effects run in declared order after projection.

Verified:

- four animated-capable light families with range/falloff/spot parameters;
- deterministic smoke, sparks, dust, and rain presets;
- glow, bloom, blur, color grade, vignette, and lens-flare-style effects;
- material-controlled spheres, boxes, and planes plus screen-space shadows.

Artifact: `phase4-lighting-particles.png` and lossless PPM. The 320×180 preview
measured 0.0087 s.

Boundary: 3D primitives, shadows, and flare are visual approximations, not a
general mesh/PBR renderer.

## Phase 5 — physics and deformation

Architecture: dynamic bodies are simulated at a fixed XML timestep, cached as
absolute poses, and interpolated at frame time. Deformation uses deterministic
inverse warps during sampling.

Verified:

- gravity, radial/directional fields, damping/drag, springs, constraints,
  circle/AABB collision, friction, and restitution;
- physics cache save and byte-identical reload;
- bend, twist, wave, squash, and stretch;
- damped soft-body-style surface motion.

Artifacts: `phase5-physics-deformation.png` and lossless PPM, plus the complete
`v1.0-feature-showcase-4k.mp4` render and its 5-second inspection frame. The
showcase contains 300 H.264 frames at 3840×2160 and 30 fps. It measured
1086.223 s rendering, 20.146 s encoding, 1106.377 s wall time, 0.271 fps, and
2,591,484 KiB peak RSS on the verification host. The cached 320×180 preview
measured 0.0097 s.

Boundary: the solver and soft body are visual simulations. They are explicitly
not claimed to be physically accurate.

## Phase 6 — optimization, regression, documentation

Architecture: deterministic viewport row partitioning, media/GPU capability
fallbacks, atomic resume files, and phase-specific regression scenes were added
without changing frame order or timeline math.

Verified:

- clean strict rebuild;
- AddressSanitizer and UndefinedBehaviorSanitizer unit pass (LeakSanitizer is
  unavailable under the container's ptrace supervisor);
- unit suite plus end-to-end XSD, media, A/V, 360, physics, effect, resume,
  codec, and golden-frame tests;
- exact frame equality across thread counts and repeated executions;
- wall/render/encode time, FPS, and peak self/child RSS reporting;
- complete XSD, examples, Apache-2.0 license, dependency licenses,
  troubleshooting, and benchmark documentation.

The full clean `make test` run completed successfully in 7.6 seconds in the
verification container. The remaining boundaries are product scope items in
`feature-matrix.md`, chiefly GPU acceleration, arbitrary 3D meshes, complex
vector paths/text shaping, and a high-throughput in-process libav decoder.
