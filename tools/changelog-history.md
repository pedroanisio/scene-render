## v0.4.1 — 2026-10-11

This release vendors sr-core schema 1.7.0 byte-identical (document version 1.7) and implements its one change, SREP 87: ids that a program emits are scoped by the program's id. It also reports SREP 72's precision findings under the SREP's codes, and fixes how an included document's references are resolved.

### Schema 1.7.0 and SREP 87

- **Document version 1.7.** `scene/@version` accepts `1.7`. Version 1.7 adds no syntax and gates nothing, so a 1.7 document validates and renders as the same document at 1.6.
- **Program-emitted ids (SREP 87).** An id `E` that the build-mode program `P` emits becomes `P.E` in the document, so two programs, or a program and the document, can use the same local ids.
  - Inside an output, IDREF and IDREFS tokens, `prop` and `propAtTime` ids and the node id of `link/@source` resolve to the output's own ids first.
  - A bare reference from outside resolves when exactly one program emitted the id, with the new warning W13, and fails with the new error PRG17 when several did. The same holds for `override/@target` of a layout, a variant, an instance or an include.
  - The bone names in an emitted skeleton's skin weights are its local ids.
  - Local ids are kept per document of the build, so a written id of one document that equals an emitted id of another keeps its seed.
  - PRG14 covers a repeated id within one output and a namespaced id equal to another id.
- **Kit.** sr-core's 15 SREP 87 cases and the amended SREP 66 collision case pass on this engine. Four of them are failure cases that `run.py` cannot check; they were checked by hand.

### SREP 72 follow-ups

- **PREC1.** `project/@precision="f32"` in a document with 3D content gives the information finding PREC1, once, in `validate` and in the render report. Its message starts with SREP 72's words, "3D internal buffers stay f16". It replaces the engine's E23, which said the same and had no other use. PREC1 covers every kind the 3D pass draws: `object3D`, `particles3D`, `ocean`, `blackHole` and `accretionDisk`. sr-core's erratum of 2026-10-11 adds the last two to SREP 72's list, and SREP 72 is now Final.
- **PREC10.** A device without float32 blending or filtering already failed an f32 render, naming the missing feature. The error message now starts with "PREC10:", and the render report gives it the code PREC10, also when an output layer meets it first.

### Fixes

- **References inside an include name the include's own document.**
  - A transformConstraint's `pole` resolves in the constraint's own scope, as its `target` already did.
  - A target or pole that names a bone resolves to the bone of the skeleton in that scope. Before, the rig took the first skeleton in document order that declared a bone of that id, in any document.
  - A fracture's `source` resolves in its owner's scope (E11 when it names nothing there).
  - The 3D renderer's references (a parent, a camera's target and focus target, a joint's look-at) no longer fall back to the including document's node or light of the same id.
- **A constraint target or pole that names nothing** in its scope now gives an E11 warning. A top-level constraint no longer binds to a bone inside a symbol instance or a repeat copy, whose ids are scoped to that copy.
- **Each included document reads its own parameters:** `repeat/@over`, `param:` links, text placeholders and `param()` read the included document's defaults and data sources, never the including document's.
- **CI.** The Mesa shader cache is keyed by the runner's CPU. A cache written on a runner with other instruction-set extensions made llvmpipe crash with SIGILL in encode tests.

### Changes that can alter output or verdicts from v0.4.0

- `validate` and the render report give PREC1 where v0.4.0 gave E23 (in the report, `X-rs-scene-render-E23`). `scene-render explain E23` is an unknown code (exit 2); `explain PREC1` and `explain PREC10` describe the new codes.
- A render that fails for want of float32 blending or filtering is reported as PREC10 instead of `X-rs-scene-render-RENDER`.
- Documents with programs:
  - one that v0.4.0 refused with PRG14, because two programs or a program and the document used the same id, is now valid;
  - one that writes an id equal to a namespaced id (`<shape id="p.g1">` beside a program `p` that emits `g1`) is refused with PRG14;
  - one in which two programs' namespaced ids compose to one value is refused with PRG14.
- Documents with includes, constraints, fractures or 3D references that named the including document's element of the same id now resolve to the include's own element, or report E11. Their frames can change.
- Other rendered frames do not change.

The Linux archive contains the executable, source commit identifier, capability manifest, README, license and third-party notices. Its executable comes from the successful CI run for the release commit and is checksum-verified and smoke-tested before packaging.

Verify the download with `sha256sum -c scene-render-v0.4.1-linux-x86_64.tar.gz.sha256`, extract it, then run `./scene-render --help`.

