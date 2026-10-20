# scene-render v0.6.0

This release draws 3D objects on OpenGL, gives every command a versioned JSON output and a read-only MCP server, adds image scopes and fuller loudness measurement, implements schema attributes that earlier versions declared but did not apply, and turns malformed or oversized input into reported errors instead of panics. The vendored schema is still sr-core 1.7.0.

## OpenGL

- The 3D pass draws on OpenGL as on Vulkan, Metal and DirectX 12. A document with 3D objects now uses the best-ranked adapter whatever its backend; before, it moved to a Vulkan adapter by default and failed with exit status 2 when OpenGL was named.
- Opening an OpenGL device refuses a driver older than OpenGL 4.3 or OpenGL ES 3.1, and an adapter that cannot give the 3D pass's colour and depth formats 4x multisampling, at device creation rather than at the first frame.
- The vendored wgpu-hal asks EGL for desktop OpenGL 4.6, 4.5, 4.4, 4.3 and then 3.3, newest first, so the NVIDIA driver, which answers a 3.3 request with exactly 3.3, gives a context new enough for the shaders.

## Command line

- Every command accepts `--format json` (or `--json`) and prints one envelope, `scene-render-cli/1`, with `schema_version` 1.1.0. `crates/scene-render/schema/cli-output-1.schema.json` describes it and the README lists each command's result.
- `scene-render mcp` serves `validate`, `inspect`, `eval`, `captions`, `capabilities` and `explain` to a Model Context Protocol client over standard input and output. Each tool returns the command's JSON envelope. Messages and arguments are parsed into typed requests that refuse unknown keys; a message holds at most 1 MiB and a session at most 100,000 messages.
- `scene-render scopes FILE` measures clipping, legal range and saturation on a still or on every `--step`-th frame of a video, as text or JSON; `--chroma-limit` sets the oversaturation threshold. The production gate runs it on candidates and fails them past `--max-illegal`, `--max-out-of-gamut` and `--max-oversaturated`.
- A panic is reported as one `internal error: MESSAGE at FILE:LINE` line and exits with status 2.

## Audio measurement

- The loudness meter gives loudness range (EBU Tech 3342) and the momentary (400 ms) and short-term (3 s) maxima beside integrated loudness and true peak. The delivery report records them, and `encode` prints them on its audio line.
- True peak and the true-peak limiter oversample to at least 192 kHz: 8x at 32 and 44.1 kHz, 4x from 48 kHz up as before. At 44.1 kHz true peak can read up to about 0.2 dB higher than in v0.5.0.
- The momentary windows and the gating blocks step by the same rounded 100 ms, so the momentary maximum at sample rates such as 11025 Hz comes from the windows momentary reports.

## Declared attributes now implemented

