# XML scene reference

The normative grammar is `schema/scene-v1.xsd`. It is embedded in the binary
at build time (`scene-render --print-schema` prints it) and every scene is
validated against it with libxml2 before loading: no network access, no
external entities or DTD loading, no entity substitution. Each schema error
becomes a diagnostic `FILE:LINE: error: <element> @attribute: message` (the
attribute is taken from libxml2's message when it names one) and the load
fails with exit code 3. Documents with a DOCTYPE, and documents that are not
well-formed, are rejected by the loader's own checks with its usual message.
After the schema, the Expat loader adds numeric relationships, unique IDs,
reference types, 2:1 panorama checks, clip bounds, and supported-codec
behavior. Its diagnostics use the XML source line plus element and attribute
whenever Expat exposes them.

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

`project` requires positive `width`, `height`, `duration` (at most 1e6 s),
and `fps` (`N` or `N/D`). Optional fields are `seed`, `linearLight`, `background`,
`workingColorSpace="srgb|rec709|display-p3|rec2020"`, `mode`: `standard`,
`equirectangular`, or `viewport`, and `antialias3d="1|2|3|4"` (default 1):
the 3D pass is rendered at N x N samples per pixel and box-filtered (memory
about `24 x width x height x N^2` bytes during the pass). With 1 the 3D pass
is unchanged.

`output` requires `path` and `codec` (`h264`, `h265`, or `ffv1`). It accepts
`pixelFormat`, `preset`, mutually usable `crf`/`bitrate`, `audioCodec`,
`audioBitrate`, `colorSpace="srgb|rec709|display-p3|rec2020"`,
`colorRange="limited|full"`, and `sphericalMetadata`. Encoding runs
in-process through libavformat/libavcodec (no FFmpeg executable). The container
follows the output extension: `.mp4`/`.mov` (MP4/QuickTime) or `.mkv`
(Matroska); any other extension is an error, and `ffv1` requires `.mkv`.
`codec` selects the `libx264`, `libx265`, or `ffv1` encoder; `pixelFormat` is
any libavutil pixel format name the chosen encoder supports (an unsupported
one is rejected with the list of supported formats). Formats with more than
8 bits per component (for example `yuv420p10le`) are fed from a 16-bit RGBA
conversion of the frame. `audioCodec` names any libavcodec audio encoder
(`aac` by default). RGB is converted to Y'CbCr with the BT.709 matrix for
`srgb`, `rec709` and `display-p3` outputs and BT.2020 for `rec2020`, from full
range to the `colorRange` range, and the stream is tagged accordingly.
Timestamps are constant-rate (`0..N-1` in units of `1/fps`) and the output is
bit-exact: identical scenes produce identical files. Unavailable encoders are
reported when the encoder opens, before the first frame renders.

## Assets

| Element | Required attributes | Optional |
|---|---|---|
| `image` | `id`, `src`, `width`, `height` | `colorSpace` |
| `video` | `id`, `src`, `width`, `height`, `fps`, `duration` | `colorSpace` |
| `audio` | `id`, `src` | — |
| `text` | `id`, `text`, `width`, `height`, `size` | `color`, `font`, `fontFile`, `align`, `lineHeight`, `letterSpacing`, `direction`, `language`, `verticalAlign` |
| `vector` | `id`, `shape`, `width`, `height` | `fill`, `path`, `fillRule`, `stroke`, `strokeWidth` |
| `mesh` | `id`, `src` | — |

Declared video geometry/FPS/duration make frame indexing explicit and
deterministic. The engine does not silently derive timeline metadata: frame
index `i` at the declared `fps` shows the latest decoded frame whose
presentation time is at most `i / fps` (within 10⁻⁶ of a frame, or half a
timestamp tick of the container when that is coarser), never a later frame.
Time zero is the container's start time with audio codec priming excluded,
the same origin the asset's soundtrack uses, so the video and audio of one
file stay in sync. A stream whose frame rate or length differs from
the declaration produces a warning (the declared values still apply); a
stream whose dimensions differ is an error. Indices before the first frame
show the first frame, indices with no frame of their own (variable-rate gaps)
show the latest earlier frame, and indices past the last frame show the last
one. A stream without timestamps counts one frame per packet from the start
of the file and is never seeked (going backward reopens it). Each video asset
keeps one decoder open for the whole render; sequential access decodes
forward, backward or far jumps seek to the latest keyframe at or before the
target time (both choose the same frame), and converted frames are kept in a
256 MiB LRU per asset, so every layer sharing the asset shares its frames;
the frame just returned is always kept, even when it alone exceeds the
budget. Y'CbCr is converted with the stream's own matrix and range (BT.709,
BT.601, SMPTE 240M, FCC or BT.2020; BT.2020 constant luminance uses the
non-constant-luminance matrix, with a warning; untagged streams use BT.709
from 720 lines up and BT.601 below). Images are decoded in-process too, from any format libavformat reads
(PNG, JPEG, PPM, WebP, ...); PPM/PNM files must match the declared size,
other formats are resampled to it (Lanczos). Image
and video pixels are converted from their declared source color space to the
project working space.

Text is laid out and rasterized in-process (see "Text" below).

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

### Text

A `text` asset is a `width` x `height` box; `size` is the font size in pixels
per em, in (0, 4096]. The `text` attribute holds at most 1 MiB (1048576
bytes of UTF-8). The font is `fontFile` (resolved relative to the XML file;
face 0) or, without it, the `font` family (default `Sans`) looked up through
Fontconfig.
A family that is not installed is an error naming the family: when
Fontconfig's best match belongs to a different family the render fails rather
than substituting silently. The generic names `sans`, `sans-serif`, `serif`,
`monospace`, `mono` and `system-ui` accept whatever Fontconfig maps them to.
Characters the font lacks render as its missing-glyph box (no fallback).

- Paragraphs are separated by line feeds, written `&#10;` in the attribute
  (XML turns literal newlines in attribute values into spaces).
- Each paragraph's direction is `direction="auto"` (default: from its first
  strong character, UAX #9; left-to-right when there is none), `ltr` or `rtl`.
  Bidirectional reordering follows UAX #9 per line: whitespace ending a line
  takes the paragraph direction (rule L1) before runs are reversed (L2).
- Text is shaped with HarfBuzz (kerning, ligatures, contextual forms of
  complex scripts) per line and per bidi level and script run. Each line is
  shaped on its own, so joining forms never connect across a line break.
  `language` (a BCP 47 tag such as `ar` or `sr-Latn`) selects
  language-specific shaping; it must have the shape
  `[A-Za-z]{2,8}(-[A-Za-z0-9]{1,8})*` and at most 35 characters. HarfBuzz
  keeps every distinct language tag it sees for the life of the process, so
  a long-running renderer should not be fed unbounded sets of tags.
- Lines wrap to `width`: at spaces (which hang at the line end, together
  with any combining marks on them) and after hyphens; a word wider than the
  box breaks between grapheme clusters. Breaks never split a cluster. A line
  is re-measured after it is shaped on its own (kerning across the break is
  gone) and, if it no longer fits, breaks at the previous opportunity; only
  a single cluster wider than the box overflows.
- `align="start|center|end|justify"` (default `start`); start and end follow
  the paragraph direction (start is the right edge for right-to-left text).
  `justify` widens the spaces of every line except a paragraph's last line.
- `lineHeight` is the baseline distance as a multiple of `size` (default
  1.2, in [0.1, 10]). Each line box is `lineHeight x size` tall with the
  font's ascent and descent centred in it; baselines are whole pixels.
- `letterSpacing` (pixels, default 0, in [-`size`, 4 x `size`]) is added
  after each cluster, so a line of N clusters grows by (N-1) x letterSpacing.
- `verticalAlign="top|middle|bottom"` (default `top`) places the block of
  lines inside the box.
- Ink outside the box is clipped, with one warning per asset.

Glyphs are unhinted FreeType outlines placed at HarfBuzz positions quantized
to 1/4 pixel and anti-aliased at 8 bits; the text color is an XML color
(working space) converted to blend space like every other color. Output is
identical for identical inputs and library versions.

## Composition nodes

Groups and drawable 2D nodes accept `id`, `z`, `visible`, `opacity`, `start`,
`end`, `x`, `y`, `rotation`, `scaleX`, `scaleY`, `anchorX`, and `anchorY`.
Lower `z` draws first; equal `z` retains XML order. A group applies its
transform recursively and accepts `blend` and `effects`, a space-separated
list of `effect` ids (see Effects); an unknown id is an error reported at the
group's line and `effects` attribute. A group with `blend` other than
`normal`, `opacity` below 1 (at that time), or any `effects` is *isolated*: its children
render into a transparent buffer that is then composited once with the
group's blend, opacity, and masks (children blend against that buffer, not
against what lies below the group). Otherwise the group is a pass-through:
its children draw directly into the parent against the real backdrop, and
the group's masks (if any) scale each child's coverage.