## v0.4.0 — 2026-10-11

This release vendors sr-core schema 1.6.0 byte for byte. It also carries the follow-ups that SREPs 76 to 85 ask of the engine, the version gates of SREP 83 and the value forms of SREPs 84 and 85, plus include, 2.5D, rendering and memory fixes found while testing v0.3.2.

### Schema 1.6.0, vendored byte-identical

`schema/scene-render-1.1.xsd`, `schema/scene-render-1.1.sch` and `schema/capabilities-1.schema.json` are byte-identical to sr-core's release 1.6.0 (tag `schema-1.6.0`, commit 54af5c1). `schema/UPSTREAM` gives their SHA-256 and lists the SREPs this engine implements. The capability manifest names schema 1.6.0. Earlier releases carried the 1.6 additions as local extensions with a few differences from sr-core. Those differences are gone:

- The sr-model generator reads a union that holds an anonymous member type (`tonemap/@reference`, SREP 67), so the vendored XSD needs no renamed types.
- sr-core does not declare `program/@effects`, which v0.3.0 to v0.3.2 accepted: a `program` node that writes it is refused with S04.

SREP 86 (Process) records why the gates of `blackHole`, `accretionDisk`, `camera/@geodesics`, `pyroBlast`, `voxelAsset` and voxel objects admit version 1.3 and later, while every other addition of schema 1.6.0 requires version 1.6.

### SREP follow-ups

- **Version gates (SREPs 76, 80 and 83).** BH1, VOX1 and SREP 40's gates V8, MSQ1, P3D1, OCN1, CRT1, FRX1 and GEO1 refuse versions 1.0 to 1.2 only, and their messages say "1.3 or later". In v0.3.2 they refused every version but 1.3, so no document could hold a black hole, cells, a volume, pyro, an ocean, a crater, a fracture or globe relief together with a 1.6 feature such as `viewport3D`.
- **Warning renames (SREP 76).** The black-hole warnings are W11 (a hole or disk that no geodesic camera draws) and W12 (`denoise` on a geodesic camera). In v0.3.2 they were W03 and W04, codes that SREP 57 gives to its generated-marker warnings.
- **Boolean values (SREPs 76, 77 and 84).** Rules read an `xs:boolean` attribute by its value: "true" and "1" are true, "false" and "0" are false, and surrounding whitespace is ignored. This covers BH5 to BH8, W11, W12 and W05 (`geodesics`, `denoise`), CRT11 and CRT12 (`mantle`), C15, C43, R15, R48, VOL5, C80, GEO2, PYRO9 to PYRO11 and the warning W02. Integer attributes in these rules are read whatever their sign, leading zeros or padding.
- **Decimal and double values (SREP 85).** C13, C14, C15, C37, C38, C44, C52, C53, C54, C56, PYRO1, PYRO2, PYRO4, VOL4, P3D3, P3D10, OCN3, OCN5, PYRO11, PYC6, FRX11, ITR1 and SRT1 read their numeric attributes by value, so "+1" is 1. In v0.3.2, XPath's `number()` read "+1" as NaN. Exponent forms such as `1e0` follow the native validator, which is normative for them. INF, -INF and NaN compare as the Schematron compares them.
- **Disk profile (SREP 76).** The geodesic pass divides the thin-disk profile by its exact maximum, (6/7)^(3/2) 7^(-1/4) ≈ 0.48787, instead of 0.4880.
- **Supersample filter (SREP 89).** The down filter clamps each channel to [0, max(1, M)], where M is the largest sample of non-zero weight, instead of [0, 1]. Colour finishing (exposure, looks, tone mapping) runs on the filtered frame. A frame whose samples all lie within [0, 1] gives the same bytes as before.

### Fixes

