# Text projection and caption rules

## Glyph projection

The animator selects a unit and accumulates its property offsets as before. Each glyph is scaled about its unit pivot plus anchor offset, rotated first about X and then Y, translated in Z by `zDepth`, then skewed and rotated in the XY plane and translated into its text-box position. Positive X rotation takes the top edge away from the viewer; positive Y rotation takes the right edge away. The X rotation maps `(x,y,0)` to `(x,y cos X,-y sin X)`; the following Y rotation maps this to `(x cos Y + y sin X sin Y, y cos X, x sin Y - y sin X cos Y)`. Angles are degrees.

For a layer placed in 3D, let K map text-box `(x,y,z,1)` through the asset fit, node world transform, the layer's 3D placement, enclosing 3D planes and the resolved camera to `(clipX,clipY,clipW)`. Let H consist of K's x, y and constant columns (the flat text plane). The glyph is drawn at the homogeneous point `H^-1 K (x,y,z,1)`, divided by its third coordinate, in the original text box; compositing the layer through H then puts it at precisely the camera projection. This uses the actual camera pose, projection, parenting and shake, rather than only its field of view. Orthographic cameras apply their affine projection without perspective shrinking. An edge-on, singular plane falls back to the default glyph perspective before the layer's own projection.

Ordinary 2D text without a containing 3D plane uses the engine's default camera: horizontal field of view 60 degrees, eye `(W/2,H/2,-d)`, looking along +Z, with `d = W/(2 tan 30 degrees)` in project pixels. A frame camera does not affect ordinary 2D text. In that frame, a point `(x,y,z)` projects to `(W/2 + (x-W/2)d/(d+z), H/2 + (y-H/2)d/(d+z))`. For numerical safety, homogeneous divisors normalized to 1 on the unturned text plane are bounded below by 0.001. This bounds geometry crossing the projection pole; it is not a visibility or depth-buffer test.

Outlines, COLR geometry, strokes, shadows, underline/overline/strikeout geometry and glyph highlights are projected together. Bitmap glyphs use homogeneous vertex weights for perspective-correct texture interpolation. Curves are flattened at the renderer's text tolerance before projection. Paragraph backgrounds remain in the text box. Glyph blur is applied after glyph projection. Text-path placement is applied in the glyph plane before camera projection.

## Independent range easing

For falloff v in [0,1], set L = clamp(easeLow/100,0,1) and H = clamp(easeHigh/100,0,1). The result is `v + H(v²-v³) - L(v³-2v²+v)`. Thus the low endpoint slope is 1-L and the high endpoint slope is 1-H. The final selector amount scales this result. Presets keep their preset timing curves.

## Caption placement and emphasis

Cue position is the WebVTT settings string. VTT sidecars preserve its tokens with normalized whitespace. Burning supports `line:N%` as the block's top edge and `position:N%` as its horizontal centre, percentages in [0,100] of the project frame. The first valid value of each setting wins. Unsupported tokens and comma alignment suffixes produce render notes; the percentage part remains usable. Other settings do not affect burning. SRT has no cue-settings field.

Karaoke, highlight, pop and enlarge paint emphasised words with the activeStyle fill, else activeColor, else #FFD400, for the whole page. Other presets set them in at least weight 700, retaining the track style otherwise. Caption z is an integer in the top-level composition stack, including evaluated animated node z. Equal-z composition entries paint before captions, and equal-z caption tracks retain document order. Groups and effect-isolated pictures retain their boundaries; a child's z is local to its parent and does not escape that group. The default 1000000 is high, not infinity: higher-z entries cover captions.

## Human labels

The shared `sr_model::diag::label` rule trims name, falls back to id, then to `element[index]` with a one-based index among same-tag siblings. Labels serve people; IDs, structural paths and checkpoint resume keys remain locators. A checkpoint store writes a deterministic `.labels.json` companion keyed by structural path, so duplicate names cannot collide. Text animators and puppet pins are stateless and have no individual resume state.