`layer` additionally requires `asset` and accepts `blend`, `clipIn`, `clipOut`,
`loop`, `reverse`, `speed`, and `timeStretch`. `loop="0"` means one play;
positive N means N total plays. An animated `source.time` track bypasses the
implicit clip/speed mapping.

The implicit source interval is `[clipIn, clipOut)`. At a frame-aligned
`clipOut`, reverse playback starts with the preceding frame; after the last
play, forward playback holds that frame and reverse playback holds the
frame at `clipIn`.

`shape` requires `shape="rect|ellipse"`, `width`, and `height`; it accepts
`fill`, `stroke`, `strokeWidth`, and `blend`. Edges are anti-aliased from a
signed distance at the node's pixel footprint; the stroke is `strokeWidth`
wide and centred on the outline, drawn over the fill.

`particleEmitter` is parametric. Particle `i` (in emission order) is born at
`i / rate` seconds after the emitter's `start` (with an animated `rate`, where
the rate integrated from `start` on a fixed 1/240 s grid reaches `i`; the
integral is cached per key segment and closed form before the first and
after the last key, so evaluating late frames costs no more than early ones), and at
age `a` of its lifetime `L` (fraction `f = a / L`) sits in the emitter's local
space at

```
x = ex + cos(d) * v * a + gravityX * a^2 / 2
y = ey + sin(d) * v * a + gravityY * a^2 / 2
```

