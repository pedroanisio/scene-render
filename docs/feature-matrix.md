# Feature matrix

| Area | v1.1.0 status | Boundary |
|---|---|---|
| XML + XSD | Implemented | Expat syntax plus engine semantic checks; contextual line/element/attribute diagnostics; DOCTYPE rejected |
| UHD 3840×2160 | Implemented | Any positive CLI/XML resolution is accepted within memory limits |
| 360 equirectangular | Implemented | Configurable mono 2:1 canvas; MP4 Spatial Media v1 UUID/XML; Matroska stream tags |
| Animated 360 viewport | Implemented | Perspective yaw/pitch/roll/FOV; panorama translation is intentionally irrelevant |
| Image decode | Implemented | PPM in C; other formats through supervised FFmpeg |
| Video decode/cache | Implemented | Four decoded RGBA frames per shared source; FFmpeg process per cache miss |
| Audio decode/mix/sync | Implemented | Mono/stereo float mix, trim, finite loop, volume, pan; exact selected-interval duration |
| Text | Implemented | FFmpeg drawtext with FreeType/Fontconfig/FriBidi/HarfBuzz shaping; UTF-8 and explicit font family/file |
| Vector assets | Implemented | Rect, ellipse, and antialiased filled paths using M/L/H/V/C/Q/Z (absolute or relative) |
| 2D shapes | Implemented | Rectangles/ellipses, fill/stroke, transforms, animation, physics, deformation |
| 3D objects | Implemented visual renderer | Lit sphere/box/plane plus triangulated Wavefront OBJ with vertex normals or generated face normals |
| 3D visibility | Implemented | CPU depth buffer across imported meshes and primitives; perspective-correct mesh depth, camera-facing sphere depth |
| Depth cards (2.5D) | Implemented | Camera-projected 2D planes: affine fast path, perspective warp (plane buffer ≤ 4096 px side / 8 Mpx), far-to-near run sort, per-sample depth shared with 3D, alpha ≥ 0.5 depth writes, DOF blur (σ ≤ 64 px); crossing translucent cards blend in draw order; standard mode only |
| Nested layers | Implemented | Dynamic arrays and dynamic XML nesting; memory-limited rather than fixed layer count |
| Masks/clipping | Implemented | Rect, ellipse, rounded rectangle, inversion, nested group transforms |
| Blend/alpha | Implemented | Normal/add/multiply/screen/overlay/difference with straight alpha |
| Color management | Implemented named spaces | Separate sRGB and Rec.709 transfers, Display P3, Rec.2020 matrices/transfers; per-media source, project working, output tags |
| Linear-light compositing | Implemented | Uses the active working-space transfer; selectable per project |
| Keyframes | Implemented | Step, linear, ease-in, ease-out, ease-in-out, cubic Bézier |
| Media time | Implemented | Trim, finite loop, reverse, speed, stretch, explicit source-time remap |
| Camera | Implemented | Perspective/orthographic 3D, depth cards, and spherical viewport motion; `zoom` focal length |
| Lights/materials | Implemented visual model | Ambient/directional/point/spot; base/emissive/metallic/roughness controls; animation |
| Shadows | Implemented approximation | Screen-space ground shadow plus bounding-volume inter-object occlusion |
| Effects | Implemented | Glow, bloom, sliding box blur, grade, vignette, lens-flare-style |
| Particles | Implemented | Stateless deterministic smoke/sparks/dust/rain presets; animated rate/lifetime/speed/spread/size |
| Rigid bodies | Implemented visual solver | Fixed-step circle and AABB contacts; mixed pairs use bounding circles; explicitly not physically certified |
| Forces/constraints | Implemented | Gravity, directional/radial fields, springs/distance, drag/damping, friction/restitution |
| Soft bodies | Visual approximation | Damped procedural displacement, not FEM/volumetric physics |
| Deformation | Implemented approximation | Bend, twist, wave, squash, stretch inverse warps |
| Physics cache | Implemented | Atomic, version/signature-checked binary poses; fields/constraints/geometry invalidate correctly |
| H.264/H.265/FFV1 | Implemented when exposed by FFmpeg | Encoder is preflighted; availability/legal terms depend on installed FFmpeg |
| MP4/Matroska | Implemented | Container inferred from path; color/range metadata emitted |
| Resume | Implemented | Atomic raw post-color RGBA cache; output container is rebuilt |
| Multithreading | Implemented where deterministic | Viewport rows, effects, blur, and CPU color conversion use disjoint partitions |
| GPU | Optional OpenCL 1.2 hybrid | GPU final color conversion; CPU reference raster/compositor; automatic deterministic fallback |
| Metrics | Implemented | Render, encoder write/wait, wall, FPS, self/child RSS and conservative sum |

“Approximation” is intentional and user-visible. The project does not claim
physically accurate rigid/soft-body simulation, PBR materials, ray-traced
shadows, optical lens flare, or identical floating-point output across GPU
vendors. The CPU path is the byte-exact regression reference.
