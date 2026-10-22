# scene-render v0.7.1

This release fixes the path tracer's hardware ray queries on Mesa's lavapipe, which missed most surfaces of a path-traced scene, and falls back to the BVH walk for every scene too large for the adapter's acceleration structures. The vendored schema is still sr-core 1.7.0.

## Path tracer

- The hardware traversal builds one bottom-level structure of the scene's triangles and one of the bounds of its splats and instances, each holding its own primitives only, and maps a hit to its primitive through a table. Before, each structure also held the other kind's primitives as inactive entries; Mesa's lavapipe (observed on 25.0.7 and 25.2.8) let those into its bounds and then missed nearly every hit, so path-traced frames lost most of their surfaces. A scene with any volume is path-traced, so on lavapipe this also reached scenes whose camera does not ask for the path tracer, such as one with an empty pyro volume.
- The fallback to the BVH walk compares the triangle structure's vertex count with the adapter's `max_blas_primitive_count`, the count wgpu validates. Before, it compared the triangle count, so a scene with more vertices than that limit but fewer triangles failed to build its structures instead of falling back.

## Changes that can alter output from v0.7.0

- On lavapipe, path-traced frames that use hardware ray queries change: they now show the surfaces the BVH walk and OpenGL show. `SR_GPU_RAY_QUERY=0`, the BVH walk and OpenGL are unchanged.

## Licence

- scene-render is licensed under the PolyForm Noncommercial License 1.0.0, as from 0.5.0 (`LICENSE`, `NOTICE`). Commercial use needs a separate licence (`COMMERCIAL.md`).

The Linux archive contains the executable, source commit identifier, capability manifest, README, changelog, disclaimer, licence, notice, commercial-licence page and third-party notices. Its executable comes from the successful CI run for the release commit and is checksum-verified and smoke-tested before packaging.

Verify the download with `sha256sum -c scene-render-v0.7.1-linux-x86_64.tar.gz.sha256`, extract it, then run `./scene-render --help`.

## Unreleased

- Clay objects mesh their field on every core: each pass of the surface-nets extraction runs over independent slabs and joins them in order, so the mesh is the same, vertex for vertex and index for index, as before. A fingerprinted seven-blob object at `resolution="160"` meshed in 359.5 ms on one core and meshes in 45.4 ms on a 12-thread Ryzen 5 5600X; boiling clay re-meshes on every boil frame, so a 5 s clay film encoded in 4.6 s instead of 21.7 s there.
- A geodesic camera's picture reads each sample's orbit from a table instead of integrating it. A ray's orbit depends only on its impact parameter and on whether it starts toward the hole, so each frame tabulates u(φ) and du/dφ for 2048 impact parameters (f64 RK4, crowded round the critical 3√3 M where the higher-order images are thin) and each sample interpolates its crossings and its escape angle from the two rows around it. A 1280 × 720 film at 64 samples per pixel encoded in 2.4 s instead of 16.0 s on an RTX 3050; its frames differ from the traced ones by 54.4 dB PSNR at worst (56.1 dB on average).
- Documents with a simulation evaluate the next frame on a helper thread while the current one renders, as documents without one already did. A simulated document gives each frame the same graph whether or not the frame after it was evaluated first, which every simulated corpus document is now tested for; the frames are unchanged. A 5 s film of a flock, a slime mould and a smoke solver spent 3.3 s instead of 4.0 s waiting on evaluation.