with `d = direction + spread * u1` (degrees, `u1` uniform in [-1, 1]: `spread`
is the largest deviation either side), `v = speed + speedVariance * u2`,
`L = lifetime + lifetimeVariance * u0` (at least 1 µs), and `(ex, ey)` uniform
in the centred `emitterWidth` x `emitterHeight` spawn rectangle. Its radius
goes linearly from `size` to `sizeEnd` (default `size`) and its color from
`color` to `colorEnd` in linear light (default: `color` at zero alpha, a fade
out). Particles are drawn oldest first as anti-aliased `shape="disc|square"`
(default disc) of that radius in canvas pixels. Every particle is a closed
function of the seed and its index, so any frame renders identically alone,
in sequence, or on any thread count. The random streams come from `seed`
(unsigned 64-bit) when given, else from the project seed XOR a hash of the
emitter id. At most `maxParticles` (default 10000, up to 10,000,000) are
alive; when the cap binds the newest are kept.

| Attribute | Default | Notes |
|---|---|---|
| `rate` | 10 | particles per second, ≥ 0 |
| `lifetime`, `lifetimeVariance` | 1, 0 | seconds |
| `speed`, `speedVariance` | 100, 0 | px/s |
| `direction`, `spread` | -90 (up), 0 | degrees, 0 = +x, 90 = down |
| `gravityX`, `gravityY` | 0, 0 | px/s² |
| `size`, `sizeEnd` | 4, `size` | radius in px |
| `color`, `colorEnd` | white, `color` at alpha 0 | |
| `emitterWidth`, `emitterHeight` | 0, 0 | spawn area in px |
| `maxParticles`, `seed`, `shape`, `blend` | 10000, —, disc, normal | |

