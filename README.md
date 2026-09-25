# scene-render

`scene-render` 1.1.0 is a modular C17/POSIX video-generation engine. It reads a
validated XML scene, evaluates an absolute-frame timeline, simulates visual
physics at a fixed timestep, renders one RGBA frame at a time, mixes bounded
audio blocks, and streams synchronized media to FFmpeg. The scene model,
validation, timeline, camera, renderer, compositor, lighting/effects, physics,
deformation, audio mixer, caches, diagnostics, and CLI are implemented in C.

The engine renders standard 3840×2160 video, configurable 2:1 equirectangular
video (including 3840×1920), and animated 3840×2160 perspective viewports from
360° scenes. It supports shared image/video/audio assets, shaped UTF-8 text,
filled vector paths, imported Wavefront OBJ meshes, nested layer groups and
masks, six blend modes, particles, animated lights/materials/cameras, fixed-step
rigid bodies, visual soft bodies, five deformation modifiers, and ordered post
effects.

## Build

Required and optional dependencies, exact verified versions, and licenses are
listed in [`docs/dependencies.md`](docs/dependencies.md). Building requires
`pkg-config` and the FFmpeg development libraries (libavformat ≥ 60,
libavcodec ≥ 60, libavutil ≥ 58, libswscale ≥ 7, libswresample ≥ 4; e.g.
Debian/Ubuntu `libavformat-dev libavcodec-dev libavutil-dev libswscale-dev
libswresample-dev`) with the libx264/libx265/FFV1/AAC encoders you intend to
use, plus FreeType, HarfBuzz (≥ 2.8.2), FriBidi and Fontconfig development
files for text (Debian/Ubuntu `libfreetype-dev libharfbuzz-dev libfribidi-dev
libfontconfig-dev`). Encoding, media decoding and text rendering all run
in-process; no `ffmpeg` executable is needed. Production codecs are
deliberately not reimplemented. The tests build `sr-probe`, which replaces
`ffprobe` in the integration script.

```sh
cmake -S . -B build -DCMAKE_BUILD_TYPE=Release
cmake --build build --parallel
ctest --test-dir build --output-on-failure
```

The strict Make path is equivalent:

```sh
make -j
make test
```

Both builds select C17 and compile project sources with
`-Wall -Wextra -Wpedantic -Werror`. POSIX clocks, files and pthreads are the
documented platform layer.

## Commands

```sh
# Grammar and semantic validation without media decode or rendering.
./build/scene-render --scene examples/basic-multilayer.xml --validate

# Ten-second 4K UHD render with metrics.
./build/scene-render --scene examples/ten-layer-composition.xml \
  --output build/demo.mp4 --threads auto --quality high --metrics

# Configurable equirectangular output with Spatial Media metadata in MP4.
./build/scene-render --scene examples/equirectangular-360.xml \
  --output build/panorama.mp4 --quality high

# Animated 4K viewport from a 3840×1920 panorama.
./build/scene-render --scene examples/viewport-360.xml \
  --output build/viewport.mp4 --threads auto

# One preview or a half-open frame range.
./build/scene-render --scene examples/production-features.xml \
  --preview-frame 30 --preview-out build/frame-0030.ppm
./build/scene-render --scene examples/keyframe-curves.xml \
  --preview-frame 30 --preview-out build/frame-0030.png
./build/scene-render --scene examples/video-audio-remap.xml \
  --frame-range 120:180 --output build/range.mp4

# Request the optional OpenCL path; unavailable hardware falls back safely.
./build/scene-render --scene examples/production-features.xml \
  --renderer gpu --threads auto --output build/gpu-request.mp4

# Render a viewport-mode scene as its equirectangular 360 master.
./build/scene-render --scene examples/archive-beacon.xml \
  --mode equirectangular --output build/archive-beacon-360.mp4

# Resumable render: segments of 150 frames under build/viewport.mp4.parts/;
# rerunning the same command after an interruption renders only the
# missing segments.
./build/scene-render --scene examples/viewport-360.xml \
  --output build/viewport.mp4 --resume

# Per-frame FNV-1a hashes of the exact encoder input, without encoding.
./build/scene-render --scene examples/keyframe-curves.xml \
  --frame-range 0:30 --hash

# Validate against the embedded XSD, or print it.
./build/scene-render --scene examples/keyframe-curves.xml --validate
./build/scene-render --print-schema > scene-v1.xsd
```

Run `scene-render --help` for the complete CLI. Argument parsing lives in
`src/cli_args.c` (a pure function tested by `unit.args`); `src/main.c` only
orchestrates.

