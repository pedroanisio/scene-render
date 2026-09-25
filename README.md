# scene-render

`scene-render` 1.1.0 is a modular C17/POSIX video-generation engine. It
validates an XML scene against an embedded XSD, evaluates an absolute-frame
timeline, simulates visual physics at a fixed timestep, renders one float
premultiplied frame at a time, mixes the audio for each frame, and encodes
video and audio in-process through the FFmpeg libraries (libavformat,
libavcodec, libswscale, libswresample). Text is laid out with FreeType,
HarfBuzz, FriBidi and Fontconfig, also in-process: the engine never starts
another program. The scene model, validation, timeline, camera, renderer,
compositor, lighting/effects, physics, deformation, audio mixer, caches,
diagnostics, and CLI are implemented in C.

The engine renders standard 3840×2160 video, configurable 2:1 equirectangular
video (including 3840×1920), and animated 3840×2160 perspective viewports from
360° scenes. It supports shared image/video/audio assets, shaped UTF-8 text,
filled vector paths, imported Wavefront OBJ meshes, nested layer groups and
masks, six blend modes, particles, animated lights/materials/cameras, fixed-step
rigid bodies, visual soft bodies, five deformation modifiers, and ordered post
effects.

## Build

Required and optional dependencies, exact verified versions, and licenses are
listed in [`docs/dependencies.md`](docs/dependencies.md). Building requires a
C17 compiler, CMake ≥ 3.20 (or GNU Make), `pkg-config`, and the development
files of:

| Library | Minimum | Debian/Ubuntu package |
|---|---|---|
| Expat | 2.5 | `libexpat1-dev` |
| libxml2 | 2.9.1 | `libxml2-dev` |
| libavformat, libavcodec, libavutil, libswscale, libswresample | 60, 60, 58, 7, 4 | `libavformat-dev libavcodec-dev libavutil-dev libswscale-dev libswresample-dev` |
| FreeType, HarfBuzz, FriBidi, Fontconfig | any, 2.8.2, 1.0, 2.13 | `libfreetype-dev libharfbuzz-dev libfribidi-dev libfontconfig-dev` |

libavcodec must expose the encoders the scenes use (`libx264` for `h264`,
`libx265` for `h265`, `ffv1`, `aac`; `png` for PNG previews). Whether a given
FFmpeg build is LGPL or GPL depends on its configure flags (`--enable-gpl
--enable-libx264 --enable-libx265` make it GPL); see `docs/dependencies.md`.
No `ffmpeg` or `ffprobe` executable is needed: the tests build `sr-probe`
for stream inspection. OpenCL is loaded at run time only for
`--renderer gpu`.

```sh
cmake -S . -B build -DCMAKE_BUILD_TYPE=Release
cmake --build build --parallel
ctest --test-dir build --output-on-failure
```

The Make path needs no CMake and builds the same binaries (`BUILD=DIR`
selects the output directory); `make test` runs every unit suite and the
integration script, while the `cli.*` checks run only under CTest:

```sh
make -j
make test
```

Both builds select C17 and compile project sources with
`-Wall -Wextra -Wpedantic -Wshadow -Wstrict-prototypes -Wmissing-prototypes
-Wformat=2 -Werror` (`SR_WERROR=OFF`/`SR_WERROR=0` drops `-Werror`) and
`-ffp-contract=off -fno-fast-math`. `-DSR_SANITIZE=ON` (`make SR_SANITIZE=1`)
adds AddressSanitizer and UBSan, `-DSR_COVERAGE=ON` (`make SR_COVERAGE=1`)
gcov instrumentation, `-DSR_PROFILE=ON` gprof. POSIX clocks, files and
pthreads are the platform layer.

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

`--metrics` prints a `metrics:` line (the process's own peak RSS is the
whole memory cost, since encoding is in-process), a `video:` line with
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
| `dusk-depth.xml` | 8-second 1080p rebuild of the dusk range with depth cards: each plane only declares `depth`, one camera trucks, dollies and yaws; parallax, depth scaling, a perspective title card, fog planes, card-scaled fireflies, depth of field, and a 3D beacon occluded by the nearer forest card all follow from the camera |

Generated sample media under `examples/assets` contains only FFmpeg test
signals. `octahedron.obj` is original project test geometry.

The latest complete render is `artifacts/v1.1-feature-showcase-4k.mp4`; its
5-second inspection frame and hashes are stored beside it. The verified file
contains 300 3840×2160 H.264 frames and 10.000000 seconds of stereo AAC.

## Execution pipeline

1. **Validate and load.** libxml2 validates the document against the XSD
   embedded at build time (no network, no external entities, no DTD); each
   schema error is reported as `FILE:LINE: error: <element> @attribute:
   message`. Expat then builds the owned scene graph with semantic checks
   (numeric ranges, unique ids, reference types, 2:1 panoramas, clip
   bounds). A DOCTYPE is rejected.
2. **Resolve.** References are resolved, children are ordered by
   `(z, XML order)`, and layers point to shared asset records.