`rate`, `speed`, `spread`, `size`, `lifetime`, and `direction` are animatable
and sampled at each particle's birth (a particle keeps the values it was
emitted with); `color` and `colorEnd` take color keyframes, also sampled at
birth. `preset="smoke|sparks|dust|rain"` is optional and only changes
defaults, reproducing the earlier fixed presets; explicit attributes
override them:

| Preset | Defaults |
|---|---|
| `sparks` | `gravityY` 400; speed drawn from 0.5–1.0 x `speed` |
| `smoke` | speed 0.5–1.0 x `speed`; ±15 px sideways wobble `15 sin(3a + u1)`; size grows to `size x (1 + L)` |
| `dust` | speed 0.125–0.25 x `speed` |
| `rain` | `direction` 90; `emitterWidth` 80; speed 0.5–1.0 x `speed` |

With a preset and no `speedVariance`, `v = speed x (m + w u2)` with the
preset's mean `m` and half-width `w` above; `speedVariance` switches to the
plain formula.

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
`yaw`, `pitch`, and `color`. Effect properties are `intensity`, `radius`,
`threshold`, `saturation`, `contrast`, `brightness`, `offsetX`, `offsetY`,
`relief`, and `color`. Particle properties are `rate`, `lifetime`, `speed`,
`spread`, `size`, `direction`, `color`, and `colorEnd`. Shape properties
additionally include `fill` and `stroke`. Modifier properties are `amount`,
`frequency`, and `phase`; mesh-warp `point` properties are `x` and `y`.
Force-field properties are `strength`, `forceX`, `forceY`, `x`, and `y`. 3D
object transform properties include position, rotation, and scale axes.

Color properties (`fill`, `stroke`, `color`, `colorEnd`) take color strings
as key values:

```xml
<animate property="fill" defaultInterpolation="ease-in-out">
  <key time="0" value="#FF0000"/>
  <key time="2" value="#0000FF80"/>
</animate>
```

They interpolate on straight-alpha linear-light components: each key's red,
green, and blue are decoded with the working space's transfer, interpolated
with the key's curve (the same eased fraction for every channel, alpha
included), re-encoded, and then converted to blend space like any XML color.
The midpoint of black and white is therefore `#BCBCBC`, not `#808080`. Text
colors are static asset properties.

## Cameras and 360

`scene360` declares `layout="equirectangular"`, a 2:1 `width`/`height`, and an
optional `viewportCamera` ID. Equirectangular mode requires project dimensions
to match. Viewport mode renders the declared panorama and writes project-sized
perspective output.

`camera` inside `composition` requires `id` and accepts `active`,
`projection="perspective|orthographic"`, `x/y/z`, `yaw/pitch/roll`, `fov`,
`near`, `far`, `zoom`, `focusDistance`, and `aperture`. Spherical viewports
use orientation and FOV; standard 3D primitives and depth cards additionally
use translation and projection. `zoom` (positive, px, animatable as
`zoom`) is the focal length and replaces `fov` when present:
`fov = 2 atan(height / 2 / zoom)`. `focusDistance` (positive, default 1000)
and `aperture` (non-negative px, default 0) set depth of field for depth
cards; both animate under the same names.

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
file receives both the Spherical Video V2 `sv3d`/`st3d` boxes (written by the
muxer from equirectangular spherical side data) and the Google Spatial Media
v1 spherical UUID box, injected after the encode into the video `trak` of
either MP4 layout (the encoder writes `moov` first, "faststart"; chunk offsets
are shifted accordingly). Matroska receives its equirectangular `Projection`
element from the same side data. Injection is atomic and only occurs after a
successful encode.

## Depth cards (2.5D)

Any group, layer, shape, or particle emitter that declares `depth`,
`rotationX`, or `rotationY` (as attributes or animated as `depth`,
`rotation.x`, `rotation.y`) is a **depth card**: a flat plane placed in the
3D world and projected by the active camera. Nodes without them, and all
nodes of scenes without cards, render exactly as before (camera ignored).