| Option | Meaning |
|---|---|
| `--scene FILE` | Scene XML; required except with `--help`, `--version`, `--print-schema` |
| `--output FILE` | Override `output/@path`; the container follows the extension (`.mp4`, `.mov`, `.mkv`) |
| `--validate` | XSD + semantic validation only; prints `valid: FILE (N assets, M top-level layers)` |
| `--print-schema` | Print the XSD embedded in the binary (`schema/scene-v1.xsd` at build time) |
| `--frame-range A:B` | Render the half-open range `[A,B)` (`A < B`; `B` is clamped to the scene) |
| `--preview-frame N`, `--frame N` | Write one 8-bit preview frame and print `<path> <16-hex FNV-1a 64>` over its RGBA bytes |
| `--preview-out FILE` | Preview path; default `frame-NNNNNN.png` in the working directory; `.png` writes PNG, anything else PPM |
| `--hash` | Render the range without encoding; print `<frame> <16-hex>` per frame over the exact bytes the encoder would receive (8- or 16-bit RGBA), then `audio <16-hex>` over the range's mixed float PCM when the scene has audio. Identical for every `--threads` value |
| `--mode standard\|equirectangular\|viewport` | Override the project mode (360 modes need `scene360`) |
| `--resolution WIDTHxHEIGHT`, `--fps N` or `N/D`, `--quality low\|medium\|high` | Override size, rate, encoder quality preset |
| `--threads auto\|N`, `--renderer cpu\|gpu` | Worker count; OpenCL colour conversion with CPU fallback |
| `--resume`, `--segment-frames N`, `--keep-parts` | Segmented resumable render (below); default 150 frames per segment |
| `--physics-cache DIR` | Store/reuse the physics simulation as `DIR/physics-<signature>.bin` (created if missing); overrides `physics/@cache` |
| `--metrics`, `--metrics-trace FILE` | Metrics on stderr; per-frame stage timings as JSON Lines |
| `--verbose`, `--version`, `--help` | Verbose diagnostics; `scene-render VERSION`; usage |

Each option may be given once (repeating one, or its alias, is a usage
error naming it). A valued option's value may not be empty and may not begin
with `--` (`--scene --validate` is a missing value, not a file named
`--validate`). Numbers are unsigned decimal digits only: signs, whitespace
and trailing characters are rejected.

Exit status is a stable contract: `0` success, `2` usage or argument error
(unknown option, frame out of range, bad override), `3` XML/XSD/semantic
error, `4` asset error, `5` render error, `6` encoder error, `7` I/O error
(including standard output that cannot be written, e.g. `>/dev/full`), `8`
out of memory.

The scene file is read once into memory (at most 64 MiB, else exit 3
"scene file too large"); both parsers and the resume fingerprint use those
bytes, decoded as UTF-8. A DOCTYPE (or any other markup declaration) in the
prolog is rejected before either parser runs. Every scene is then validated
against the embedded XSD with libxml2 (≥ 2.9.14; an external entity loader
that refuses every load is installed for the whole pass) before the Expat
loader applies its semantic checks; each schema error is reported as
`FILE:LINE: error: <element> @attribute: message` and exits with 3. Element
nesting is limited to 256 levels.

`--resume` renders the range in segments of `--segment-frames` frames, each an
independently encoded video-only file with its own keyframe in
`OUTPUT.parts/` (written under a unique temporary name, fsynced, committed by
atomic rename), next to a `manifest` that records the hash of the scene
bytes that were parsed, every input file's size, mtime and full-content hash
(assets, `fontFile` fonts and the files font families resolved to), the
range, the segment size, all effective video settings, the resolved thread
count, the colour-conversion backend actually used and the engine version.
Input files are hashed before they are loaded and re-checked (size, mtime,
inode) after loading and before every segment commit; a change is exit 4
"changed while loading/rendering". A rerun with any manifest difference
discards the old segments; otherwise only missing segments are rendered,
and a kept segment that no longer demuxes as expected (stream parameters,
frame count, leading keyframe) is deleted and rendered again. The output is
then muxed into a temporary file beside `OUTPUT` by copying the segments'
video packets (timestamps shifted per segment) while the audio of the whole
range is mixed and encoded once; spherical metadata is applied to that file,
which then replaces `OUTPUT` by rename, so a failed assembly leaves the
previous `OUTPUT` untouched. `OUTPUT.parts/` must be a real directory owned
by the invoking user (a symbolic link is refused); nothing inside it is
followed, and only segment/manifest/lock names are ever deleted. A run holds
an exclusive lock on `OUTPUT.parts/lock`: a concurrent `--resume` on the same
`OUTPUT` exits 7 "another render is using". `OUTPUT.parts/` is removed
afterwards unless `--keep-parts` is given. A resumed render is byte-identical to an
uninterrupted `--resume` render of the same command, but not to a render
without `--resume`: every segment starts with a keyframe, so the video
bitstream differs (frames decode to the same pictures within codec loss).

