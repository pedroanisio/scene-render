# scene-render

`scene-render` 1.0.0 is a modular C17/POSIX video-generation engine. It reads a
validated XML scene, evaluates an absolute-frame timeline, simulates physics at
a fixed timestep, renders one RGBA frame at a time, mixes audio in bounded
blocks, and streams the result to FFmpeg. Original orchestration, scene,
timeline, camera, renderer, compositor, lighting/effects, physics/deformation,
audio mixer, diagnostics, cache, and CLI code is C.

The engine supports standard 3840×2160 output, 2:1 equirectangular 360° output,
and animated perspective viewport extraction from a 360° canvas. It also
supports shared image/video sources, text and vector assets, nested layers,
masks, six blend modes, particles, simple lit 3D primitives, post-processing,
fixed-step 2D rigid bodies, springs, force fields, and five deformation
modifiers.

## Build

Dependencies and tested versions are in
[`docs/dependencies.md`](docs/dependencies.md). FFmpeg must be available at run
time; production codecs are not implemented by this project.

```sh
cmake -S . -B build -DCMAKE_BUILD_TYPE=Release
cmake --build build --parallel
ctest --test-dir build --output-on-failure
```

The supplied environment was verified with the strict Makefile path:

```sh
make -j
make test
```

Both paths select C17. The project is compiled with
`-Wall -Wextra -Wpedantic -Werror`; POSIX `fork`, pipes, clocks, and pthreads are
the deliberately documented platform layer.

## Commands

```sh
# Validate only; no media is decoded.
./build/scene-render --scene examples/basic-multilayer.xml --validate

# Ten-second 4K UHD render.
./build/scene-render --scene examples/basic-multilayer.xml \
  --output build/demo.mp4 --threads auto --quality high --metrics

# 3840x1920 equirectangular render.
./build/scene-render --scene examples/equirectangular-360.xml \
  --output build/panorama.mp4 --quality high

# Animated 3840x2160 viewport from a 3840x1920 panorama.
./build/scene-render --scene examples/viewport-360.xml \
  --output build/viewport.mp4 --threads auto

# One preview and a half-open frame range.
./build/scene-render --scene examples/lighting-physics-deformation.xml \
  --preview-frame 90 --preview-out build/frame-0090.ppm
./build/scene-render --scene examples/video-audio-remap.xml \
  --frame-range 120:180 --output build/range.mp4

# Preserve completed RGBA frames and reuse them after interruption.
./build/scene-render --scene examples/viewport-360.xml \
  --output build/viewport.mp4 --resume
```

The complete CLI is available with `--help`. Important options are
`--validate`, `--frame-range A:B`, `--preview-frame N`, `--preview-out FILE`,
`--resolution WIDTHxHEIGHT`, `--fps N/D`, `--quality low|medium|high`,
`--threads auto|N`, `--renderer cpu|gpu`, `--resume`, `--verbose`, and
`--metrics`.

There is currently no GPU implementation. `--renderer gpu` emits a capability
warning and uses the deterministic CPU backend. Viewport projection uses
deterministic row partitions when multiple threads are requested; the scalar
compositor remains the fallback everywhere.

## Supplied scenes

| Scene | Demonstrates |
|---|---|
| `basic-multilayer.xml` | 10-second 4K composition, groups, masks, blends |
| `ten-layer-composition.xml` | Ten simultaneous alpha/blend layers |
| `keyframe-curves.xml` | All six interpolation modes |
| `video-audio-remap.xml` | Shared video instances, loop/reverse/remap, AAC audio |
| `equirectangular-360.xml` | 3840×1920 360° output |
| `viewport-360.xml` | Animated yaw, pitch, roll, and FOV into 4K UHD |
| `lighting-physics-deformation.xml` | Four lights, shadows, 3D, particles, effects, collision, spring, force fields, soft-body approximation, and all deformation families |

The small video and WAV in `examples/assets` are redistributable generated test
signals created with FFmpeg's `testsrc2` and `sine` sources.

The complete animated feature showcase is
`artifacts/v1.0-feature-showcase-4k.mp4`: 10 seconds, 3840×2160, 30 fps,
H.264, with lighting, particles, rigid-body physics, force fields, a spring,
soft-body-style motion, deformation, shadows, and post-processing. A 5-second
inspection frame is supplied as `artifacts/v1.0-feature-showcase-frame-5s.png`.

## Execution pipeline