3. **Assets.** Stills are decoded once with libavformat/libavcodec (PNG,
   JPEG, PPM, ...) and converted to blend space; text is shaped and
   rasterized once per asset; vector paths are filled and stroked once; OBJ
   meshes are parsed once. Each video asset opens one persistent decoder
   whose converted frames live in a 256 MiB per-asset LRU. Each audio asset
   is decoded once, in memory, to the mix format with libswresample.
4. **Physics.** Rigid and soft bodies are simulated for the whole duration
   at `physics/@fixedStep`, independent of the output rate; the samples can
   be stored in and restored from a signature-checked cache
   (`physics/@cache`, `--physics-cache`).
5. **Frames.** Frame `N` is rendered at time `N × fps_den / fps_num`,
   computed from the integer index: clear to the background, 3D pass
   (depth-tested primitives and meshes, shadow maps, optional supersampling),
   2D compositing (groups, masks, blends, deformers, particles, group
   effects), viewport extraction in viewport mode, whole-frame effects, and
   conversion to 8- or 16-bit straight RGBA in the output color space.
6. **Audio.** The mixer produces exactly the samples of frame `N`,
   `[floor(N·rate·fps_den/fps_num), floor((N+1)·rate·fps_den/fps_num))`,
   with trim, loops, volume, equal-power pan, fades, speed and reverse.
7. **Encode.** libavcodec encodes the video (libx264, libx265 or FFV1 via
   libswscale's explicit Y'CbCr matrix and range) and the audio (AAC by
   default) with bit-exact flags; libavformat muxes MP4/QuickTime or
   Matroska with color tags. Equirectangular output carries spherical side
   data (MP4 `sv3d`/`st3d`, Matroska `Projection`) and, in MP4, the Spatial
   Media v1 UUID box injected after the encode. `--hash` stops before the
   encoder, previews write PNG or PPM, and `--resume` encodes segments that
   are then assembled by packet copy.

See [`docs/architecture.md`](docs/architecture.md),
[`docs/xml-reference.md`](docs/xml-reference.md), and the normative
[`schema/scene-v1.xsd`](schema/scene-v1.xsd).

## Architecture

| Module (`src/`) | Responsibility |
|---|---|
| `xml_schema`, `xml`, `xml_elements`, `xml_nodes`, `xml_visual`, `xml_camera`, `xml_audio`, `xml_physics`, `xml_resolve` | XSD validation (libxml2), Expat callbacks, typed attributes, semantic checks, reference resolution |
| `scene`, `timeline`, `common`, `diagnostics` | owned scene graph, keyframe curves, checked helpers, contextual messages |
| `assets`, `video`, `text`, `vector_path`, `procedural`, `mesh`, `audio` | image/video decoding and the frame LRU, text engine, vector paths, OBJ meshes, audio decoding and the mixer |
| `color`, `raster`, `compositor`, `deform`, `particles`, `effects` | blend-space conversions, coverage and blend kernels, the 2D compositor, warps, particles, frame and group effects |
| `lighting`, `camera`, `physics` | 3D pass and shadow maps, panorama viewport, fixed-step physics and its cache |
| `parallel`, `gpu` | deterministic row jobs, optional OpenCL color conversion |
| `renderer`, `resume`, `encoder`, `spatial` | frame loop, preview/hash/segmented resume, libav encoding and muxing, MP4 spherical metadata |
| `cli_args`, `main` | argument parsing (pure, unit-tested) and orchestration |

The complete table with ownership rules is in
[`docs/architecture.md`](docs/architecture.md); features map to modules and
tests in [`docs/feature-matrix.md`](docs/feature-matrix.md).

## Determinism

With identical XML, input assets, engine and library versions, fonts, and
compiler/C library/architecture, the RGBA frames handed to the encoder are
byte-identical. The reasons:

- every target is compiled with `-ffp-contract=off -fno-fast-math`, so the
  compiler cannot fuse multiply-adds or reorder floating-point arithmetic;
- frame time is computed from the integer frame index and the rational
  frame rate, never accumulated; audio sample ranges use exact integer
  arithmetic;
- physics runs at a fixed step over the whole scene before the first frame;
  particles are closed-form functions of the seed and their index; nothing
  depends on the previous frame or the render order;
- parallel work (`--threads`) is split into disjoint row ranges with no
  floating-point reduction across workers, so any thread count gives the
  same bytes; groups and children draw in a stable order.

`unit.golden` checks this on 14 scenes (1 and 4 threads, and after another
frame rendered first) against committed PNG references, `cli.hash_threads`
compares `--hash` output across thread counts, and the integration script
compares 1- and 4-thread previews and PPM hashes in `tests/golden.sha256`.
The encoders run with bit-exact flags, so identical frames and libraries
also give identical files (`encode.bitexact_output`); another FFmpeg or
x264/x265 build may encode differently. The OpenCL color conversion is not
claimed identical across GPU vendors; the CPU path is the reference.