`--metrics` prints a `metrics:` line (the engine's own RSS is the whole
memory cost: there are no encoder child processes), a `video:` line with
decoder cache statistics (`sources requests cache_hits decoded seeks`), a
`physics:` line (`steps` simulated by this run, `cache_hit`), a `resume:`
line (`segments_rendered segments_reused`), and per-stage times.

The GPU selection dynamically loads OpenCL 1.2 and executes final named-space
color conversion on a GPU. Scene traversal and the reference rasterizer remain
CPU-based. If the loader, device, or kernel is unavailable, the exact CPU path
is selected with a warning. No OpenCL SDK is required to compile the project.

## Supplied scenes

| Scene | Demonstrates |
|---|---|
| `basic-multilayer.xml` | 10-second 4K hierarchy, masks, transforms, blends |
| `ten-layer-composition.xml` | Ten simultaneous transparent/blended instances |
| `keyframe-curves.xml` | Step, linear, ease-in/out, ease-in-out, cubic Bézier |
| `video-audio-remap.xml` | Shared video cache, loop/reverse/remap, mixed AAC audio |
| `equirectangular-360.xml` | 3840×1920 equirectangular output |
| `viewport-360.xml` | Animated yaw, pitch, roll, and FOV into 4K UHD |
| `lighting-physics-deformation.xml` | Lights, particles, effects, collision, constraints, soft-body approximation, five modifiers |
| `production-features.xml` | UTF-8 shaping, path rasterization, shaped/inverted masks, OBJ mesh, P3→sRGB conversion |
| `v1.1-feature-showcase.xml` | Ten-second 4K integration showcase with synchronized audio |
| `archive-beacon.xml` | 30-second sci-fi sequence authored once in panorama space and rendered as both the UHD viewport cut and the 360 master: 3D station and planet, reused telemetry video, debris physics, shield deformation, beam, audio mix, end card. Generated by `scripts/archive-beacon/build_scene.py`; `scripts/archive-beacon/render.py` renders both with measured render times on the end card |
| `gravity-well.xml` | 12-second 1080p orbit study around a radial force field |
| `dusk-parallax.xml` | 48-second 1080p explainer about this engine, set in a five-plane parallax mountain range (CC0 pixel art, see `assets/third-party/dusk-parallax`): dolly zoom and tilt per depth plane, screen-blended wave-deformed fog, particle fireflies, vector-path birds, syntax-coloured XML card, pipeline diagram. Generated by `scripts/dusk-parallax/build_scene.py` after `prepare_assets.py` |

Generated sample media under `examples/assets` contains only FFmpeg test
signals. `octahedron.obj` is original project test geometry.

The latest complete render is `artifacts/v1.1-feature-showcase-4k.mp4`; its
5-second inspection frame and hashes are stored beside it. The verified file
contains 300 3840×2160 H.264 frames and 10.000000 seconds of stereo AAC.

## Execution pipeline

1. Expat tokenizes XML while engine-owned checks report source line, element,
   attribute, and reason. DOCTYPE is rejected.
2. References are resolved, child order is stabilized by `(z, XML order)`, and
   layer instances point to shared immutable asset records.
3. Still media is decoded once. Video is decoded lazily into a four-frame
   per-source LRU. Declared source color spaces convert into the working space.
4. Rigid bodies simulate at XML `fixedStep`, independent of output FPS.
   Versioned/signature-checked pose caches can be reused across renders.
5. Every frame time is calculated directly from its integer index. Lighting,
   depth-tested primitives/meshes, ordered 2D compositing, optional viewport
   extraction, effects, and output color conversion produce one RGBA frame.
6. Audio sources decode to temporary float streams and mix in 4096-frame
   blocks with trim, finite loops, volume, and pan.
7. RGBA video and float audio stream to a supervised FFmpeg process at an exact
   rational frame rate. MP4 equirectangular files receive Spatial Media v1 UUID
   metadata; Matroska receives projection/spherical stream tags.

See [`docs/architecture.md`](docs/architecture.md),
[`docs/xml-reference.md`](docs/xml-reference.md), and the normative
[`schema/scene-v1.xsd`](schema/scene-v1.xsd).