1. Expat tokenizes XML while engine-owned validators report source line,
   element, attribute, and reason.
2. References are resolved, layer order is stabilized by `(z, XML order)`, and
   immutable sources are shared between instances.
3. Media frames are decoded lazily into a four-frame per-source LRU. Still,
   text, and vector sources are decoded or rasterized once.
4. Rigid bodies are simulated at XML `fixedStep`, independent of output FPS.
   Optional binary caches are loaded before simulation and written afterward.
5. Each absolute frame time is evaluated directly. Lighting/3D, ordered 2D
   compositing, 360 projection, and post-effects produce the output frame.
6. Audio sources are decoded to temporary float streams and mixed in 4096-frame
   blocks with trim, loops, volume, and pan.
7. RGBA video and float audio stream to a supervised FFmpeg process with exact
   rational frame rate and synchronized zero-based output timestamps.

See [`docs/architecture.md`](docs/architecture.md) for module ownership and
failure behavior, and [`docs/xml-reference.md`](docs/xml-reference.md) plus
[`schema/scene-v1.xsd`](schema/scene-v1.xsd) for the scene language.

## Determinism

For identical XML bytes, assets, CLI overrides, engine/dependency versions,
platform floating-point behavior, and seed, CPU-rendered RGBA frames are
byte-identical. Frame times are `frame_index × fps_den / fps_num`; no time is
accumulated. Physics uses a fixed step, particles use stateless integer hashes,
layer order is stable, and parallel viewport workers write disjoint rows.

Encoded container bytes are not the determinism boundary because encoder
implementations can change. Tests compare decoded/rendered frames exactly and
pin SHA-256 golden values.

## Color and memory

Decoded RGB is treated as sRGB. When `linearLight="true"`, blend operands are
converted with the sRGB transfer function, blended in linear light, and
converted back. Alpha uses straight-alpha source-over equations. FFmpeg handles
the requested output pixel conversion.

Video is never loaded in full. The working set is output/panorama frames,
effect scratch buffers when needed, cached stills, four decoded frames per
video source, physics samples, and bounded audio buffers. Resume caches use raw
RGBA files and therefore trade disk space for recoverability.

## Physical and visual scope

The 2D rigid-body solver is deterministic and useful for visual motion, but it
is **not claimed to be physically accurate**. It uses semi-implicit Euler,
simple circle/AABB contacts, impulse restitution/friction, and iterative spring
constraints. Soft bodies, mesh-like deformation, 3D shadows, and lens flare
are explicitly visual approximations. The built-in 3D backend renders spheres,
boxes, and planes; it is not a general mesh renderer. The built-in text raster
supports a compact ASCII glyph set, and vectors are rectangles or ellipses.

These boundaries are listed in [`docs/feature-matrix.md`](docs/feature-matrix.md).

## Verification

```sh
make unit
make integration
make test
```

Tests cover XML failures, DOCTYPE rejection, all interpolation/blend modes,
shared assets, exact golden frames, deterministic 1-vs-4-thread viewport
projection, physics cache replay, particles/effects/deformation, 360 output,
resume reuse, H.264/H.265/FFV1 when installed, and synchronized H.264/AAC.
Reports and artifact hashes are in [`docs/phases.md`](docs/phases.md),
[`docs/benchmark.md`](docs/benchmark.md), and `artifacts/SHA256SUMS`.

## Troubleshooting

- `FFmpeg executable is unavailable`: install FFmpeg and inspect
  `ffmpeg -encoders` for `libx264`, `libx265`, `ffv1`, and the requested audio
  codec.
- Encoder pipe failure: rerun with `--verbose`; validate codec/container and
  pixel-format compatibility plus disk space.
- A video frame cannot decode: confirm XML width, height, FPS, and duration
  match the source. `ffprobe` is useful for inspection.
- High memory use: blur/bloom needs two extra RGBA buffers; viewport mode also
  retains its panorama and output frame.
- Slow remapped video: widely separated instances can exceed the four-frame
  LRU and cause extra FFmpeg decodes.
- Resume consumes disk: remove the explicit `OUTPUT.resume` directory when it
  is no longer needed. Cache signatures isolate changed scenes/assets.
- GPU warning: this build has no GPU backend and has intentionally fallen back
  to CPU.

## License

Original code is Apache-2.0; see [`LICENSE`](LICENSE). Third-party components
are not vendored as source. Their exact tested versions and licenses are
documented in [`docs/dependencies.md`](docs/dependencies.md).