- Mesh-gradient points now honour animated box-relative `x` and `y` coordinates, defaulting independently to `col/(cols-1)` and `row/(rows-1)`. Geometry and colour share the Catmull–Rom surface; absent coordinates preserve existing pixels.
- Inline vector assets (primitives and paths) now honour `strokePosition` and `paintOrder` through the shape stroke implementation. No existing corpus, example or golden vector asset sets these attributes.
- Curved connectors use the shared SREP 58 32-sample length table and evaluate outline cuts, labels, gaps and trimmed endpoints on the true cubic. Curved output can change away from the symmetric apex, including `tests/corpus/valid/connector-points.scene.xml`; straight and orthogonal routes retain their geometry.
- Trim shape modifiers default to simultaneous, matching `shape/@trimMode`.
- Mesh-warp and soft-body grid dimensions share the schema's defensive 2–16 clamp before allocation.
- Text animator `easeHigh` and `easeLow` independently ease the high and low ends of range selection. `rotationX`, `rotationY`, and `zDepth` project complete glyphs through the layer's 3D camera, or the default 60-degree camera for ordinary 2D text. Authored easing and glyph depth/rotation can intentionally change output.
- Caption emphasis uses the active colour for karaoke/highlight/pop/enlarge and bold weight for other presets. WebVTT cue settings round-trip; percentage `line` and `position` place burned captions, and unsupported settings produce render notes.
- Caption tracks stack by `z` among top-level composition entries (default 1000000), preserving track document order on ties. Groups remain compositing units.
- Nodes, symbols, text animators and puppet pins share human labels in diagnostics, findings and checkpoint companion metadata: nonblank name, else id, else element index. IDs and resume keys remain stable. Inspect lists symbol labels. The JSON envelope is `schema_version` 1.1.0: a diagnostic may carry `label`.
- Lighting implements `falloff`: linear, quadratic, smooth (the schema default) and none. Omitted falloff now uses smoothstep instead of the previous fixed quadratic attenuation; use explicit quadratic to retain that curve.
- Pixel motion blur declares its amount and sample controls and respects the quality-tier sample budget (8 in draft), retaining the existing 64-tap maximum.
- Matte-choke implements position: positive amounts erode inside, dilate outside (the schema default), or average both results at center. Negative amounts reverse inside/outside. Existing sign-only chokes can retain their former erosion/dilation direction by setting position to inside.
- Stroke and outline implement softness with the schema default 0.1, including when omitted. At ordinary scale that default stays below the one-pixel blur threshold; scaled nodes can now have softer edges. Their declaration lists and matte-choke’s list advertise only controls the kernels use.
- De-esser gain supplies post-processing make-up gain. Limiter gain drives the input before limiting so the documented threshold remains the processed signal’s ceiling. Zero gain retains both filters’ previous processing.
- Ocean gravity is consistently m/s²: solver and default wave speed convert through `pixelsPerMeter` and the ocean's world scale; impact cavities use physical gravity. Ocean motion changes when world units differ from metres.
- Particle and flock sprites accept image sequences (looped by particle age), videos and generators (layer time), with sequence missing-frame policies and particle blend order preserved.
- `scene-render params FILE` prints parameter metadata and variant labels as JSON. Parameter diagnostics include labels alongside stable ids.
- Unknown chart kinds produce render errors instead of column charts. Stepping programs require explicit dimensions. Both fracture paths share the documented 0.3 energy default; the existing 2D mass floor remains `1e-6` kg.
- Render-report asset records carry the licence and credit of every asset, generated media included. Video layer records include source in/out timecodes at the asset frame rate, with drop-frame labels preserved.
- Image, video, audio and mesh proxies are selected through the representation resolver. Explicit names win; audio and image/video representations can also be selected by language tag, then primary language subtag.
- Representation dimensions and aspect ratios are checked (A08), as are declared audio duration, sample rate and channel count (A09). Layout retains the original asset dimensions; video display width includes its pixel aspect ratio.
- An audio track without a language takes its asset's, or the selected representation's, when tagging the encoded stream; audible video layers contribute theirs. Several languages remain `mul` and none `und`.
- Tracking-kind/data mismatches, and camera tracks used by 2D constraints or stabilisation, are rejected with E17. Planar tracks continue to supply their centre, rotation and scale to 2D constraints.

## Input bounds and errors

- A scene document is held to 64 MiB, 4,000,000 XML nodes and 256 levels of nesting. Deeper nesting is S12; the other limits are XML errors. Program outputs re-parsed in place are held to the same bounds.
- Includes are keyed by canonical path: a cycle is one E13 at its first repeat, and includes nested past the include-depth limit are E13 with their own message.
- LUT sizes are bounded to 2–256 a side for 3D tables and 2–65536 entries for 1D tables, with checked products, for `.cube`, CLF and OCIO bakes. IES files with impossible angle counts, out-of-order or non-finite angles, or negative or non-finite candela are errors.
- A schema default that does not parse as its attribute's type is a model error naming the element, and a timecode whose frame number overflows is a value error. A panic in delivery's look-ahead evaluator is a delivery error.

## Changes that can alter output or verdicts from v0.5.0

- 3D LUTs are interpolated in f32 in the shader instead of through the hardware filter, which keeps as few as 4 fractional bits under Vulkan. Graded colours can change by several code values; an identity table now leaves every colour unchanged.
- Documents with 3D objects can render on an OpenGL adapter that v0.5.0 would not choose.
- True peak at 32 and 44.1 kHz, and the momentary maximum at rates whose 100 ms is not a quarter of the rounded 400 ms, can differ from v0.5.0.
- Documents past the size, node or nesting bounds, self-including documents, and out-of-range LUT or IES tables now fail validation instead of being expanded or crashing.
- The attributes listed above under *Declared attributes now implemented* change output where documents set them, and in the cases their entries name, where they are omitted.

## Licence

- scene-render is licensed under the PolyForm Noncommercial License 1.0.0, as from 0.5.0 (`LICENSE`, `NOTICE`). Commercial use needs a separate licence (`COMMERCIAL.md`).

The Linux archive contains the executable, source commit identifier, capability manifest, README, changelog, disclaimer, licence, notice, commercial-licence page and third-party notices. Its executable comes from the successful CI run for the release commit and is checksum-verified and smoke-tested before packaging.

