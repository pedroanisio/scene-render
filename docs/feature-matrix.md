# Feature matrix

| Area | v1.0.0 status | Boundary |
|---|---|---|
| XML + XSD | Implemented | Expat syntax plus engine semantic checks; DOCTYPE rejected |
| UHD 3840×2160 | Implemented | Arbitrary positive overrides also accepted |
| 360 equirectangular | Implemented | Mono 2:1 pixels; no spherical container metadata injection |
| Animated 360 viewport | Implemented | Perspective yaw/pitch/roll/FOV; panorama translation is irrelevant |
| Image decode | Implemented | PPM in C; other formats through FFmpeg |
| Video decode/cache | Implemented | Four decoded frames per shared source; FFmpeg process per miss |
| Audio decode/mix/sync | Implemented | Mono/stereo float mix, trim/loop/volume/pan |
| Text | Implemented | Built-in compact ASCII bitmap font, no shaping |
| Vector/2D shapes | Implemented | Rectangles and ellipses |
| 3D objects | Implemented approximation | Lit sphere, box, plane primitives; no arbitrary mesh import |
| Nested layers | Implemented | Dynamic arrays, memory-limited |
| Masks/clipping | Implemented | Rectangular masks; inverted group mask is rejected |
| Blend/alpha | Implemented | Normal/add/multiply/screen/overlay/difference |
| Linear-light composite | Implemented | sRGB transfer functions; no ICC profile pipeline |
| Keyframes | Implemented | Step, linear, three eases, cubic Bézier |
| Media time | Implemented | Trim, finite loops, reverse, speed, stretch, explicit remap |
| Camera | Implemented | Perspective/orthographic 3D primitives and spherical viewport motion |
| Lights | Implemented approximation | Ambient/directional/point/spot with color, range, falloff, animation |
| Shadows | Implemented approximation | Screen-space primitive shadows, not a shadow map/ray tracer |
| Effects | Implemented | Glow, bloom, blur, grade, vignette, lens-flare-style |
| Particles | Implemented | Stateless deterministic smoke/sparks/dust/rain presets |
| Rigid bodies | Implemented visual solver | Fixed-step circle/AABB contacts; not physically certified |
| Forces/constraints | Implemented | Gravity, directional/radial fields, springs, drag, damping, friction, restitution |
| Soft bodies | Visual approximation | Damped procedural surface displacement, not FEM/volumetric physics |
| Deformation | Implemented approximation | Bend, twist, wave, squash, stretch inverse warps |
| Physics cache | Implemented | Version/signature checked binary poses |
| H.264/H.265/FFV1 | Implemented when FFmpeg exposes encoder | Availability and legal terms depend on local FFmpeg |
| MP4/Matroska | Implemented | Container inferred by FFmpeg from output path |
| Resume | Implemented | Raw RGBA frame cache; output container is rebuilt |
| Multithreading | Implemented selectively | Deterministic viewport row partitions; scalar compositor fallback |
| GPU | CPU fallback only | Explicit warning; no GPU renderer in v1.0.0 |
| Metrics | Implemented | render time, encode/write/wait time, wall FPS, self/child peak RSS |

“Approximation” is intentional and visible in the API/documentation. No
physics, soft-body, shadow, flare, or 3D result is claimed to be physically
accurate.
