# XML scene reference

The normative grammar is `schema/scene-v1.xsd`. Runtime validation adds numeric
relationships, unique IDs, reference types, 2:1 panorama checks, clip bounds,
and supported-codec behavior. Diagnostics use the XML source line plus element
and attribute whenever Expat exposes them.

## Document order

The XSD sequence is:

```xml
<scene version="1.0">
  <project .../>
  <output .../>           <!-- optional -->
  <assets>...</assets>    <!-- optional -->
  <materials>...</materials><!-- optional -->
  <scene360 .../>         <!-- optional -->
  <composition>...</composition>
  <lights>...</lights>    <!-- optional -->
  <effects>...</effects>  <!-- optional -->
  <physics>...</physics>  <!-- optional -->
  <audioMix>...</audioMix><!-- optional -->
</scene>
```

Runtime paths inside XML are relative to the XML file. CLI output/preview paths
are relative to the current working directory.

## Project and output

`project` requires positive `width`, `height`, `duration`, and `fps` (`N` or
`N/D`). Optional fields are `seed`, `linearLight`, `background`,
`workingColorSpace="srgb|rec709|display-p3|rec2020"`, and `mode`: `standard`,
`equirectangular`, or `viewport`.

`output` requires `path` and `codec` (`h264`, `h265`, or `ffv1`). It accepts
`pixelFormat`, `preset`, mutually usable `crf`/`bitrate`, `audioCodec`,
`audioBitrate`, `colorSpace="srgb|rec709|display-p3|rec2020"`,
`colorRange="limited|full"`, and `sphericalMetadata`. The engine preflights
the requested FFmpeg video and audio encoders and reports unavailable codecs
before rendering.

## Assets

| Element | Required attributes | Optional |
|---|---|---|
| `image` | `id`, `src`, `width`, `height` | `colorSpace` |
| `video` | `id`, `src`, `width`, `height`, `fps`, `duration` | `colorSpace` |
| `audio` | `id`, `src` | — |
| `text` | `id`, `text`, `width`, `height`, `size` | `color`, `font`, `fontFile` |
| `vector` | `id`, `shape`, `width`, `height` | `fill`, `path`, `fillRule`, `stroke`, `strokeWidth` |
| `mesh` | `id`, `src` | — |

Declared video geometry/FPS/duration make frame indexing explicit and
deterministic. The engine does not silently derive timeline metadata. Image
and video pixels are converted from their declared source color space to the
project working space. Text is shaped and rasterized by FFmpeg's drawtext
stack (FreeType, Fontconfig, FriBidi, and HarfBuzz in the verified build), so
UTF-8 and bidirectional scripts are supported. `fontFile` paths are resolved
relative to the XML file.

Vectors use `shape="rect|ellipse|path"`. Path data supports absolute and
relative `M`, `L`, `H`, `V`, `C`, `Q`, and `Z` commands in asset pixel
coordinates; every subpath needs at least two points. Coverage is the exact
area of each pixel inside the shape (signed-area scanline accumulation), with
`fillRule="evenodd"` (default) or `"nonzero"`. `stroke` (default transparent)
and `strokeWidth` (default 0, off) draw a stroke centred on the outline with
round joins and caps, over the fill; open subpaths are stroked open and filled
as if closed. Rect and ellipse vectors fill the asset box with one pixel of
anti-aliasing; half of their stroke lies outside the box and is clipped. Meshes load Wavefront OBJ vertices,
normals, and polygonal faces; faces are fan-triangulated and missing normals
are generated.

## Composition nodes

Groups and drawable 2D nodes accept `id`, `z`, `visible`, `opacity`, `start`,
`end`, `x`, `y`, `rotation`, `scaleX`, `scaleY`, `anchorX`, and `anchorY`.
Lower `z` draws first; equal `z` retains XML order. A group applies its
transform recursively and accepts `blend`. A group with `blend` other than
`normal` or `opacity` below 1 (at that time) is *isolated*: its children
render into a transparent buffer that is then composited once with the
group's blend, opacity, and masks (children blend against that buffer, not
against what lies below the group). Otherwise the group is a pass-through:
its children draw directly into the parent against the real backdrop, and
the group's masks (if any) scale each child's coverage.

`layer` additionally requires `asset` and accepts `blend`, `clipIn`, `clipOut`,
`loop`, `reverse`, `speed`, and `timeStretch`. `loop="0"` means one play;
positive N means N total plays. An animated `source.time` track bypasses the
implicit clip/speed mapping.

`shape` requires `shape="rect|ellipse"`, `width`, and `height`; it accepts
`fill`, `stroke`, `strokeWidth`, and `blend`. Edges are anti-aliased from a
signed distance at the node's pixel footprint; the stroke is `strokeWidth`
wide and centred on the outline, drawn over the fill.

`particleEmitter` requires `preset="smoke|sparks|dust|rain"`; it accepts
`rate`, `lifetime`, `speed`, `spread` in degrees, `size`, `color`, and `blend`.
The numeric emitter controls `rate`, `lifetime`, `speed`, `spread`, and `size`
are animatable.

`mask` supports `type="rect|ellipse|rounded-rect"`, `x`, `y`, `width`,
`height`, `radius`, and `invert`. Masks may be placed on drawable nodes or
groups, and a node may carry any number of them: every `mask` child is kept
in document order and the coverage is the product of each mask's
anti-aliased coverage (`1 - coverage` when inverted), evaluated in the node's
inverse world transform. `x`, `y`, `width`, `height`, and `radius` are
animatable with nested `<animate property="x|y|width|height|radius">`.
Group masks apply when an isolated group is composited, or to every child
draw of a pass-through group, so they compose through nested world
transforms.

## Animation

