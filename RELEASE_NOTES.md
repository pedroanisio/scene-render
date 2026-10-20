# scene-render v0.7.0

This release lets the path tracer stop tracing pixels whose mean has converged, denoises lighting apart from surface colour, finds its hits with hardware ray queries where the adapter offers them, and compiles its media shaders on NVIDIA's OpenGL driver. Validation warns about, and `inspect` reports, the cost of each path-traced camera, and more of the declared 3D and layout attributes are implemented. The vendored schema is still sr-core 1.7.0.

## Path tracer

- `render --path-noise REL` and `encode --path-noise REL` stop tracing a path-traced pixel once, from 16 samples on, its luminance and coverage are within REL relative standard error of their means; `pathSamples` caps the rest. A stopped pixel keeps its mean, so denoised and tiled output match a full render's statistics. `render --stats` reports `paths` with the `traced` samples against the `budget`. Without the flag rendering is unchanged.
- The path tracer's denoiser divides each pixel by its coverage and first-hit albedo before filtering and multiplies it back after, and sets how far a tap may differ in luminance from the variance of each pixel's own samples. Against a 4096-sample reference it halves the error at 8 samples while keeping fine albedo texture (the previous filter kept 46 % of it and its error exceeded the noisy frame's), no longer raises the error of an already clean 256-sample frame (previously 5.7 times), and leaves silhouette coverage as traced (previously 1.6 times the noisy error). `denoise` defaults to true, so path-traced output intentionally changes; set `denoise="false"` for the raw estimate.
- Where the adapter offers hardware ray queries, the path tracer finds its hits through acceleration structures the driver builds for each pass, on the GPU's ray-tracing units where it has them, and otherwise walks its own BVH as before. Both trace the same paths with the same random numbers, so images agree up to rays whose nearest hit differs by rounding. `SR_GPU_RAY_QUERY=0` keeps the BVH walk. `gpus` reports `ray_query` for each adapter and `render --stats` reports `hardware_rays` under `paths`. The JSON envelope is `schema_version` 1.2.0.
- On NVIDIA's OpenGL driver, which inlines every function call, the path tracer's media shaders compile in bounded memory. A lit medium gathers its in-scattered light in one loop over lights, crossed domains and surfaces instead of marching a shadow ray inside every step of the transport march, and the shader specializer removes code under overrides that are false. Before, compiling a frame with lit smoke or gas could exhaust memory. Images do not change beyond rounding.

## Path-traced cost

- W15 warns when a `renderer="pathtrace"` camera traces more than 1920 × 1080 × 256 × 4 path segments per frame: its frame's width × height × `pathSamples` × `maxBounces`, the frame being its `viewport3D`'s or the project's. 320 samples at 20 bounces in 1080p is 6.25 times that.
- `inspect` lists each path-traced camera's frame size, samples, bounces and path segments per frame, marking those W15 warns about; in JSON they are `path_costs`.

## Command line

- `encode --output ID -o PATH` writes that output to PATH with every setting it declares. Before, `-o` silently replaced the named output with an ad-hoc one at the project size with default settings. PATH must keep the output's file type; several `--output` ids, an unknown id, or `--codec` with `--output` are usage errors (exit status 2).

## Declared attributes now implemented

- Instances honor `clipOut` for their playback span and automatic end, and `fit`/`boxWidth`/`boxHeight` for symbol placement, including cover clipping and a blurred cover behind contain-blur.
- Physical cameras use the sensor height for frames narrower than the sensor aspect, including depth of field. With default sensor dimensions, square and portrait scenes using `focalLength` intentionally change; wider frames retain the horizontal gate. Geodesic `pathSamples` above 4096 now emit W14 before clamping.
- `antialias3d` selects 1 (default), 2 or 4 raster samples, rounding 3 up to 4. Unsupported counts fall back with a warning. Previously the renderer always used four samples: default 3D edge coverage therefore intentionally changes; set 4 to retain that sampling.
- Plain fit output reframing honors layout and segment focus, including animated segment focus.
- A voxels object whose `palette` lists fewer materials than the indices its cells use, and that has no `material` of its own, colours those cells from the `.vox` file, as the schema states. Before, the frame failed with "the palette index N has no material" although the file has colours; `tests/corpus/valid/voxels.scene.xml` now renders.
- Medium albedo defaults to white in the volume library and renderer, using the library default as the renderer fallback.
- NaN opacity from animation keys becomes zero before inheritance, and repeat-copy opacity factors are clamped to 0..1. INERT-I17 identifies 2.5D attributes without threeD and alignment attributes without alignment; centered alignment still ignores margin.

## Examples

- `examples/glass-of-water/` drops a drop into a glass of water with native elements only: transmissive glass and water materials under the path tracer, an ocean bounded by the glass that the drop's `waterImpulse` drives, and the ocean's `whitewater` spray as the splash. A test holds it to those elements.

## Changes that can alter output or verdicts from v0.6.0

- Path-traced frames are denoised by the new filter, so their output changes; `denoise="false"` gives the raw estimate as before.
- With default sensor dimensions, square and portrait frames from physical cameras that set `focalLength` change framing and depth of field.
- 3D raster passes take one sample per pixel unless `antialias3d` asks for more; v0.6.0 always took four, so default 3D edges change.
- Path-traced cameras past the W15 threshold now warn, and geodesic `pathSamples` above 4096 warn W14 before clamping.
- On adapters with hardware ray queries, path-traced images can differ from v0.6.0 at rays whose nearest hit differs by rounding.
- `encode --output ID -o PATH` writes the named output with its declared settings, where v0.6.0 wrote an ad-hoc output at the project size.
- The JSON envelope is `schema_version` 1.2.0: `gpus` adds `ray_query`, `inspect` adds `path_costs` and `render --stats` adds `paths`.

## Licence

- scene-render is licensed under the PolyForm Noncommercial License 1.0.0, as from 0.5.0 (`LICENSE`, `NOTICE`). Commercial use needs a separate licence (`COMMERCIAL.md`).

The Linux archive contains the executable, source commit identifier, capability manifest, README, changelog, disclaimer, licence, notice, commercial-licence page and third-party notices. Its executable comes from the successful CI run for the release commit and is checksum-verified and smoke-tested before packaging.

Verify the download with `sha256sum -c scene-render-v0.7.0-linux-x86_64.tar.gz.sha256`, extract it, then run `./scene-render --help`.
