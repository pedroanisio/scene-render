# Features

`scene-render` 1.1: one XML scene in, one video file out. C17 engine, FFmpeg for
decode/encode. Details: [xml-reference](xml-reference.md) ·
[feature-matrix](feature-matrix.md) (limits) · [architecture](architecture.md).

## Output

| Feature | Notes |
|---|---|
| Standard video | Any resolution/fps; 3840×2160 typical |
| 360° equirectangular | 2:1 canvas; Spatial Media metadata in MP4 |
| 360° viewport | Animated perspective cut (yaw/pitch/roll/FOV) from a panorama |
| Codecs | H.264, H.265, FFV1 video; AAC etc. audio (whatever FFmpeg exposes) |
| Containers | MP4, Matroska (from file extension) |
| Color | sRGB, Rec.709, Display P3, Rec.2020; per-asset source space, working space, tagged output |

## Scene content

| Feature | Notes |
|---|---|
| Images | PPM natively; any format via FFmpeg |
| Video | Shared sources, 4-frame cache; trim, loop, reverse, speed, stretch, time remap |
| Text | UTF-8, shaped, bidi; font family or font file |
| Vectors | Rect, ellipse, SVG-style paths (M L H V C Q Z), antialiased |
| Shapes | Rect/ellipse with fill + stroke (both animatable colors) |
| 3D | Sphere, box, plane, Wavefront OBJ meshes; depth buffer; `antialias3d` 1–4 supersampling |
| Materials | Base color, emissive, metallic, roughness |
| Lights | Ambient, directional, point, spot; shadow maps (PCF) for directional/spot, approximate for point; animated color |
| Cameras | Perspective / orthographic |
| Particles | Parametric emitters (direction, spread, speed ± variance, gravity, size→sizeEnd, color→colorEnd, lifetime ± variance, spawn area, cap, seed, disc/square); optional smoke/sparks/dust/rain presets; stateless and deterministic |

## Compositing

- Nested groups, unlimited layers, z-order
- Six blend modes: normal, add, multiply, screen, overlay, difference
- Masks: rect, ellipse, rounded rect; invertible; on layers or groups
- Linear-light blending (optional)

## Animation

- Keyframes on almost every numeric property: transforms, opacity, camera, lights, every effect parameter, particles, deformers and mesh-warp points, force fields, media time
- Color keyframes (shape fill/stroke, particle color/colorEnd, light and effect color), interpolated in linear light
- Easing: step, linear, ease-in/out/in-out, cubic Bézier

## Physics & deformation

- 2D rigid bodies: static/kinematic/dynamic, circle/box, friction, restitution, damping; exact circle-vs-box and oriented box-box (SAT) contacts
- Gravity, directional, radial and vortex force fields (animatable)
- Spring, distance and pin constraints (rigid or springy pins)
- Soft bodies: simulated mass-spring grids with pressure and pinning
- Deformers: bend, twist, wave, squash, stretch, mesh-warp
- Fixed timestep, cacheable

## Post effects

Glow, bloom, blur, color grade, vignette, lens flare, drop shadow, 2D lighting (point/spot/directional/ambient lights, falloff curves, alpha relief). Applied in order to the whole frame, or to a group's buffer when the group lists them in `effects`; every parameter animates; skipped when intensity is 0.

## Audio

Mono/stereo mix: per-track start, trim, loop, volume, pan; synced to video.

## Workflow (CLI)

| Flag | Does |
|---|---|
| `--validate` | Check XML only |
| `--preview-frame N` | Render one frame to PPM |
| `--frame-range A:B` | Render a slice |
| `--mode` | Switch standard / equirectangular / viewport |
| `--resolution`, `--fps`, `--quality` | Override output |
| `--threads auto\|N` | Parallel workers |
| `--renderer gpu` | OpenCL color stage; CPU fallback |
| `--resume` | Continue an interrupted render |
| `--metrics` | Timing and memory |

## Guarantees

- Deterministic: same inputs, same seed, byte-identical frames (CPU)
- Strict validation: errors cite line, element, attribute
- Bounded memory: frames streamed, video never fully loaded

## Not included

No physically accurate simulation, PBR, ray tracing, or GPU rasterizer. See
[feature-matrix](feature-matrix.md).