## Determinism

With identical XML, input assets, engine and dependency versions, font setup,
platform floating-point behavior, and seed, CPU-rendered RGBA frames are
byte-identical. Frame times never accumulate. Physics has a fixed step,
particles use stateless integer hashes, ordering is stable, and parallel jobs
write disjoint output ranges. Tests compare one- and four-thread results.

Encoded container bytes are outside the frame determinism boundary because
external encoder builds can change. Golden tests hash the lossless rendered
PPM output. The optional GPU color kernel is not claimed byte-identical across
different GPU vendors; CPU fallback remains the reference.

## Color, streaming, and memory

Projects select `srgb`, `rec709`, `display-p3`, or `rec2020` working space.
Image and video assets declare their source space (sRGB by default). Linear
light blending uses the active space's transfer function, final RGB conversion
uses explicit matrices, and FFmpeg output receives primaries, transfer,
matrix, and full/limited range tags.

Input and output are streamed; entire videos are never retained. The principal
working set is current render surfaces, effect scratch buffers, decoded stills,
four frames per active video asset, physics poses, and bounded audio blocks.
Metrics report render/write/wait/wall time, throughput, process RSS values, and
a conservative self-plus-child peak upper bound.

## Physical and visual scope

The fixed-step 2D solver is deterministic visual behavior, **not verified
physical simulation**. It uses semi-implicit Euler integration, circle and
axis-aligned-box contacts (mixed pairs use bounding circles), restitution/friction
impulses, force fields, damping, and iterative
spring/distance constraints. Soft bodies, deformation, bounding-volume mesh
shadows, screen-space ground shadows, and lens flare are visual approximations.

The CPU 3D path supports lit sphere/box/plane primitives and triangulated
Wavefront OBJ geometry with a depth buffer. It is not a general PBR engine,
does not promise physically based energy conservation, and does not implement
arbitrary material/shader graphs. These boundaries are explicit in
[`docs/feature-matrix.md`](docs/feature-matrix.md).

## Verification

```sh
make unit
make integration
make test
```

The suite covers contextual XML failures, deep dynamic layer nesting, all
interpolation/blend modes, shared assets, vector paths, OBJ import, named color
conversion, exact goldens, deterministic parallelism, shaped/inverted masks,
physics cache replay, animated particles/effects/deformation, A/V duration equality,
360 output, MP4 spherical UUID metadata, segmented resume (interrupted and
completed runs byte-identical, manifest invalidation), the CLI contract
(`cli.*` CTest entries driven by `tests/cli/*.cmake`), and H.264/H.265/FFV1
when exposed by FFmpeg. CMake/CTest and sanitizer results are recorded in
[`docs/phases.md`](docs/phases.md); the 4K measurement is in
[`docs/benchmark.md`](docs/benchmark.md).

## Troubleshooting

- `libavcodec encoder ... is unavailable`: the linked FFmpeg libraries lack
  that encoder; install a build exposing libx264/libx265/FFV1/AAC as needed.
- `encoder ... does not support pixel format`: pick one of the listed formats.
- `output ... needs a .mp4, .mov or .mkv extension` / `ffv1 requires a .mkv`:
  change `--output` or `output/@path`.
- Encoding failure: rerun with `--verbose` (libav's own log is shown then);
  check disk space and output permissions.
- Asset decode failure: verify path and declared width/height/FPS/duration.
- Text failure: verify the requested Fontconfig family or `fontFile` path.
- High memory use: bloom/blur uses two additional RGBA buffers; viewport mode
  also retains panorama and viewport frames.
- Slow divergent time remaps: each video asset caches up to 256 MiB of
  converted frames; `--metrics` shows cache hits and seeks.
- Resume disk use: `OUTPUT.parts/` holds one encoded segment per
  `--segment-frames` frames until the render completes (`--keep-parts` keeps
  it); delete it to force a full re-render. Every input file is hashed in
  full on each `--resume` run, which costs one read of each asset.
- OpenCL warning: no usable GPU was found; the deterministic CPU reference was
  selected automatically.

## License

Original code is Apache-2.0; see [`LICENSE`](LICENSE). Third-party components
are not vendored as source. Exact verified versions and licenses are documented
in [`docs/dependencies.md`](docs/dependencies.md).

The one vendored third-party asset is the Inter typeface in
[`assets/third-party/inter`](assets/third-party/inter), used by
`archive-beacon.xml`. It is licensed under the SIL Open Font License 1.1, not
Apache-2.0; its source URL, version, checksums, and attribution sit beside the
font files.