- **Includes and instances.** References written inside an included document, a symbol instance or a repeat copy resolve in that scope. Before, the 3D renderer and the rigid-body world compared them as written, so an included black hole lost its disk and its camera lost its look-at target.
- **Included definitions.** An included document's materials, material maps, effects (and their shader files), fonts and maps resolve in that document. Before, the renderer looked them up in the including document only, so an included `object3D` failed with "material not found" or took the including document's material of the same id. The assets, effects and materials an include merges animate on the include's clock, as its content does. A compute node's `src` and a transition's `shader` resolve from the directory of the document that writes them. Sections an include does not merge (lights, paints, styles, physics, audio, markers, parameters and the others) are reported as E12.
- **threeD groups.** A group, instance, include or repeat with `threeD` is placed in 2.5D as one picture, with or without effects. Before, a group without effects drew its children flat and dropped its `zDepth`, `rotationX` and `rotationY` without a warning. A `threeD` child of such a group goes through the camera once, and the group's sheet keeps what it draws up to 2048 px beyond the frame. Repeat copies are not placed one by one. In 360 renders, a group with a `threeD` of its own is placed as one picture.
- **Validation with programs.** `validate` reports errors that appear only once build-mode programs run, such as two programs emitting the same id (PRG14). Before, these documents passed validation and failed at render.
- **Bloom knee.** The soft knee of bloom and halation never reaches below black. With a threshold under 0.5, dark ground was lifted (#050505 became #1C1C1C).
- **Texel-aligned composites.** An offscreen or layer whose texels land one to one on whole target pixels is copied with `textureLoad` rather than sampled. Sampling weighed in the neighbouring texel on llvmpipe and rounded about a quarter of the edge channels one step down.
- **ISF pass buffers.** Reads at a texel centre return that texel exactly, so cellular automata and jump-flood passes do not freeze in bands or stripe as they did on llvmpipe. Sampler names are bound per scope, and a failed compile names `[GLSL-F64]` or `[GLSL-SAMPLER-ARG]` with the user's line.
- **Memory.** Decoded images unused for a few frames are evicted. Vector targets beyond a two-frame budget are rasterised just before their draw and released after it, instead of all at once for the whole frame. The text face cache keeps only faces whose font bytes are still in use.
- **Speed.** Zero-opacity leaves are skipped, adjustment layers reuse their backdrop snapshot, and parsed font faces are cached. Renderer CPU per frame fell by 20 to 23 % on a measured film, with byte-identical frames.

### Known gaps

- SREP 72's information finding PREC1 (`precision="f32"` with 3D content) is not implemented; the engine reports E23 there. sr-core 1.6.0 keeps SREP 72 Accepted for this reason.

### Changes that can alter output or verdicts from v0.3.2

- `program/@effects` is refused (S04).
- Documents of version 1.4 to 1.6 that use a black hole, cells or a construct of SREP 40 are valid; v0.3.2 refused them.
- Black-hole warnings are reported as W11 and W12 instead of W03 and W04.
- Boolean, integer, decimal and double attributes written as "1", "0", "+1", with leading zeros or with padding get the verdict of their value. Some documents v0.3.2 refused are now valid, and some it accepted are refused, for example `follow=" true "` with a closed boundary.
- `pyroSource/@end="INF"` is refused by PYRO2. INF in an ocean wavelength or whitewater window, an emitter direction or clay's boil is accepted with W01, as the Schematron accepts it; the renderer still refuses a non-finite value when it runs. Two pyros under one volume report VOL1 only.
- `validate` fails documents whose programs produce load errors (PRG14).
- Accretion disks are 0.026 % brighter at their peak.
- Supersampled frames keep values over 1 for colour finishing and are tone mapped.
- Bloom and halation with a threshold under 0.5 leave dark pixels dark.
- Texel-aligned composites and ISF pass buffer reads can differ by one step in some channels.
- Black holes, disks and camera targets inside includes, instances and repeats are drawn.
- Included documents use their own materials, effects, fonts and maps, and their merged definitions animate on the include's clock. A transition `matte` or a skin's `skeleton` written inside a symbol instance or a repeat copy names the node of its own scope first. Unmerged sections of an included document are reported as E12.
- Groups with `threeD` are placed in 2.5D whether or not they carry effects, and their `threeD` children are projected once.

The Linux archive contains the executable, source commit identifier, capability manifest, README, license and third-party notices. Its executable comes from the successful CI run for the release commit and is checksum-verified and smoke-tested before packaging.

Verify the download with `sha256sum -c scene-render-v0.4.0-linux-x86_64.tar.gz.sha256`, extract it, then run `./scene-render --help`.

## v0.3.2 — 2026-10-10

Three scene features from SREPs 71, 74 and 75, sound across transitions (SREP 82), and fixes for defects found while testing v0.3.1.

### Scene features (document version 1.6 where gated)

- **SREP 71, materials and sky.** It adds a subsurface lobe (Christensen–Burley profile, pre-integrated over curvature), a procedural gradient sky with a sun on dome lights, and material maps fed by generator or image assets. The new rules are SKY1, SKY2 and MTX1-*. A visible procedural sky that no 3D pass draws takes the background's place.
- **SREP 74, viewport3D.** A 3D scene with its own camera, lights and background is placed as a node in the 2D composition. Its rules are V15, VP1 and VP3. The black-hole rules BH5–BH8 and W03 now apply within the camera's own viewport, or within the document outside every viewport.
- **SREP 75, fractal deep zoom.** It adds Mandelbrot and Julia fractal assets, with rules V16 and FRC1. The CPU uses direct f64 at shallow zooms and switches to perturbation, with rebasing and an extended exponent, at deep ones. Centres are exact decimals of any length. Validators built on libxml2 older than 2.13.0 refuse decimals of more than 24 digits.

The schema editor accepted these SREPs on 2026-10-10; they enter sr-core with schema 1.6.0. Until then the engine's vendored schema carries them as local extensions (see `schema/UPSTREAM`).

### Sound across transitions (SREP 82)

A transition now mixes the sound of the video layers on its sides, including layers inside a side, by its `audio` attribute, at the eased progress p:
- `crossfade` (the default) gives the outgoing side 1 − p and the incoming side p;
- `equal-power` gives cos(πp/2) and sin(πp/2);
- `cut` switches from one side to the other where p reaches ½;
- `none` leaves both sides at their own volume.

Sequence junctions crossfade, except that a `cut` junction cuts its sound with its picture. The gains multiply the layers' volume automation, one value per frame, interpolated between frames. Before this release `audio` was read by no code, and both sides played at full volume over the whole window.

### Fixes

- **Prewarm and stepsPerFrame memory.** A frame that takes several steps now runs them as one looped pass over two textures, as `iterate` does. Before, every step kept a new texture until the frame was submitted. At 1,000 prewarm steps (1280×720), peak memory fell from 4,088 MB to 258 MB; it is 278 MB at 6,000 steps. The pixels are byte-identical.
- **Odd frame sizes.** New error OUT1: H.264 and H.265 outputs whose frame the chroma format halves must have even dimensions. Validate reports it with the even size to set, and `encode -o` refuses it before rendering. An odd-sized AV1 frame is encoded with libaom.
- **Silent 3D cases.** Two new render-report warnings:
  - `3D-OFF-CAMERA`, for an object3D outside the view in every frame it is drawn;
  - `3D-METAL-NO-ENVIRONMENT`, for metallic 0.5 or more with no environment image.

  As warnings, they fail `encode --strict`.
- **Morph transition.** The luminance displacement described in the README is restored; a rewrite of the transition shader had reduced morph to a crossfade.
- **`--parallel auto`.** The split now depends only on the programme: one chunk per 10 s, at most 12, and one stream under 20 s. The host decides only how many chunks render at once, so the same scene splits the same way on any machine.
  - Auto renders of 20 s or more on software adapters are now joined chunks rather than one stream, and their bytes differ from v0.3.1.
  - On a 40 s scene the joins cost 0.8–1.7 dB PSNR on single frames, within the stream's own frame-to-frame variation, with no visible change.
- **README.** It adds a "Document version 1.6" section covering programs, compute, tonemap, iterate, stepping, parametric, pixel-exact and flock options, with examples that validate.

### Changes that can alter output from v0.3.1

- Morph transitions draw the luminance displacement instead of a crossfade.
- Transitions and sequence junctions between video layers with sound now mix that sound by `audio`; with the default `crossfade` each side is 6 dB quieter at the middle of the window than in v0.3.1.
- Auto-parallel encodes of 20 s or more on software adapters split into chunks.
- Films with off-camera objects or metals without an environment image now fail `encode --strict`.

The Linux archive contains the executable, source commit identifier, capability manifest, README, license and third-party notices. Its executable comes from the successful CI run for the release commit and is checksum-verified and smoke-tested before packaging.

Verify the download with `sha256sum -c scene-render-v0.3.2-linux-x86_64.tar.gz.sha256`, extract it, then run `./scene-render --help`.

## v0.3.1 — 2026-10-10

Scenes with up to four ambient or directional lights use specialized shader pipelines that preserve light order and rendered output. Larger light lists and local lights retain the general lighting path.

The optimization measured 6.7% and 10.9% higher frame throughput in two 24-frame Inova benchmark windows against v0.3.0. These are short-window measurements, not a full-package speedup claim. Two encoded clips matched byte for byte, including decoded video, audio and diagnostics. The GPU oracle checks all 31 light arrangements through four lights, opaque and blended materials, and fallback paths.

The Linux archive contains the executable, source commit identifier, capability manifest, README, license and third-party notices. Its executable comes from the successful CI run for the release commit and is checksum-verified and smoke-tested before packaging.

Verify the download with `sha256sum -c scene-render-v0.3.1-linux-x86_64.tar.gz.sha256`, extract it, then run `./scene-render --help`.

## v0.3.0 — 2026-10-10

This release adds scene document version 1.6 and integrates the following rendering features:

- Build-time and stepping WebAssembly programs with deterministic host functions, fuel limits and pinned output hashes.
- Compute histograms, density tonemapping, iterated effects, error diffusion and segmented sorting.
- Node-local shader stepping, prewarming and checkpoints.
- Parametric paths, parametric surfaces and heightfields.
- Shader content rectangles, supersampling, per-node edge blending and optional float32 working textures.
- Float32 working precision is preserved through CPU effects and checkpoint restores, with format conversion for output joins.
- Upright flock sprites through `orientToVelocity="false"`.
- Faster bounded texture-coordinate conversion in the depth-of-field shader, checked against the original shader.

The schema retains the vendored sr-core 1.5.0 base with the local 1.6 extensions documented in `schema/UPSTREAM`. The compute histogram carry tests use llvmpipe; discrete-GPU carry remains unverified.

The Linux archive contains the executable, source commit identifier, capability manifest, README, license and third-party notices. Its executable comes from the successful CI run for the release commit and is checksum-verified and smoke-tested before packaging.

Verify the download with `sha256sum -c scene-render-v0.3.0-linux-x86_64.tar.gz.sha256`, extract it, then run `./scene-render --help`.

## v0.2.1 — 2026-10-09

This patch release fixes scene evaluation, asset resolution and encode finalization.

- Motion paths now update camera and object3D positions. Camera paths with unsupported automatic orientation report a validation error.
- Rigid bodies in groups that start later enter the simulation when their group starts. Late-starting soft bodies report that they are unsupported.
- Flock sprites resolve their image assets without needing a separate image layer.
- Software H.264 and H.265 encoders use fixed thread counts so CPU core counts do not change their output. Encoded bytes can differ from earlier releases.
- Contrast analysis shares a bounded probe budget per output and runs after the encoded file is closed.
- Fuzz crash reports identify panicking worker locations and cover nonfinite ocean body inputs.

The Linux archive contains the executable, source commit identifier, capability manifest, README, license and third-party notices. Its executable is taken from the successful CI run for the release commit, checksum-verified and smoke-tested before packaging.

Verify the download with `sha256sum -c scene-render-v0.2.1-linux-x86_64.tar.gz.sha256`, extract it, then run `./scene-render --help`.

Requirements: Linux x86-64, glibc 2.39 or newer, Vulkan (including Mesa software Vulkan), FFmpeg and ffprobe 5.1 or newer, and suitable fonts. On Ubuntu 24.04: `sudo apt install ffmpeg libvulkan1 mesa-vulkan-drivers fonts-dejavu-core`. HEIC decoding additionally requires `libheif-examples libheif-plugin-libde265`.

Windows and macOS are checked for portability; rendering equivalence across different CPUs or GPU drivers is not guaranteed. Multiple scattering is bounded by the requested collision limit and the volume quadrature resolution; its indirect walk terminates at surfaces rather than adding surface-volume interreflection.

## v0.2.0 — 2026-10-09

This release integrates the outstanding scene-format implementations and cinematic rendering work, with fixes for nonfinite physics inputs and path-traced water.

- Repeat copies can use generated points; connectors follow their endpoints; PDF assets and text-anchored regions are supported in the packaged executable.
- Render reports include accessibility findings, legibility checks, pinned-font findings and capability notices. Caption layouts can be exported with the captions dump command.
- Text uses consistent line seating, rectangle corner radii follow the format contract, selective-color strength mixes display-encoded values, and missing animation blend targets use the rest pose.
- Voxel bodies can fracture under stress, and crater placement accounts for curved terrain. The ocean solver includes an absorbing boundary mode.
- Volumes support bounded multiple scattering in the path tracer. One scattering bounce remains the default; zero retains absorption and emission without scattering.
- Schema support includes generated marker references and the 1.5 reference checks and conditional capability manifest format, while retaining the documented cinematic extensions.
- Rendering reuses contrast work and specializes raster passes to reduce repeated work.

The Linux archive contains the executable, source commit identifier, capability manifest, README, license and third-party notices. Its executable is taken from the successful CI run for the release commit, checksum-verified and smoke-tested before packaging.

Verify the download with `sha256sum -c scene-render-v0.2.0-linux-x86_64.tar.gz.sha256`, extract it, then run `./scene-render --help`.

Requirements: Linux x86-64, glibc 2.39 or newer, Vulkan (including Mesa software Vulkan), FFmpeg and ffprobe 5.1 or newer, and suitable fonts. On Ubuntu 24.04: `sudo apt install ffmpeg libvulkan1 mesa-vulkan-drivers fonts-dejavu-core`. HEIC decoding additionally requires `libheif-examples libheif-plugin-libde265`.

Windows and macOS are checked for portability; rendering equivalence across different CPUs or GPU drivers is not guaranteed. Multiple scattering is bounded by the requested collision limit and the volume quadrature resolution; its indirect walk terminates at surfaces rather than adding surface-volume interreflection.

## v0.1.4 — 2026-10-06

Reliability fixes for incremental rendering, volume loading, media metadata, delivery and data parsing.

- Delivery URLs escape filenames containing spaces, punctuation and Unicode.
- Adjacent baked volume frames share decoded payloads; incremental rendering tracks baked payload changes and missing files.
- Spherical MP4 metadata rejects invalid extended sizes, removes obsolete projection metadata and preserves permissions through secure temporary files.
- AV1 two-pass encoding verifies SVT-AV1 statistics support and falls back to libaom-av1 when the installed FFmpeg build ignores pass flags. Builds with neither implementation report an error.
- HEIF colour detection respects the primary image property associations.
- PMTiles rejects oversized sections before allocating memory.
- CSV and TSV preserve quoted empty records while skipping blank physical lines.

The reliability fixes include regressions that failed before their implementation changes and passed afterward. Encoder checks include both modern and older FFmpeg builds. The release workflow runs workspace tests, Clippy, portability checks, evidence and performance checks before packaging and smoke-testing the validated binary. Numerical tests use optimized builds with debug assertions and overflow checks enabled. The smoke and fuzz checks reuse the same binaries built by the evidence job.

This release also includes the mesh, path tracing, simulation and cinematic rendering changes accumulated since v0.1.2. These features require scene-specific visual review; this release does not establish production readiness for every cinematic workflow.

Download the Linux x86-64 archive and checksum. Run `sha256sum -c scene-render-v0.1.4-linux-x86_64.tar.gz.sha256`, extract the archive, then run `./scene-render --help`. The archive includes the binary, README, license and source commit identifier.

The binary requires Linux x86-64 with glibc 2.39 or newer, a Vulkan driver, FFmpeg and ffprobe 5.1 or newer, and suitable fonts. Mesa software Vulkan is supported. Ubuntu 24.04 dependencies: `sudo apt install ffmpeg libvulkan1 mesa-vulkan-drivers fonts-dejavu-core`. HEIC decoding additionally requires `libheif-examples libheif-plugin-libde265`.

The distributed binary targets Linux x86-64. Windows and macOS portability checks do not establish rendering equivalence on those platforms. Evidence probes establish technical behavior; artistic quality still requires visual and temporal review.

### Transmission rendering

Image changes in the path tracer for scenes with transmissive materials (`transmission` above 0); scenes without one render the same bytes at the same speed.

- Surfaces seen through water or glass now receive the analytic lights: the shadow ray is refracted at the interface instead of being blocked by it, with Fresnel transmittance and absorption. Before, a floor under water got no direct light at all. In `examples/cinematic-impact/impact.scene.xml` the mean brightness at t = 2.0 goes from 0.00663 to 0.00909 (plume and ejecta removed for the measurement). Shadows that glass casts on surfaces in air stay black.
- `attenuationColor` and `attenuationDistance` now take effect in path-traced renders (they already did in raster): after one attenuation distance the light left is the colour, per channel.
- Glass or water over nothing now shows the visible dome where it showed black, including the dome's reflection on water at the horizon.
- The ocean's default spray is transmissive, so frames with whitewater change slightly around the spray (a UHD hero frame: 394 pixels, at most 28 code values).
- Limits: a camera that starts under the water does not see the sun on the floor, caustics are not produced, and very steep waves can leave samples dark. See the cinematic impact SREP, "Light through water and glass".

## v0.1.3 — 2026-10-06

Reliability fixes for incremental rendering, volume loading, media metadata, delivery and data parsing.

- Delivery URLs escape filenames containing spaces, punctuation and Unicode.
- Adjacent baked volume frames share decoded payloads; incremental rendering tracks baked payload changes and missing files.
- Spherical MP4 metadata rejects invalid extended sizes, removes obsolete projection metadata and preserves permissions through secure temporary files.
- SVT-AV1 encoding supplies two-pass statistics options.
- HEIF colour detection respects the primary image property associations.
- PMTiles rejects oversized sections before allocating memory.
- CSV and TSV preserve quoted empty records while skipping blank physical lines.

Each of these ten fixes includes a regression that failed before the implementation change and passed afterward. Targeted Rust tests and formatting checks passed locally. The release workflow runs workspace tests, Clippy, portability checks, evidence and performance checks before building and smoke-testing the binary.

This release also includes the mesh, path tracing, simulation and cinematic rendering changes accumulated since v0.1.2. These features require scene-specific visual review; this release does not establish production readiness for every cinematic workflow.

Download the Linux x86-64 archive and checksum. Run `sha256sum -c scene-render-v0.1.3-linux-x86_64.tar.gz.sha256`, extract the archive, then run `./scene-render --help`. The archive includes the binary, README, license and source commit identifier.

The binary requires Linux x86-64 with glibc 2.39 or newer, a Vulkan driver, FFmpeg and ffprobe 5.1 or newer, and suitable fonts. Mesa software Vulkan is supported. Ubuntu 24.04 dependencies: `sudo apt install ffmpeg libvulkan1 mesa-vulkan-drivers fonts-dejavu-core`. HEIC decoding additionally requires `libheif-examples libheif-plugin-libde265`.

The distributed binary targets Linux x86-64. Windows and macOS portability checks do not establish rendering equivalence on those platforms. Evidence probes establish technical behavior; artistic quality still requires visual and temporal review.

### Transmission rendering

Image changes in the path tracer for scenes with transmissive materials (`transmission` above 0); scenes without one render the same bytes at the same speed.

- Surfaces seen through water or glass now receive the analytic lights: the shadow ray is refracted at the interface instead of being blocked by it, with Fresnel transmittance and absorption. Before, a floor under water got no direct light at all. In `examples/cinematic-impact/impact.scene.xml` the mean brightness at t = 2.0 goes from 0.00663 to 0.00909 (plume and ejecta removed for the measurement). Shadows that glass casts on surfaces in air stay black.
- `attenuationColor` and `attenuationDistance` now take effect in path-traced renders (they already did in raster): after one attenuation distance the light left is the colour, per channel.
- Glass or water over nothing now shows the visible dome where it showed black, including the dome's reflection on water at the horizon.
- The ocean's default spray is transmissive, so frames with whitewater change slightly around the spray (a UHD hero frame: 394 pixels, at most 28 code values).
- Limits: a camera that starts under the water does not see the sun on the floor, caustics are not produced, and very steep waves can leave samples dark. See the cinematic impact SREP, "Light through water and glass".

## v0.1.2 — 2026-09-30

Linux x86-64 update with quality tiers, incremental rendering, watch mode, GPU timing and fixes for rendering correctness and accessibility.

- `watch` reloads changed image files, and `--changed-only` includes active transition types, parameters and static attributes in frame fingerprints.
- Incremental renders retain strict validation: frames with unsupported content are rendered again so shader failures cannot disappear on a subsequent run.
- Effect caches follow animated generator paints, including `paint2` and paint references nested inside assets.
- Wide strokes, outlines and matte-choke preserve partial alpha. Wide gathers can cost more at large radii on translucent inputs; their cost grows with radius.
- Caption burn-in and authored text layers have distinct contrast identities. A layer named `captions` receives normal contrast checks, and text covered by later layers is measured in the final picture.
- Linux CI installs the HEVC decoder required by HEIC fixtures and checks decoding before building.
- Quality tiers, incremental rendering, watch mode, GPU timing and performance comparisons are included from the changes since v0.1.1.

Download the `linux-x86_64.tar.gz` archive and its `.sha256` file. Verify it with `sha256sum -c scene-render-v0.1.2-linux-x86_64.tar.gz.sha256`, extract it, and run `./scene-render --help` from the extracted directory. The archive contains the binary, README, license, and source commit identifier.

The binary is built on Ubuntu 24.04 and requires Linux x86-64 with glibc 2.39 or newer. Rendering requires a Vulkan driver (Mesa software Vulkan also works); media decoding and encoding require FFmpeg and ffprobe 5.1 or newer. Install fonts appropriate to your scenes. On Ubuntu 24.04: `sudo apt install ffmpeg libvulkan1 mesa-vulkan-drivers fonts-dejavu-core`. HEIC decoding also requires `libheif-examples libheif-plugin-libde265`.

Validation before the version bump: 637 Rust tests passed, with 4 benchmarks ignored, and 14 Python tests passed. Seven new regression tests failed before their fixes and passed afterward. Formatting, Clippy and golden comparisons passed without replacing references. The `still-formats` evidence case passed locally, including HEIC decoding. The release workflow smoke-tests validation, PNG rendering and a two-frame FFV1 encode using the release binary.

The distributed binary targets Linux x86-64. Windows and macOS portability checks do not establish rendering equivalence on those platforms. Evidence probes establish technical behavior; artistic quality still requires visual and temporal review.

## v0.1.1 — 2026-09-30

Linux x86-64 update with rendering performance improvements, stronger visual checks and fixes for portability, evidence reports and accessibility.

- Executable discovery supports Windows `PATHEXT` and preserves Unix execute-permission checks. Portability CI compiles all targets and runs provider unit tests.
- Evidence clips use absolute output paths and must exist before the report links them. Renderer commands found through `PATH` can be hashed and reported.
- Text matching its backdrop reports 1:1 contrast when rendered coverage is present, including isolated groups, overlays and captions. Transparent, covered, offscreen and empty text remain excluded.
- Image-size validation accounts for EXIF orientation, and benchmark renders propagate errors during warm-up and measurement.
- Rendering selects hardware adapters ahead of software adapters, bounds and reuses effect targets, and reduces redundant passes. Particle bursts follow composition time.
- Missing goldens fail instead of being accepted automatically. CI runs evidence probes and complete review clips independently from the test, performance and oracle jobs.

Download the `linux-x86_64.tar.gz` archive and its `.sha256` file. Verify it with `sha256sum -c scene-render-v0.1.1-linux-x86_64.tar.gz.sha256`, extract it, and run `./scene-render --help` from the extracted directory. The archive contains the binary, README, license, and source commit identifier.

The binary is built on Ubuntu 24.04 and requires Linux x86-64 with glibc 2.39 or newer. Rendering requires a Vulkan driver (Mesa software Vulkan also works); media decoding and encoding require FFmpeg and ffprobe 5.1 or newer. Install fonts appropriate to your scenes. On Ubuntu 24.04: `sudo apt install ffmpeg libvulkan1 mesa-vulkan-drivers fonts-dejavu-core`.

Validation before the version bump: 615 Rust tests passed, with 4 ignored, and 6 Python tests passed. Formatting, Clippy and golden comparisons passed. All 24 evidence cases matched their expected outcomes (22 native, 2 degraded); both review clips were verified as 3-second, 640×360, 24 fps videos with valid report links. The release workflow smoke-tests validation, PNG rendering and a two-frame FFV1 encode using the packaged binary.

The distributed binary targets Linux x86-64. Windows and macOS portability checks do not establish rendering equivalence on those platforms. Evidence probes establish technical behavior; artistic quality still requires visual and temporal review.

## v0.1.0 — 2026-09-30

First tagged Linux x86-64 build of rs-scene-render.

This release includes fixes for parallel delivery diagnostics, marker-relative remap timing, overlay and output-caption contrast checks, and required-caption validation. The preceding delivery fixes cover strict template diagnostics, poster composition, repeated captions through remaps, and overlay asset representations.

Download the `linux-x86_64.tar.gz` archive and its `.sha256` file. Verify it with `sha256sum -c scene-render-v0.1.0-linux-x86_64.tar.gz.sha256`, extract it, and run `./scene-render --help` from the extracted directory. The archive contains the binary, README, license, and source commit identifier.

The binary is built on Ubuntu 24.04 and requires Linux x86-64 with glibc 2.39 or newer. Rendering requires a Vulkan driver (Mesa software Vulkan also works); media decoding and encoding require FFmpeg and ffprobe 5.1 or newer. Install fonts appropriate to your scenes. On Ubuntu 24.04: `sudo apt install ffmpeg libvulkan1 mesa-vulkan-drivers fonts-dejavu-core`.

Validation: 120 delivery, CLI, and evaluator tests passed locally, along with formatting, Clippy, and release hygiene checks. The release workflow smoke-tests validation, PNG rendering, and a two-frame FFV1 encode using the packaged binary.

Known CI limitation: the existing workspace CI failed its GPU golden-image comparison before this release work (frame 0 maximum delta E 1.1755). The full workspace suite is not claimed to pass; the release build and its smoke tests are reported separately.