```xml
<animate property="position.x" defaultInterpolation="ease-in-out">
  <key time="0" value="100"/>
  <key time="2" value="500"/>
</animate>
```

Interpolation names are `step`, `linear`, `ease-in`, `ease-out`,
`ease-in-out`, and `cubic-bezier`. A cubic key requires
`bezier="x1,y1,x2,y2"`, with both X controls in `[0,1]`. Keys may appear out of
order but times must be unique after sorting.

Node properties include `opacity`, `position.x/y`, `rotation`, `scale.x/y`,
`anchor.x/y`, and `source.time`. Camera properties are `position.x/y/z`, `yaw`,
`pitch`, `roll`, and `fov`. Light properties include `intensity`, position,
`yaw`, and `pitch`. Effect properties include `intensity` and `radius`.
Particle properties are `rate`, `lifetime`, `speed`, `spread`, and `size`.
Modifier properties are `amount`, `frequency`, and `phase`. 3D object transform
properties include position, rotation, and scale axes.

## Cameras and 360

`scene360` declares `layout="equirectangular"`, a 2:1 `width`/`height`, and an
optional `viewportCamera` ID. Equirectangular mode requires project dimensions
to match. Viewport mode renders the declared panorama and writes project-sized
perspective output.

`camera` inside `composition` requires `id` and accepts `active`,
`projection="perspective|orthographic"`, `x/y/z`, `yaw/pitch/roll`, `fov`,
`near`, and `far`. Spherical viewports use orientation and FOV; standard 3D
primitives additionally use translation and projection.

Viewport orientation conventions (all angles in degrees):

- `fov` is the **vertical** field of view; the horizontal extent follows from
  the output aspect ratio. Animated values must stay within (1, 179).
- `yaw="0" pitch="0"` looks at the center of the panorama (longitude 0).
  Positive `yaw` turns right, toward larger panorama x.
- Positive `pitch` looks **down** (toward the bottom rows of the panorama);
  negative `pitch` looks up.
- Positive `roll` rotates the camera counter-clockwise about its forward
  axis, so the horizon appears rotated clockwise in the output.
- Rotations compose as roll, then pitch, then yaw (yaw is outermost).
- Panorama pixel (i, j) is centred at (i + 0.5, j + 0.5); lookups are
  bilinear between pixel centres, wrap horizontally across the 360° seam, and
  clamp vertically at the poles.

For equirectangular MP4/MOV output with `sphericalMetadata="true"`, the final
file receives the Google Spatial Media v1 spherical UUID box. Matroska keeps
the projection stream tags written by FFmpeg. Metadata injection is atomic and
only occurs after a successful encode.

## Materials, 3D, and lights

`material` requires `id` and accepts `baseColor`, `metallic`, `roughness`, and
`emissive`.

`object3D` requires `id` and `primitive="sphere|box|plane|mesh"`; it accepts
`mesh` (required for `primitive="mesh"`), `material`, position, three rotations, three
scales, `radius`, `castShadow`, and `receiveShadow`. Transform animations may
be nested. Meshes are triangle-rasterized with a shared depth buffer. Shadow
occlusion is a deterministic bounding-volume/screen-space approximation, not
a physically based ray tracer.

Top-level `lights` contains `light` entries. Each requires `id` and
`type="ambient|directional|point|spot"`, and accepts color, intensity, position,
yaw/pitch, range, falloff, spot angle, and shadow flag. Light animation is
nested normally.

## Effects

Top-level `effects` is applied in XML order. Each `effect` requires `id` and a
type: `glow`, `bloom`, `blur`, `color-grade`, `vignette`, or `lens-flare`.
Common controls are `enabled`, `intensity`, `radius`, `threshold`,
`saturation`, `contrast`, `brightness`, and `color`; irrelevant controls are
ignored by a particular effect.

## Physics and deformation

`physics` accepts `fixedStep`, `gravityX`, `gravityY`, and an optional binary
`cache` path. It may contain:

- directional `forceField` with `forceX/forceY`;
- radial `forceField` with `x/y`, `strength`, and `falloff`;
- `constraint type="spring|distance"` with body IDs `a/b`, `restLength`,
  `stiffness`, and `damping`.

A drawable node may contain `rigidBody` with static/kinematic/dynamic type,
box/circle shape, mass, friction, restitution, linear/angular damping, initial
velocities, angular velocity, and radius. Damping (per second, default
0.01) scales velocity by `exp(-damping * fixedStep)` each step, so it decays
smoothly and never reverses direction. `softBody` accepts mass, stiffness,
damping, and pressure and produces a documented procedural approximation.

`deform` contains ordered `modifier` elements of type `bend`, `twist`, `wave`,
`squash`, or `stretch`. Controls are `amount`, `frequency`, `phase`, and
`axis="x|y"`; controls can contain animation tracks.

## Audio

`audioMix` accepts `sampleRate` (8–384 kHz) and `channels` (1 or 2). Each
`audioTrack` requires `id` and an audio `asset`, and accepts `start`, `clipIn`,
`clipOut`, finite `loop` play count, `volume` `[0,1]`, and `pan` `[-1,1]`.

## Colors and time

Colors are `#RRGGBB`, `#RRGGBBAA`, or normalized `r,g,b[,a]`, written as
transfer-encoded values of the project working space. Named color spaces are
sRGB, Rec.709, Display-P3, and Rec.2020. Assets are decoded from their source
space into the blend space (the working gamut, linear light when
`linearLight="true"`, premultiplied float), blending follows the W3C
separable formulas there (`add` is not clamped until output), and the final
frame is converted to 8-bit in the output space with matching FFmpeg
primaries/transfer/matrix/range tags. Times and durations are decimal
seconds. Frames are selected on a half-open interval; frame N occurs exactly
at `N × fps_den / fps_num`.