Verify the download with `sha256sum -c scene-render-v0.6.0-linux-x86_64.tar.gz.sha256`, extract it, then run `./scene-render --help`.

## Unreleased

- The path tracer's denoiser divides each pixel by its coverage and first-hit albedo before filtering and multiplies it back after, and sets how far a tap may differ in luminance from the variance of each pixel's own samples. Against a 4096-sample reference it halves the error at 8 samples while keeping fine albedo texture (the previous filter kept 46 % of it and its error exceeded the noisy frame's), no longer raises the error of an already clean 256-sample frame (previously 5.7 times), and leaves silhouette coverage as traced (previously 1.6 times the noisy error). `denoise` defaults to true, so path-traced output intentionally changes; set `denoise="false"` for the raw estimate.
- `render --path-noise REL` and `encode --path-noise REL` stop tracing a path-traced pixel once, from 16 samples on, its luminance and coverage are within REL relative standard error of their means; `pathSamples` caps the rest. A stopped pixel keeps its mean, so denoised and tiled output match a full render's statistics. `render --stats` reports `paths` with the `traced` samples against the `budget`. Without the flag rendering is unchanged.
- W15 warns when a `renderer="pathtrace"` camera traces more than 1920 × 1080 × 256 × 4 path segments per frame: its frame's width × height × `pathSamples` × `maxBounces`, the frame being its `viewport3D`'s or the project's. 320 samples at 20 bounces in 1080p is 6.25 times that.
- On NVIDIA's OpenGL driver, which inlines every function call, the path tracer's media shaders compile in bounded memory. A lit medium gathers its in-scattered light in one loop over lights, crossed domains and surfaces instead of marching a shadow ray inside every step of the transport march, and the shader specializer removes code under overrides that are false. Before, compiling a frame with lit smoke or gas could exhaust memory. Images do not change beyond rounding.
- Where the adapter offers hardware ray queries, the path tracer finds its hits through acceleration structures the driver builds for each pass, on the GPU's ray-tracing units where it has them, and otherwise walks its own BVH as before. Both trace the same paths with the same random numbers, so images agree up to rays whose nearest hit differs by rounding. `SR_GPU_RAY_QUERY=0` keeps the BVH walk. `gpus` reports `ray_query` for each adapter and `render --stats` reports `hardware_rays` under `paths`. The JSON envelope is `schema_version` 1.2.0.
- `inspect` lists each path-traced camera's frame size, samples, bounces and path segments per frame, marking those W15 warns about; in JSON they are `path_costs`.
- `encode --output ID -o PATH` writes that output to PATH with every setting it declares. Before, `-o` silently replaced the named output with an ad-hoc one at the project size with default settings. PATH must keep the output's file type; several `--output` ids, an unknown id, or `--codec` with `--output` are usage errors (exit status 2).
- `examples/glass-of-water/` drops a drop into a glass of water with native elements only: transmissive glass and water materials under the path tracer, an ocean bounded by the glass that the drop's `waterImpulse` drives, and the ocean's `whitewater` spray as the splash. A test holds it to those elements.
- Instances honor `clipOut` for their playback span and automatic end, and `fit`/`boxWidth`/`boxHeight` for symbol placement, including cover clipping and a blurred cover behind contain-blur.
- Physical cameras use the sensor height for frames narrower than the sensor aspect, including depth of field. With default sensor dimensions, square and portrait scenes using `focalLength` intentionally change; wider frames retain the horizontal gate. Geodesic `pathSamples` above 4096 now emit W14 before clamping.
- `antialias3d` selects 1 (default), 2 or 4 raster samples, rounding 3 up to 4. Unsupported counts fall back with a warning. Previously the renderer always used four samples: default 3D edge coverage therefore intentionally changes; set 4 to retain that sampling.
- Plain fit output reframing honors layout and segment focus, including animated segment focus.
- A voxels object whose `palette` lists fewer materials than the indices its cells use, and that has no `material` of its own, colours those cells from the `.vox` file, as the schema states. Before, the frame failed with "the palette index N has no material" although the file has colours; `tests/corpus/valid/voxels.scene.xml` now renders.
- Medium albedo defaults to white in the volume library and renderer, using the library default as the renderer fallback.
- NaN opacity from animation keys becomes zero before inheritance, and repeat-copy opacity factors are clamped to 0..1. INERT-I17 identifies 2.5D attributes without threeD and alignment attributes without alignment; centered alignment still ignores margin.
