# Golden reference images

`tests/unit/test_golden.c` (CTest `unit.golden`) renders every scene in this
directory at two or three chosen frames and compares each byte of the
decoded frame with `expected/<scene>-fNNN.png`. Frames are rendered through
`sr_render` in preview mode, the path of

```sh
scene-render --scene tests/golden/<scene>.xml --frame N --preview-out F.png
```

so the reference is the 8-bit straight RGBA the output color conversion
produces (the same bytes the encoder would receive for an 8-bit pixel
format). Each frame is rendered twice: with 1 thread on a freshly loaded
scene, and with 4 threads on a scene that has first rendered another of its
frames, so neither the thread count nor state left by an earlier frame
(asset caches, video decoders, physics samples) may change a single byte.
On mismatch the test reports how many bytes differ and the largest
difference, and keeps the rendered frame in `BUILD/test_tmp/golden/`.

| Scene | Covers | Frames |
|---|---|---|
| `composite.xml` | all six blend modes in linear light, opacity/rotation/fill keys, cubic-bezier key, z order | 0, 12, 23 |
| `groups-masks.xml` | nested isolated groups (blend, opacity), pass-through group with an animated rounded-rect mask and an animated inverted mask, masked shape | 0, 12, 23 |
| `paths.xml` | cubic/quadratic/relative path commands, evenodd vs nonzero, open and closed strokes, stroked rect/ellipse vectors and shapes | 0, 12, 23 |
| `images.xml` | 8x8 image magnified 6x to 17.3x, rotated, animated scale; a 1904x1120 PNG resampled to its declared size; edges at the canvas border | 0, 12, 23 |
| `video.xml` | one H.264 clip on four layers: plain, speed 2 with loop, reversed, `source.time` remap | 0, 12, 23 |
| `text.xml` | vendored Inter fonts only: wrap, justify, centre, end, letter spacing, line feeds, vertical alignment, right-to-left paragraphs | 0, 23 |
| `text-scripts.xml` | Hebrew and Arabic shaping and bidi through Fontconfig `sans` (see below) | 0, 11 |
| `equirect.xml` | 2:1 equirectangular canvas, a shape crossing the seam, particles | 0, 12, 23 |
| `viewport.xml` | perspective viewport of a 256x128 panorama with animated yaw (across the seam), pitch, roll and FOV | 0, 12, 23 |
| `scene3d.xml` | plane, OBJ mesh, box and sphere; shadow-mapped directional and spot lights, point light, 2x2 supersampling | 0, 12, 23 |
| `fx.xml` | group drop shadow (animated offset/color) and grade, 2D lighting with relief, group blur, whole-frame glow, bloom, lens flare, vignette | 0, 12, 23 |
| `particles.xml` | parametric emitter with animated rate/direction, square particles with color keys, the four presets | 0, 12, 23 |
| `physics.xml` | static floor and rotated platform, bouncing circles, spinning box, pin and spring constraints, vortex and wind fields, pinned and free soft bodies | 0, 12, 23 |
| `deform.xml` | mesh-warp with animated points, bend, twist, wave, squash, stretch | 0, 12, 23 |
| `curves.xml` | hold/steps, all ten Penner families, Catmull-Rom, TCB, spring and temporal Bezier handles | 0, 8, 20 |
| `tracks.xml` | all extrapolation modes, additive positions/colours, normalized/local clocks, looping particle emission and spring lifetimes | 0, 12, 20 |
| `material-animation.xml` | shared base color/alpha, emissive, metallic and roughness animation with shadows | 0, 12, 20 |
| `lengths.xml` | relative lengths, group scopes, mixed-unit motion, host-local masks and physics | 0, 12, 23 |
| `styles.xml` | load-time color aliases in project and animated material colors, equivalent to `material-animation.xml` | 0, 12, 20 |

The scenes are at most 320x180 and 24 frames. Assets come from
`examples/assets/` and `assets/third-party/`.

The new `curves` and `tracks` references capture the schema 1.1 interpolation
families and track options, including cached particle-rate evaluation. Their
six frames were visually reviewed; existing references are unchanged.

The three `material-animation` references capture the newly animatable
material colors, transparency, metallic and roughness. They were visually
reviewed at frames 0, 12 and 20; all previous references remain unchanged.

The three `styles` references are byte-identical copies of the existing
`material-animation` references. They verify that token aliases produce the
same pixels as literal colors. Frames 0, 12 and 20 were visually reviewed;
no existing reference was regenerated.

## Determinism scope

The references are exact for one toolchain and library set: they were made
in the Freedesktop SDK 25.08 (x86-64, GCC 15.2.0, glibc 2.42, FFmpeg 7.1.3
libraries, FreeType 2.14.3, HarfBuzz 11.4.5, FriBidi 1.0.16, Fontconfig
2.17.1, libxml2 2.14.6), and they match in the Release build (`-O3`), the
Debug build and the sanitizer build. The build disables floating-point
contraction (`-ffp-contract=off -fno-fast-math`), so the compiler cannot fuse
multiply-adds and change rounding; frame times are computed from the integer
frame index; parallel work writes disjoint rows. What can still change the
bytes:

- another compiler version or architecture (code generation of
  transcendental calls, vectorization of reductions), or another libm;
- another FreeType or HarfBuzz (glyph outlines, shaping, positions), or other
  font files;
- another libavcodec/libswscale (video decoding of `video.xml`, Lanczos
  resampling of the PNG in `images.xml`, PNG decoding).

`text-scripts.xml` also depends on the fonts Fontconfig resolves: the test
runs it only when `sans` resolves to `DejaVuSans.ttf` (the reference font)
and prints a `skip:` line otherwise. All other scenes name their font files
explicitly.

In another environment, regenerate the references there and review them
instead of loosening the comparison.

## Regenerating and reviewing

Regenerate only when a rendering change is intended:

```sh
SR_UPDATE_GOLDEN=1 build/sr-unit-tests golden
```

This rewrites `expected/*.png` from the 1-thread renders (the 4-thread
renders are still compared against the new files). Then review every
changed image before committing it: `git status tests/golden/expected`
lists them, and each can be opened next to its previous version
(`git show HEAD:tests/golden/expected/NAME.png > /tmp/old.png`). A changed
reference needs the reason in the commit message.

A new scene needs a `GOLDEN(...)` line and a table entry in
`tests/unit/test_golden.c`, then a run with `SR_UPDATE_GOLDEN=1` to create
its references.

`tests/golden.sha256` is separate: it holds SHA-256 hashes of the PPM
previews checked by `tests/run-integration.sh`.

`lengths.xml` adds references for scoped relative lengths: mixed-unit motion,
mask dimensions, partial/unsized groups, media, particles and prepared physics.
The three new PNGs cover frames 0, 12 and 23; existing references are unchanged.
