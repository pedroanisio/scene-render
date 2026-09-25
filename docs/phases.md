# Incremental delivery report

All measurements were produced in the supplied Ubuntu 24.04 x86-64 container
with GCC 13.3.0, Expat 2.6.1, and FFmpeg 6.1.1. Verification artifacts live in
`artifacts/`; published hashes are in `artifacts/SHA256SUMS`.

## Phase 1 — XML, timeline, image layers, and 4K output

Architecture and dependencies: Expat callbacks build an owned scene graph;
the master timeline evaluates directly from an integer frame number; a C
compositor applies stable z-order, hierarchy, transforms, alpha, masks, and six
blend equations. One RGBA frame at a time is piped to supervised FFmpeg.

Implemented and verified:

- strict C17 build, formal XSD, semantic/reference checks, and contextual
  line/element/attribute diagnostics;
- dynamically sized layer trees, shared image records, nested transforms,
  shaped and inverted masks, and linear-light compositing;
- all six interpolation curves and all required blend modes;
- H.264/H.265/FFV1 output and 300-frame 3840×2160 rendering;
- exact golden output and a ten-simultaneous-layer scene.

Artifacts: `phase1-verification.mp4`, `phase1-4k-frame.png`, and
`phase1-10s-4k.mp4`. The lighter historical 10-layer baseline measured
135.138 s wall time and 2.220 FPS. Layer count is memory-limited; the CPU
compositor is deterministic, not real-time.

## Phase 2 — video, audio, time manipulation, and resume

Architecture and dependencies: video frames are indexed by declared rational
FPS and decoded lazily through FFmpeg into a four-frame shared LRU. Audio
decodes to temporary float streams and is mixed by C in 4096-frame blocks.
Atomic post-color RGBA files provide content-signature-based resume.

Implemented and verified:

- multiple independently timed instances of one video asset without duplicate
  decoded-media ownership;
- trim, finite loop, reverse, speed/stretch, and explicit source-time remap;
- mono/stereo volume and pan mixing, exact selected-range sample count, AAC
  encode, and 10.000000 s video/audio duration equality in the final render;
- interrupted-render reuse with versioned manifests and atomic frame writes;
- encoder availability preflight and nonzero failures for unavailable codecs.

Artifact: `phase2-media-audio.mp4`. Its 160×90 verification run contains H.264
and AAC streams. A cache miss starts a supervised FFmpeg decode process; this
keeps the C dependency surface small but divergent remaps can thrash the
four-frame LRU.

## Phase 3 — 360°, camera motion, and viewport extraction

Architecture and dependencies: a perspective ray is rotated by animated roll,
pitch, and yaw, then bilinearly samples a wrapped equirectangular canvas.
Deterministic workers own disjoint row intervals. A C MP4 parser can atomically
inject the Google Spatial Media v1 UUID/XML after successful muxing.

Implemented and verified:

- configurable mono 2:1 equirectangular output, including 3840×1920;
- animated perspective viewport extraction at 3840×2160;
- byte-identical viewport frames at one and four threads;
- standard perspective/orthographic 3D camera projection;
- MP4 spherical UUID metadata, Matroska projection tags, and tagged color
  metadata checked by integration tests.

Artifacts: `phase3-equirect.mkv`, `phase3-viewport.png`, and its lossless PPM.
The supplied model is mono equirectangular. Translation intentionally has no
effect for an infinitely distant panorama; stereo/cubemap formats are outside
the declared schema.

## Phase 4 — lighting, particles, and post-processing

Architecture and dependencies: deterministic CPU rasterization shades built-in
primitives and Wavefront OBJ triangles against animated ambient, directional,
point, and spot lights. Stateless seeded particle hashes avoid mutable random
streams. Post effects use deterministic row jobs and sliding-window blur.

Implemented and verified:

- material-controlled spheres, boxes, planes, and triangulated OBJ meshes with
  generated normals and a shared depth buffer;
- four light families, range/falloff/spot controls, animated parameters, and
  screen-space/bounding-volume shadow approximations;
- deterministic smoke, sparks, dust, and rain with animated emitter controls;
- glow, bloom, blur, grade, vignette, and lens-flare-style effects;
- shaped UTF-8/bidirectional text through FFmpeg's mature text stack and
  antialiased filled M/L/H/V/C/Q/Z vector paths.