## Color, streaming, and memory

Projects select an `srgb`, `rec709`, `display-p3` or `rec2020` working space
and optionally linear-light blending. Image and video assets declare their
source space (sRGB by default) and are converted once into the blend space:
float premultiplied RGBA in the working gamut, linear when `linearLight` is
set. The final frame is unpremultiplied, converted to the output gamut and
transfer, clamped, and rounded to 8-bit (16-bit for pixel formats deeper
than 8 bits); libswscale converts it to Y'CbCr with the BT.709 or BT.2020
matrix and the requested range, and the stream is tagged with matching
primaries, transfer, matrix and range.

Nothing is buffered beyond the current frame: the renderer keeps its float
frames (16 bytes per pixel, 126.6 MiB at 3840×2160), one isolated-group
buffer per nesting depth, effect scratch buffers, the 8- or 16-bit encoder
input, the decoded stills and text, up to 256 MiB of converted frames per
video asset, the physics samples, and each audio asset decoded in memory.
Video and audio go straight from memory to libavcodec; there are no
temporary files except `--resume` segments. `--metrics` reports frames,
render/encode/wall seconds, FPS, peak RSS of the process (the whole cost:
encoding is in-process), CPU time, decoder cache statistics, physics steps,
resume segments, and per-stage wall/CPU seconds; `--metrics-trace` writes
one JSON line per frame.

## Physical and visual scope

The fixed-step 2D solver is deterministic visual behavior, **not verified
physical simulation**. It uses semi-implicit Euler integration, exact
circle-vs-oriented-box contacts and a separating-axis test between oriented
boxes, restitution and friction impulses, directional, radial and vortex
force fields, exponential damping, springs, distance constraints and pins.
Soft bodies are mass-spring grids with pressure and pins; deformers,
screen-space point-light shadows and the lens flare are visual
approximations.

The CPU 3D path draws spheres, boxes and planes as lit camera-facing sprites
and Wavefront OBJ meshes as triangles, with a shared depth buffer, shadow
maps for directional and spot lights, and optional supersampling. It is not
a general PBR engine and has no material or shader graphs. These boundaries
are listed in [`docs/feature-matrix.md`](docs/feature-matrix.md).

## Verification

```sh
ctest --test-dir build --output-on-failure    # or: make test
```

CTest runs one entry per unit suite (`unit.<suite>`), the integration script
(`integration`) and the command-line contract (`cli.*`, driven by
`tests/cli/*.cmake`). Among the suites:

- `unit.golden` compares 14 feature scenes in `tests/golden/` byte for byte
  with reviewed PNG references, with 1 and 4 threads;
  `SR_UPDATE_GOLDEN=1 build/sr-unit-tests golden` regenerates them (see
  [`tests/golden/README.md`](tests/golden/README.md) for the determinism
  scope and the review procedure);
- `unit.oom` fails every allocation made by engine code during a scene load,
  an asset load, a one-frame render and an encode, one at a time, and checks
  that each failure returns `SR_ERR_MEMORY` (or `SR_ERR_XML` with an
  "out of memory" diagnostic from inside the Expat callbacks) and leaks
  nothing; allocations inside the shared libraries are not intercepted;
- `unit.encode_faults` fails each libav call the core makes.

The sanitizer build runs the same suite:

```sh
cmake -S . -B build/asan -DCMAKE_BUILD_TYPE=Debug -DSR_SANITIZE=ON
cmake --build build/asan --parallel
ASAN_OPTIONS=detect_leaks=0 ctest --test-dir build/asan --output-on-failure
```

(LeakSanitizer needs ptrace and does not run inside the Flatpak SDK sandbox;
`unit.oom` has its own leak check.) Coverage uses gcov directly:

```sh
cmake -S . -B build/coverage -DSR_COVERAGE=ON
cmake --build build/coverage --parallel
ctest --test-dir build/coverage
python3 tools/coverage.py build/coverage   # per-file and TOTAL line/branch coverage of src/
tools/coverage-gate.sh                     # all of the above; fails below the floors
```

`tools/coverage-gate.sh` is independent of CTest's result: it fails when
line or branch coverage of `src/` falls more than 2 points below the
measured 90.30% of lines and 71.29% of branches. The 4K measurement is in
[`docs/benchmark.md`](docs/benchmark.md) and the history of the phases in
[`docs/phases.md`](docs/phases.md).

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

Two third-party asset sets are vendored, each with its license, source URL,
version, checksums and attribution beside the files, and neither is
Apache-2.0: the Inter typeface in
[`assets/third-party/inter`](assets/third-party/inter) (SIL Open Font
License 1.1; used by `archive-beacon.xml`, `dusk-parallax.xml`, `text-layout.xml` and the golden
text scenes) and the dusk-parallax pixel art in
[`assets/third-party/dusk-parallax`](assets/third-party/dusk-parallax)
(CC0 1.0; used by `dusk-parallax.xml` and `tests/golden/images.xml`).
