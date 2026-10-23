# scene-render v0.7.3

This release makes the CPU path, lavapipe and llvmpipe, faster: the path tracer traces supported scenes in Rust on every core instead of in a compute shader, ambient occlusion runs at half size, and the compositor, output converter, mixer and x264 hand-off do less work per frame. On lavapipe a 1280x720 claymation with ambient occlusion encoded in 101.47 s instead of 120.30 s, and a 120-frame path-traced fracture film in 139 s. GPUs keep their frames; on software adapters the path tracer's and the occlusion's frames change within the bounds given below. The vendored schema is still sr-core 1.7.0.

## Path tracing on software adapters

- On software adapters the path tracer traces on the CPU in Rust instead of in a lavapipe compute shader: a port of the shader's trace pass (BVH and instance traversal, textures and materials, the dome and directional lights with their shadows, the BSDF, Russian roulette and the sample seeding) fills the same accumulation and guide buffers on every core, and the GPU denoise and output passes run on them as before. It applies when a scene uses only what the port covers; point, spot, area, ambient and IES lights, transmission, subsurface scattering, normal maps, splats, depth of field, adaptive sampling, light grids and a procedural sky keep the GPU trace, and `SR_PT_CPU=0` turns it off. On the 02-fracture scene a post-impact frame takes about 2.0 s instead of 4.4 s, and the whole 120-frame CPU-only encode took 139 s; frames differ at float level (PSNR 69 to 84 dB against the shader's frames at frames 20, 40, 60, 80, 100 and 119).
- On a CPU adapter the path tracer visits its 8x8 trace workgroups in a coprime-stride order instead of row by row: lavapipe gives each thread a contiguous run of workgroups, and uneven rows left one thread working at the end of each dispatch. Frames 20 and 80 of the 02-fracture scene decode bit-identical to before; frame 80's trace pass took 4.16 s instead of 5.1-5.4 s. GPUs keep the grid order.

## Rendering

- On a software adapter (lavapipe, llvmpipe) a camera's ambient occlusion and the depth and normal prepass it reads run at half the frame's size when nothing else reads that prepass (no screen-space reflections or contact shadows); the lit pass reads the occlusion filtered. A GPU keeps the full-size passes and the same frames. On lavapipe a 147 s 1280x720 claymation with ambient occlusion encoded in 101.47 s and 102.18 s instead of 120.30 s (user CPU 847 s instead of 1064 s); every one of its 736 frames stays within 49.1 dB PSNR of the full-size pass (50.8 dB on average).
- The first draw into a cleared target is recorded as a texture copy when it would write each texel unchanged: a frame-sized vector target that lands texel for pixel, at full opacity with the normal blend and no mask. Over a transparent target that draw adds nothing to its source, so the copy gives the same frame without running the compositor's fragment shader on every pixel. On lavapipe a 10 s slice of a 1080p explainer chapter encoded in 3.21 s instead of 3.46 s (render 1.67 s instead of 1.94 s), with the same frames and audio on every slice compared.
- The output converter compiles a variant for a frame that covers the output unmoved and unscaled, the usual case, in which each pixel samples the frame at its own position without placing or bounds-testing it. On lavapipe two 10 s slices of 1080p chapters rendered in 1.47 s and 1.32 s instead of 1.49 s and 1.38 s, with the same frames and audio; the converter's byte-for-byte test now checks this variant against the general shader.

## Delivery, encoding and audio

- x264 receives the renderer's nv12 frames as they are: asking it for yuv420p put a software conversion of every frame in front of an encoder that interleaves the chroma again. The stream is byte for byte the same; on 140 frames of a 1080p chapter x264 used 8.21 s of user CPU instead of 8.59 s.
- The audio mixer allocates each silent channel zeroed instead of writing one channel of zeros and copying it into the others. Copying those buffers was a quarter of the main thread's CPU time in a 10 s lavapipe encode slice, whose mix still spans the whole programme; such slices encoded in 9.78 s instead of 10.26 s together, with the same frames and audio.
- A delivery rendered in segments starts its join as soon as the last segment's worker returns: the coordinator used to notice that only at its next 200 ms progress tick. On lavapipe a 25 s chapter encoded in 4.35 s instead of 4.43 s on average over four runs, with the same frames.

## Changes that can alter output from v0.7.2

- On a software adapter a path-traced scene the CPU tracer takes changes at float level: its frames differ from the shader's by 69 to 84 dB PSNR on the 02-fracture scene. `SR_PT_CPU=0` restores the shader. Scenes with media, water or foam, adaptive sampling or light grids keep the shader and their frames.
- On a software adapter a camera with ambient occlusion and no screen-space reflections or contact shadows renders its occlusion at half size, within 49.1 dB PSNR of the full-size pass on a 736-frame claymation.
- On a GPU every frame and audio packet is as in v0.7.2.

## Licence

- scene-render is licensed under the PolyForm Noncommercial License 1.0.0, as from 0.5.0 (`LICENSE`, `NOTICE`). Commercial use needs a separate licence (`COMMERCIAL.md`).

The Linux archive contains the executable, source commit identifier, capability manifest, README, changelog, disclaimer, licence, notice, commercial-licence page and third-party notices. Its executable comes from the successful CI run for the release commit and is checksum-verified and smoke-tested before packaging.

Verify the download with `sha256sum -c scene-render-v0.7.3-linux-x86_64.tar.gz.sha256`, extract it, then run `./scene-render --help`.

## Unreleased

- Delivery extracts a frame's clay surfaces on the thread that evaluates frames ahead of the renderer, so that work overlaps the previous frame's render. The objects that changed are extracted in parallel with each other, and a surface whose blobs, finish, resolution and boil step are unchanged is reused. The renderer keys its clay meshes by a hash of those inputs instead of formatting every blob, and uploads the extracted vectors without copying them. Frames are unchanged: a 736-frame 1280x720 claymation with 94 clay objects, 71 of them boiling every frame, encoded to the same frames (framemd5) on lavapipe in 86.2 s instead of 91.7 s.