Artifacts: `phase4-lighting-particles.png` and lossless PPM; the v1.1
inspection frame additionally demonstrates the OBJ, shaped text, and path
pipeline. Lighting, metallic/roughness response, shadows, and flare are visual
models, not PBR, ray tracing, or optical simulation.

## Phase 5 — physics, constraints, and deformation

Architecture and dependencies: bodies simulate once at an XML fixed timestep
independent of output FPS. Render poses interpolate adjacent samples. A
version/signature-checked binary cache includes geometry, forces, constraints,
seed, timestep, duration, and engine version and is committed by atomic rename.
Deformation is an inverse sample warp.

Implemented and verified:

- gravity, directional/radial force fields, damping/drag, springs/distance
  constraints, friction, restitution, and circle/AABB collision behavior;
- cache save/reload with byte-identical rendered output;
- bend, twist, wave, squash, and stretch modifiers;
- damped soft-body-style surface displacement;
- collision, gravity, a spring, and four modifiers in the 10-second showcase.

Artifacts: `phase5-physics-deformation.png` and lossless PPM. Rigid-body and
soft-body behavior is explicitly a deterministic visual approximation and has
not been certified as physically accurate. Mixed circle/box collision uses
bounding circles.

## Phase 6 — optimization, regression, documentation, and benchmark

Architecture and dependencies: named sRGB, Rec.709, Display-P3, and Rec.2020
transfers/matrices separate source, working, and output color. The optional
OpenCL 1.2 module is dynamically loaded through a stable C boundary and
offloads final color conversion; CPU rasterization remains the reference.
Parallel allocation/device failures fall back to deterministic CPU execution.

Implemented and verified:

- strict Make build plus unit/integration suite passed;
- fresh CMake 3.30.5 Release build and CTest: 2/2 tests passed (7.69 s);
- AddressSanitizer + UndefinedBehaviorSanitizer unit suite and the production
  feature preview passed; LeakSanitizer was disabled under the supervisor;
- XSD validation for every example, dynamic 160-level nesting, precise XML
  failure text, goldens, one/four-thread equality, GPU fallback, physics cache,
  resume, A/V sync, H.264/H.265/FFV1, Rec.709/sRGB tags, and spherical metadata;
- all C implementation files remain below 500 lines;
- exact dependency/license inventory, architecture/XML references,
  troubleshooting, feature matrix, benchmark, Apache-2.0 license, and examples.

The final `v1.1-feature-showcase-4k.mp4` is 3840×2160, 30 FPS, 300 frames, and
10.000000 s with 10.000000 s stereo AAC. It measured 793.400 s rendering,
12.555 s encoder writes/final wait, 806.785 s wall time, 0.372 FPS, 104,048 KiB
engine peak RSS, and a conservative 2,264,136 KiB self-plus-child peak. A full
decode reported no errors. Its SHA-256 is
`f3723d3ca936cdfefe3e47d1bf99182d9a46c28a5dfbfe6c43560f6a714f7807`.

The verification host exposed an OpenCL loader but no usable GPU device, so
the real-device kernel path could not be benchmarked there; capability
detection and exact CPU fallback were exercised. Other explicit product
boundaries are the four-frame FFmpeg-process video LRU, OBJ-only imported mesh
format, mono equirectangular projection, approximate physical/lighting models,
and the lack of a full GPU raster backend. None is represented as a completed
capability beyond the scope documented in `feature-matrix.md`.

## Port to an in-process engine (P0–P7)

The phases above describe the v1.1 engine as first delivered: FFmpeg ran as
a supervised child process for decoding, encoding and text (`drawtext`),
video frames were cached four at a time, and resume kept raw RGBA frames on
disk. The port below replaced those parts, one phase per branch, each merged
after an independent review of its diff and a round of fixes for the
review's findings. Verification throughout used the Freedesktop SDK 25.08
(GCC 15.2.0, glibc 2.42, x86-64). The statements in the earlier sections
that describe the process-based design are kept as history; README.md and
the other documents describe the current code.