- **Placement.** A card's plane coordinates are ordinary composition
  pixels (its ancestors' and its own 2D transforms, including physics
  poses). At `depth` d the plane point (u, v) is the world point
  (u − W/2, H/2 − v, d): x right, y up, z away from the camera, the same
  space 3D objects use under a camera. `rotationX` tips the top edge away
  from the camera and `rotationY` turns the right edge away (degrees, about
  the card's anchor, X first).
- **Recipe.** A camera at `z = -zoom` shows a card at depth 0 at its
  authored pixels. A card at depth d then scales by zoom / (zoom + d), and a
  camera move of D px shifts it by D · zoom / (zoom + d): parallax and depth
  scaling follow from camera motion alone, and yaw, pitch, roll, and zoom
  animate the whole layered scene in perspective.
- **Without a camera** the view is orthographic from the origin: untilted
  cards keep their authored pixels and depth only orders them (3D objects
  then have view depth −z).
- **As a unit.** A card renders its subtree into its own buffer and is
  composited once with its blend, opacity, and the masks of pass-through
  ancestors; children blend inside the card. Its own masks apply in its
  local space; its group `effects` and depth-of-field blur run in screen
  space after projection. Particles inside a card scale uniformly with it
(they stay screen-aligned discs or squares, not foreshortened ellipses). A card
  inside another card is a validation error, and cards require
  `mode="standard"` (reported by `--validate` too).
- **Order.** Children keep (`z`, XML order). Each run of consecutive card
  siblings is re-sorted every frame far to near by the view depth of each
  card's anchor; any non-card sibling ends a run. The 3D objects of the
  scene join the first run of cards that are direct children of
  `composition`, sorted by their centers, and are drawn one at a time among
  them; with no such run they are drawn before the scene graph, as without
  cards. Cards that must interleave with 3D objects belong directly under
  `composition`.
- **Occlusion.** Cards and 3D objects share a per-sample depth buffer
  (`antialias3d` samples). Every card sample is depth tested (a card sample
  wins a tie against what was drawn before it; 3D objects keep their strict
  test) and clipped to [`near`, `far`]; a card compositing straight
  into the frame writes its depth where its composited alpha is at least
  0.5. Opaque cards and objects therefore occlude each other per pixel, even
  when they intersect or tilt through each other. Limitations: crossing
  *translucent* cards blend in draw order, a translucent 3D material in front
  of a card hides it, and cards inside isolated groups (blend, opacity < 1,
  effects) test but never write depth. With `antialias3d` above 1, a card
  and a 3D object adjacent in draw order that share a pixel edge blend by
  resolved coverage (consecutive 3D objects resolve together).
- **Quality.** Untilted cards under a camera without yaw or pitch map by an
  exact affine transform, so text, vectors, and images keep full quality.
  Otherwise the visible part of the plane (content bounds clipped to the
  screen edges and the near and far planes) renders into a plane buffer at
  the largest on-screen magnification, capped at 4096 px a side and 8 Mpx,
  then warps to the screen with bilinear taps, up to 4 × 4 per pixel where
  the plane is minified.
- **Depth of field.** With `aperture` > 0 a card whose anchor is at view
  depth z is blurred by r = aperture · |z − focusDistance| / z px (a
  near-Gaussian with σ = r / 2, saturating at σ = 64; fractional radii blend
  smoothly). 3D objects are not blurred.

```xml
<camera id="cam" z="-1000" zoom="1000" focusDistance="1000" aperture="6">
  <animate property="position.x"><key time="0" value="0"/>
    <key time="8" value="600"/></animate>
</camera>
<layer id="sky" asset="sky" depth="6000"/>
<layer id="hills" asset="hills" depth="1500"/>
<object3D id="moon" primitive="sphere" z="4000" radius="300"/>
<layer id="trees" asset="trees" depth="0"/>
<layer id="title" asset="title" depth="-300" rotationY="-12"/>
```

## Materials, 3D, and lights

`material` requires `id` and accepts `baseColor`, `metallic`, `roughness`, and
`emissive`.

`object3D` requires `id` and `primitive="sphere|box|plane|mesh"`; it accepts
`mesh` (required for `primitive="mesh"`), `material`, position, three rotations, three
scales, `radius`, `castShadow`, and `receiveShadow`. Transform animations may
be nested. Meshes are triangle-rasterized with a shared depth buffer; spheres,
boxes, and planes are camera-facing sprites.

Top-level `lights` contains `light` entries. Each requires `id` and
`type="ambient|directional|point|spot"`, and accepts color (animatable),
intensity (0–1e6, also for animated keys), position, yaw/pitch, range,
falloff, `spotAngle` (0.5–179 degrees, default 45), `castShadow`, and
`shadowMapSize` (16–8192, default 2048). Light animation is nested normally.
Lights referenced by a 2D `lighting` effect (below) are 2D lights: they do
not light the 3D pass.

A directional or spot light with `castShadow="true"` renders a
`shadowMapSize` x `shadowMapSize` depth map each frame, seen from the light:
orthographic and fitted to the bounding sphere of all 3D objects for a
directional light, perspective over the cone (10% margin) for a spot light.
Every object with `castShadow` (default true) is written into it (a spot
map bounds each caster by the exact tangent cone of its bounding sphere):
spheres as ellipsoids, boxes and planes as the camera-facing quad that is drawn, meshes
as their triangles. Objects with `receiveShadow` (default true) sample it with
a 3 x 3 percentage-closer filter and a slope-scaled bias (1.5 texels plus 2
per unit of surface tangent, capped at 10), so shadow edges are soft over
about one texel and lit surfaces do not self-shadow; the light's direct
contribution is scaled by the visible fraction (ambient light is not
shadowed). Without a camera, world x/y are canvas pixels (y down) and the
viewer looks along -z; with a camera, world space is the camera's. Point
lights keep the earlier approximation: a caster's bounding sphere between
the point and the light scales that light by 0.2, and each `castShadow`
object draws a translucent screen-space blob when any point light casts
shadows.

## Effects

Each `effect` in top-level `effects` requires `id` and a type: `glow`,
`bloom`, `blur`, `color-grade`, `vignette`, `lens-flare`, `drop-shadow`, or
`lighting`. Common controls are `enabled`, `intensity` (default 1), `radius`
(default 4, at most 4096 px), `threshold` (0.7), `saturation` (1), `contrast` (1),
`brightness` (0), and `color` (white; black for `drop-shadow`); irrelevant
controls are ignored by a particular effect. Every numeric control and
`color` animate with nested `<animate>` (keys obey the same bounds). An
effect at zero intensity is an exact no-op. An effect never leaves a
non-finite color channel in the frame: such a channel becomes 0 (alpha is
kept).

An effect that no group lists in its `effects` attribute applies to the
whole final frame, in XML order, after the viewport projection. An effect
listed by one or more groups is *only* applied to those groups: in the
listed order, to the group's isolated buffer before it is composited (so it
inherits the group's blend, opacity, and masks), independently for each
group that lists it. Such a buffer's content area grows by each effect's
reach (blur radius; drop-shadow offset plus radius plus one pixel), so
effects are never clipped at the group's own bounds. The group's children
are drawn for its effects without the group's own masks (only the
ancestors' clip, grown by the effects' reach), and the masks cut the
result when it is composited: a shadow cast into the mask by content
outside it appears. Effect lengths
(radius, offsets) are canvas pixels; light positions and ranges of
`lighting` follow the group's transform.

