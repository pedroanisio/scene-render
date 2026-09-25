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
`N/D`). Optional fields are `seed`, `linearLight`, `background`, and `mode`:
`standard`, `equirectangular`, or `viewport`.

`output` requires `path` and `codec` (`h264`, `h265`, or `ffv1`). It accepts
`pixelFormat`, `preset`, mutually usable `crf`/`bitrate`, `audioCodec`, and
`audioBitrate`. Availability is checked by the FFmpeg child process.

## Assets

| Element | Required attributes | Optional |
|---|---|---|
| `image` | `id`, `src`, `width`, `height` | — |
| `video` | `id`, `src`, `width`, `height`, `fps`, `duration` | — |
| `audio` | `id`, `src` | — |
| `text` | `id`, `text`, `width`, `height`, `size` | `color` |
| `vector` | `id`, `shape`, `width`, `height` | `fill` |

Declared video geometry/FPS/duration make frame indexing explicit and
deterministic. The engine does not silently derive timeline metadata.

## Composition nodes

Groups and drawable 2D nodes accept `id`, `z`, `visible`, `opacity`, `start`,
`end`, `x`, `y`, `zPosition`, `rotation`, `rotationX`, `rotationY`, `scaleX`,
`scaleY`, `scaleZ`, `anchorX`, and `anchorY`. Lower `z` draws first; equal `z`
retains XML order. A group applies its transform and opacity recursively.

`layer` additionally requires `asset` and accepts `blend`, `clipIn`, `clipOut`,
`loop`, `reverse`, `speed`, and `timeStretch`. `loop="0"` means one play;
positive N means N total plays. An animated `source.time` track bypasses the
implicit clip/speed mapping.

`shape` requires `shape="rect|ellipse"`, `width`, and `height`; it accepts
`fill`, `stroke`, `strokeWidth`, and `blend`.

`particleEmitter` requires `preset="smoke|sparks|dust|rain"`; it accepts
`rate`, `lifetime`, `speed`, `spread` in degrees, `size`, `color`, and `blend`.

`mask` supports `type="rect"`, `x`, `y`, `width`, `height`, and `invert`.
Inverted masks are supported on drawable nodes; inverted group masks are
rejected because a bounded inverse compositing surface is not allocated.

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

Node properties include `opacity`, `position.x/y/z`, `rotation`, `scale.x/y`,
`anchor.x/y`, and `source.time`. Camera properties are `position.x/y/z`, `yaw`,
`pitch`, `roll`, and `fov`. Light properties include `intensity`, position,
`yaw`, and `pitch`. Effect properties include `intensity` and `radius`.
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

## Materials, 3D, and lights

`material` requires `id` and accepts `baseColor`, `metallic`, `roughness`, and
`emissive`.

`object3D` requires `id` and `primitive="sphere|box|plane"`; it accepts
`material`, position, three rotations, three scales, `radius`, `castShadow`, and
`receiveShadow`. Transform animations may be nested.

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
velocities, angular velocity, and radius. `softBody` accepts mass, stiffness,
damping, and pressure and produces a documented procedural approximation.

`deform` contains ordered `modifier` elements of type `bend`, `twist`, `wave`,
`squash`, or `stretch`. Controls are `amount`, `frequency`, `phase`, and
`axis="x|y"`; controls can contain animation tracks.

## Audio

`audioMix` accepts `sampleRate` (8–384 kHz) and `channels` (1 or 2). Each
`audioTrack` requires `id` and an audio `asset`, and accepts `start`, `clipIn`,
`clipOut`, finite `loop` play count, `volume` `[0,1]`, and `pan` `[-1,1]`.

## Colors and time

Colors are `#RRGGBB`, `#RRGGBBAA`, or normalized `r,g,b[,a]`. Times and
durations are decimal seconds. Frames are selected on a half-open interval;
frame N occurs exactly at `N × fps_den / fps_num`.