**P0 — build and test scaffolding.** CMake and Make gained `SR_WERROR`,
`SR_SANITIZE` and `SR_COVERAGE`, stricter warnings, and
`-ffp-contract=off -fno-fast-math` on every target, so compilers cannot fuse
multiply-adds and change rounding. The single test program became
`tests/unit/` with one CTest entry per suite, and the integration script
writes only to a work directory. The new UBSan runs found a `qsort` on
empty groups, a half-texel offset in panorama sampling and non-exponential
velocity damping, which were fixed (physics cache format 2). Verified by
Release and ASan/UBSan CTest runs.

**P4 — float premultiplied render core.** Frames and cached images became
float premultiplied RGBA in a blend space (working gamut, linear light when
requested), with one input conversion for 8-bit sources and one output
conversion to 8- or 16-bit straight RGBA. Shapes, masks and image edges got
signed-distance coverage at the pixel footprint; isolated groups draw into
pooled buffers with dirty rectangles. The goal was correct linear-light
blending and a single, exact color path. Verified by new blend, raster,
mask, group, image and path suites, 1- vs 7-thread preview equality, and
regenerated integration hashes.

**P1+P2 — in-process libav encode and decode.** The FFmpeg child processes
were replaced by libavformat/libavcodec/libswscale/libswresample: an
encoder writing H.264/H.265/FFV1 and AAC into MP4/QuickTime/Matroska with
bit-exact flags, color tags and spherical side data; a persistent decoder
per video asset with keyframe seeking and a 256 MiB frame LRU; audio decoded
once in memory and mixed per frame straight into the encoder. This removed
process start-up per cache miss, temporary files and pipe framing. Verified
by encode round trips, seeking-vs-sequential equality, A/V origin and
timestamp-gap tests, and `unit.encode_faults`, which fails each libav call
in turn through `--wrap` link options.

**P5 — feature parity.** Group effects, animatable effect and color
parameters, drop shadow and 2D lighting effects, parametric particles,
mesh-warp, mass-spring soft bodies, oriented-box contacts, vortex fields and
pins, shadow maps for directional and spot lights, and 3D supersampling were
added to match the reference implementation. Verified by the fx,
anim_color, particles, deform, physics and shadow suites and example
previews; the review round bounded particle integration, integer
conversions of large coordinates, soft-body substeps and cache payloads.

**P3 — in-process text.** The `drawtext` path was replaced by
FreeType/HarfBuzz/FriBidi/Fontconfig: per-line shaping, UAX #9 reordering,
wrapping at grapheme clusters, alignment including justify, letter spacing
and vertical alignment, with fonts from `fontFile` or a Fontconfig family
that must exist. It was the last use of an external process. Verified by
the text suite (kerning, ligatures, bidi order, wrapping, overflow, thread
invariance, failure injection of shaping) and example previews.

**P6 — CLI, runtime XSD validation, segmented resume, hashing.** Argument
parsing moved to a pure `cli_args` module; the XSD is embedded at build time
and every scene is validated with libxml2 before loading; `--hash` prints
per-frame hashes of the encoder input; `--resume` renders independently
encoded segments with a manifest and assembles them by packet copy (this
replaced the raw-frame resume cache); `--physics-cache` names a cache
directory. Verified by `unit.args` and the `cli.*` CTest entries (exit
codes, schema errors, preview names, thread-invariant hashes, interrupted
and resumed renders, physics cache reuse).

**P7 — verification depth and documentation.** Added `unit.golden`: 14
scenes under `tests/golden/` rendered at 2–3 frames with 1 and 4 threads and
compared byte for byte with reviewed PNG references
(`SR_UPDATE_GOLDEN=1` regenerates them). Added `unit.oom`: the unit binary
wraps `malloc`/`calloc`/`realloc`/`free`, and every engine allocation of a
scene load, an asset load, a one-frame render (1 and 3 threads) and an
encoder open/frame/finish is failed in turn, checking the status, the
diagnostics and that nothing leaks. It found allocation failures that were
swallowed or misreported (scene root id, soft-body step scratch, video
frame conversion during drawing, path and OBJ parsing, `fps` parsing,
node attachment), and a mesh leak when assets were loaded twice; these were
fixed. Added `tools/coverage.py` (gcov line and branch totals for `src/`)
and `tools/coverage-gate.sh` (fails 2 points below the measured 90.30% of
lines and 71.29% of branches). The documentation was checked against the
code. Verified by Release, ASan/UBSan and coverage CTest runs (45 tests).