`drop-shadow` accepts `offsetX`, `offsetY` (default 8, 8; each within
±1e5 px), `radius` (box
blur radius, rounded; 0 for a hard shadow), `color`, and `intensity`
(opacity, clamped to [0, 1]): the content's alpha, shifted (bilinearly for
fractional offsets) and blurred, tinted with `color`, is placed under the
content: `out = content + shadow x (1 - content alpha)`.

`lighting` multiplies the content's color (not its alpha) by the light
arriving at each pixel from the `light` ids in `lights` (space-separated):
`out = content x (1 + (L - 1) x intensity)`, where `L` sums, per light,
`color x intensity x f`. For `ambient` lights `f = 1`; for `point` and `spot`
lights at canvas position (x, y) (their `x`/`y` in the target's space: the
group's local space, or canvas pixels for a whole-frame effect)
`f = falloff(d / range)` with the effect's `falloff`: `smooth` (default)
`(1 - q^2)^2`, `linear` `1 - q`, `quadratic` `(1 - q)^2`, `none` `1`, each 0
for `q >= 1`; spot lights also multiply a smoothstep cone of full width
`spotAngle` (±5 degree soft edge) about the heading `yaw` (0 = +x, 90 =
down). `directional` lights contribute `f = 1` unless relief is on. With
`relief` > 0 each pixel gets a normal from its alpha gradient scaled by
`relief` and lights are weighted by Lambert's law: directional lights shine
along `yaw` from `pitch` degrees above the image plane, and point/spot lights
sit `z` pixels above it, so edges facing a light brighten and those facing
away darken. This is compositing light, not physically based rendering, and
it casts no shadows.

## Physics and deformation

`physics` accepts `fixedStep`, `gravityX`, `gravityY`, and an optional binary
`cache` path (relative to the scene file). `--physics-cache DIR` on the
command line overrides it: the cache is then `DIR/physics-<signature>.bin`,
named by the simulation signature, so several scenes can share one
directory. It may contain:

- directional `forceField` with `forceX/forceY`;
- radial `forceField` with `x/y`, `strength`, and `falloff`: acceleration
  `strength / (1 + d)^falloff` toward (`x`, `y`);
- vortex `forceField` with `x/y`, `strength`, and `falloff`: the same
  magnitude, tangential (clockwise on screen for positive `strength`);
- `constraint type="spring|distance"` with body IDs `a/b`, `restLength`
  (0 or absent: the initial distance), `stiffness`, and `damping`;
- `constraint type="pin"` tying body `a` to the world point `x`, `y` at
  `restLength` (absent: the initial distance; 0 holds the body on the point).
  Without `stiffness` the pin is rigid: after contacts the body is projected
  back onto the circle and loses its velocity along the pin. With `stiffness`
  it is a spring to the point, damped by `damping`.

Force-field `x`, `y`, `forceX`, `forceY`, and `strength` animate with nested
`<animate>`; fields are evaluated at each fixed step's time. Collisions use
the exact contact between a circle and an oriented box (the box's rotation
included) and a separating-axis test between oriented boxes; unrotated boxes
behave exactly as before.

A drawable node may contain `rigidBody` with static/kinematic/dynamic type,
box/circle shape, mass, friction, restitution, linear/angular damping, initial
velocities, angular velocity, and radius. Damping (per second, default
0.01) scales velocity by `exp(-damping * fixedStep)` each step, so it decays
smoothly and never reverses direction.

`softBody` on a `layer` or `shape` makes its local box a `rows` x `cols`
(2–16, default 4 x 4) grid of point masses (`mass` split evenly) joined by
structural and shear springs of `stiffness` with damping ratio `damping`,
plus an area-preserving `pressure` that pushes the outline outward when
compressed. `pin="top|bottom|left|right|corners|none"` (default none) holds
those grid nodes at their rest positions. The grid is simulated with the
rigid bodies inside the physics preparation at `fixedStep` (with the world
defaults when there is no `physics` element): world gravity and force fields
apply, grid nodes that start inside the frame collide with its bounds (0 to
the project width/height, restitution 0.2), and a node that also has a `rigidBody` anchors each grid
node to its rigid pose with a spring, so it wobbles when the body stops.
Each fixed step is integrated in as many substeps as the springs need to
stay stable (from `stiffness`, the per-node mass and `fixedStep`); a body
needing more than 4096 substeps per fixed step is rejected at validation
with a diagnostic naming `stiffness`, `mass`, and `fixedStep` (the check
assumes the rigid-body anchor spring). A simulation that still produces a
non-finite state fails the render with a diagnostic instead of recording
it, and a physics cache whose samples are not finite or exceed 1e9 in
magnitude is discarded and re-simulated.
The grid is cached with the rigid bodies, render samples interpolate between
fixed steps like rigid poses, and the node's image is warped by the grid with
the mesh-warp sampler. A node without a rigid body simulates in its static
(base) transform; animated transforms move the deformed result.

`deform` contains ordered `modifier` elements of type `bend`, `twist`, `wave`,
`squash`, `stretch`, or `mesh-warp`. Controls are `amount`, `frequency`,
`phase`, and `axis="x|y"`; controls can contain animation tracks.
`mesh-warp` takes `rows` and `cols` (2–16, default 4) and `point` children
`<point row="r" col="c" x="dx" y="dy"/>` (`x`/`y` animate) offsetting control
point (r, c) of a grid spanning the node's local box by (dx, dy) local
pixels. The content is warped by the bilinear interpolation of the offsets
(inverse-mapped per pixel by Newton iteration; when that does not converge
to a forward residual below 1e-4 px, each grid cell containing the pixel is
inverted analytically and the in-cell solution nearest the Newton start is
used, and a pixel with no source stays transparent); a grid of zero offsets
reproduces the undeformed node exactly, and the drawn area grows by the sum
of the largest offsets of every grid (mesh-warp modifiers and the soft
body compose).

## Audio

`audioMix` accepts `sampleRate` (8–384 kHz) and `channels` (1 or 2). Each
`audioTrack` requires `id` and an `asset` (an `audio` asset, or a `video`
asset whose best audio stream is used), and accepts:

| Attribute | Default | Meaning |
|---|---|---|
| `start` | 0 | scene time (s) of the track's first sample |
| `clipIn`, `clipOut` | 0, end | source range (s) that plays |
| `loop` | 0 | play count (0 and 1 both play once) |
| `volume` | 1 | gain in `[0,1]` |
| `pan` | 0 | `[-1,1]`, equal-power (stereo mixes only) |
| `fadeIn`, `fadeOut` | 0 | seconds of linear gain ramp after the start and before the end |
| `speed` | 1 | `(0,100]` source seconds per output second (varispeed) |
| `reverse` | false | each play runs from `clipOut` back to `clipIn` |

`start`, `clipIn`, `clipOut`, `fadeIn` and `fadeOut` are at most `1e7`
seconds; larger values are rejected with a diagnostic.

Every asset is decoded once, in memory, with libswresample to the mix rate and
channel count (clips longer than 4 hours are rejected), and shared by all its
tracks. Decoded samples follow the file's timestamps on the same timeline as
its video (time zero is the container start, codec priming excluded): a late
start and any timestamp gap longer than 20 ms become silence, an overlap
longer than 20 ms drops the overlapping start of the later audio, and
smaller jitter is ignored. All times convert to samples with `round(t × rate)`, and the mixer is
sample-exact: video frame `n` covers samples
`[floor(n × rate × fps_den / fps_num), floor((n+1) × rate × fps_den / fps_num))`
(exact integer arithmetic), so a range render's audio track has exactly the
selected frames' sample count. A track plays `(clipOut − clipIn) × loop`
source seconds, which last that long divided by `speed` in output time; the
fade-out ends at the track's last output sample. Pan gains are
`√2·cos θ` / `√2·sin θ` with `θ = (pan + 1)·π/4`: `pan="0"` is unity on both
channels, the summed power of the two gains is constant, and a mono source
(upmixed at −3 dB) panned fully to one side reaches unity there. `speed ≠ 1`
reads the source with linear interpolation, like tape (pitch follows speed);
it is not band-limited, so speeds above 1 can alias. The mix is clipped to
`[-1,1]`.

## Colors and time

Colors are `#RRGGBB`, `#RRGGBBAA`, or normalized `r,g,b[,a]`, written as
transfer-encoded values of the project working space. Named color spaces are
sRGB, Rec.709, Display-P3, and Rec.2020. Assets are decoded from their source
space into the blend space (the working gamut, linear light when
`linearLight="true"`, premultiplied float), blending follows the W3C
separable formulas there (`add` is not clamped until output), and the final
frame is converted to 8-bit (16-bit for pixel formats deeper than 8 bits) in
the output space with matching primaries/transfer/matrix/range tags. Times and durations are decimal
seconds. Frames are selected on a half-open interval; frame N occurs exactly
at `N × fps_den / fps_num`.
