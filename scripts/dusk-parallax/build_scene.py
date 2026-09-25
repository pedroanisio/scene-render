#!/usr/bin/env python3
"""Write examples/dusk-parallax.xml, a 48 s explainer about scene-render.

The film is set in a five-plane pixel-art mountain range at dusk (CC0 art by
ansimuz, CC0 fog by GrumpyDiamond; see assets/third-party/dusk-parallax).
Depth comes from parallax: every plane is a group whose inner "track" slides
by  -factor * u(t)  and whose outer group zooms by  1 + zoom * z(t)  around a
shared pivot, so nearer planes move and grow faster than far ones. An
opening tilt shifts the planes vertically by  factor * w(t)  in the same way.

Story beats (seconds):
   0-7    tilt down from the sky, title
   7-15   "A film is a text file": the XML that drives the forest plane
  15-25   parallax explained with tags pinned to each plane
  25-34   the per-frame pipeline, parse to FFmpeg
  34-41   built-in features, tied to what is on screen
  41-48   determinism, end card with credits

Text widths are measured with fontTools (pip install fonttools) so centred
labels are placed exactly. Run prepare_assets.py once before rendering.

  python3 scripts/dusk-parallax/build_scene.py
  scene-render --scene examples/dusk-parallax.xml --threads auto
"""
import math
import os
import re
from xml.sax.saxutils import escape

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
OUT = os.path.join(ROOT, "examples", "dusk-parallax.xml")
W, H, FPS, DURATION = 1920, 1080, 30, 48.0
SEED = 20260925

INTER = os.path.join(ROOT, "assets", "third-party", "inter")
FONTS = {"light": "Inter-Light.ttf", "regular": "Inter-Regular.ttf",
         "medium": "Inter-Medium.ttf", "semibold": "Inter-SemiBold.ttf"}
MONO_FONT = "DejaVu Sans Mono"
MONO_FILE = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf"
ART = "../assets/third-party/dusk-parallax"

# Camera path, in foreground pixels. u: horizontal travel, z: dolly zoom,
# w: opening tilt (planes start pushed down and rise into place).
PAN = [(0, 0), (7, 140), (15, 460), (25, 1560), (34, 1900), (41, 2140),
       (48, 2400)]
ZOOM = [(0, 0), (7, 0.03), (25, 0.09), (48, 0.2)]
TILT = [(0, 430), (6.5, 0)]
PIVOT = (960, 640)

# id -> (asset, png size, tile origin, tiles, pan factor, zoom factor, z)
PLANES = [
    ("sky", "sky", (2176, 1280), (-920, -100), 2, 0.02, 0.05, 0),
    ("farPeaks", "farPeaks", (1904, 1120), (-60, -40), 3, 0.10, 0.2, 10),
    ("mountains", "mountains", (3808, 1120), (-80, -40), 2, 0.25, 0.4, 20),
    ("forest", "forest", (3808, 1120), (-80, -40), 2, 0.55, 0.7, 30),
    ("foreground", "foreground", (3808, 1120), (-80, -40), 2, 1.0, 1.0, 40),
]
PLANE_FACTOR = {p[0]: p[5] for p in PLANES}

out = []


def emit(line=""):
    out.append(line)


def fmt(v):
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, float):
        s = f"{v:.4f}".rstrip("0").rstrip(".")
        return "0" if s == "-0" else s
    return str(v)


def attrs(**kw):
    return " ".join(f'{k.rstrip("_")}="{escape(fmt(v), {chr(34): "&quot;"})}"'
                    for k, v in kw.items() if v is not None)


def animate(prop, keys, interp="ease-in-out", indent="      "):
    """keys: list of (time, value) or (time, value, interpolation)."""
    emit(f'{indent}<animate property="{prop}" defaultInterpolation="{interp}">')
    for key in sorted(keys, key=lambda k: k[0]):
        extra = f' interpolation="{key[2]}"' if len(key) > 2 else ""
        emit(f'{indent}  <key time="{fmt(float(key[0]))}" '
             f'value="{fmt(float(key[1]))}"{extra}/>')
    emit(f"{indent}</animate>")


def lerp_keys(keys, t):
    for (t0, v0), (t1, v1) in zip(keys, keys[1:]):
        if t0 <= t <= t1:
            return v0 + (v1 - v0) * (t - t0) / (t1 - t0)
    return keys[-1][1] if t > keys[-1][0] else keys[0][1]


def fade(t_in, t_out, d=0.6, peak=1.0):
    return [(t_in, 0), (t_in + d, peak), (t_out - d, peak), (t_out, 0)]


# ---- text measurement -----------------------------------------------------

_metrics = {}


def _font(path):
    if path not in _metrics:
        from fontTools.pens.boundsPen import BoundsPen
        from fontTools.ttLib import TTFont
        font = TTFont(path)
        cmap = font.getBestCmap()
        hmtx = font["hmtx"]
        glyphs = font.getGlyphSet()
        tops = {}
        for cp, g in cmap.items():
            pen = BoundsPen(glyphs)
            glyphs[g].draw(pen)
            tops[cp] = pen.bounds[3] if pen.bounds else 0
        _metrics[path] = ({cp: hmtx[g][0] for cp, g in cmap.items()},
                          font["head"].unitsPerEm, tops)
    return _metrics[path]


def _path(weight):
    return MONO_FILE if weight == "mono" else os.path.join(INTER, FONTS[weight])


def text_top(text, size, weight):
    """Height of the tallest glyph above the baseline, in pixels.

    drawtext (x=0:y=0) puts the top of the text's own tallest glyph at y=0,
    so pieces set on one line must be shifted down by the difference.
    """
    _, upem, tops = _font(_path(weight))
    return max(tops.get(ord(c), 0) for c in text) * size / upem


def text_width(text, size, weight):
    advances, upem, _ = _font(_path(weight))
    missing = [c for c in text if ord(c) not in advances]
    if missing:
        raise SystemExit(f"{weight} has no glyph for {missing!r} in {text!r}")
    return sum(advances[ord(c)] for c in text) * size / upem


TEXTS = {}  # id -> (text, size, weight, colour, width)


def text(aid, value, size, weight, color):
    """Register a text asset; returns its measured width in pixels."""
    width = math.ceil(text_width(value, size, weight)) + 8
    if aid in TEXTS and TEXTS[aid][:4] != (value, size, weight, color):
        raise SystemExit(f"text id {aid} reused with different content")
    TEXTS[aid] = (value, size, weight, color, width)
    return width


def text_height(size):
    return math.ceil(size * 1.3)


# ---- scene pieces ---------------------------------------------------------

SHADOW = "#12071CFF"


def label(node_id, aid, value, size, weight, color, x, y, z=0,
          center=False, shadow=True, indent="      "):
    """A text layer, optionally centred on x, with a soft drop shadow."""
    width = text(aid, value, size, weight, color)
    left = x - width / 2 if center else x
    if shadow:
        text(aid + "Sh", value, size, weight, SHADOW)
        emit(f'{indent}<layer {attrs(id=node_id + "Sh", asset=aid + "Sh", x=round(left + size * 0.04, 1), y=round(y + size * 0.06, 1), z=z, opacity=0.55)}/>')
    emit(f'{indent}<layer {attrs(id=node_id, asset=aid, x=round(left, 1), y=y, z=z)}/>')
    return width


def panel(node_id, x, y, w, h, radius, fill, stroke=None, stroke_width=None,
          z=0, indent="      ", extra=None):
    """A rounded rectangle: a masked group around a rect shape."""
    emit(f'{indent}<group {attrs(id=node_id, x=x, y=y, z=z)}>')
    emit(f'{indent}  <mask {attrs(type="rounded-rect", x=0, y=0, width=w, height=h, radius=radius)}/>')
    emit(f'{indent}  <shape {attrs(id=node_id + "Fill", shape="rect", width=w, height=h, fill=fill, stroke=stroke, strokeWidth=stroke_width)}/>')
    if extra:
        extra(indent + "  ")
    emit(f"{indent}</group>")


def section(node_id, start, end, z, body, rise=24):
    """A timed group that fades in/out and settles upward as it appears."""
    emit(f'    <group {attrs(id=node_id, z=z, start=start, end=end)}>')
    animate("opacity", fade(start, end, 0.7), indent="      ")
    animate("position.y", [(start, rise), (start + 0.9, 0)], "ease-out",
            indent="      ")
    body()
    emit("    </group>")


def plane_offset(factor, t):
    return -factor * lerp_keys(PAN, t)


# ---- XML snippet shown on screen (true to this file) ----------------------

FOREST_END = round(plane_offset(PLANE_FACTOR["forest"], DURATION))
SNIPPET = [
    '<scene version="1.0">',
    f'  <project width="{W}" height="{H}" fps="{FPS}"/>',
    '  <assets>',
    '    <image id="forest" src="forest.png"/>',
    '  </assets>',
    '  <composition>',
    '    <group id="forestTrack" z="30">',
    '      <animate property="position.x">',
    '        <key time="0" value="0"/>',
    f'        <key time="48" value="{FOREST_END}"/>',
    '      </animate>',
]
TOKEN = re.compile(r'(?P<open></?)(?P<tag>\w+)|(?P<attr>[\w.]+)(?P<eq>=)'
                   r'(?P<val>"[^"]*")|(?P<close>/?>)|(?P<sp>\s+)')
TOKEN_COLORS = {"open": "#9C87BFFF", "close": "#9C87BFFF", "eq": "#9C87BFFF",
                "tag": "#FF9EC7FF", "attr": "#CBB8FFFF", "val": "#FFD48AFF"}
MONO_SIZE = 25
LINE_H = 40


def xml_card():
    advance = text_width("M", MONO_SIZE, "mono")
    card_w, card_h = 1010, 120 + LINE_H * len(SNIPPET)

    def body(ind):
        emit(f'{ind}<layer {attrs(id="cardFileL", asset="cardFile", x=40, y=26)}/>')
        emit(f'{ind}<shape {attrs(id="cardRule", shape="rect", width=card_w - 80, height=2, x=40, y=78, fill="#F2A6C844")}/>')
        count = 0
        for row, line in enumerate(SNIPPET):
            col = 0
            appear = 8.0 + row * 0.22
            emit(f'{ind}<group {attrs(id=f"code{row}", x=40, y=100 + row * LINE_H)}>')
            animate("opacity", [(appear, 0), (appear + 0.3, 1)], "linear",
                    indent=ind + "  ")
            pieces = []
            for m in TOKEN.finditer(line):
                for kind, part in m.groupdict().items():
                    if part is None:
                        continue
                    if kind != "sp":
                        pieces.append((col, part, kind))
                    col += len(part)
            top = max(text_top(p, MONO_SIZE, "mono") for _, p, _ in pieces)
            for pcol, part, kind in pieces:
                aid = f"tok{count}"
                count += 1
                text(aid, part, MONO_SIZE, "mono", TOKEN_COLORS[kind])
                dy = top - text_top(part, MONO_SIZE, "mono")
                emit(f'{ind}  <layer {attrs(id=aid + "L", asset=aid, x=round(pcol * advance, 2), y=round(dy, 2))}/>')
            if col != len(line):
                raise SystemExit(f"snippet tokenizer lost text in {line!r}")
            emit(f"{ind}</group>")

    text("cardFile", "examples/dusk-parallax.xml", 22, "mono", "#B79AD9FF")
    panel("xmlCard", 90, 190, card_w, card_h, 22, "#140A1EE6",
          "#F2A6C866", 2, indent="      ", extra=body)


def plane(pid, asset, size, origin, tiles, factor, zoom, z, extras=()):
    px, py = PIVOT
    emit(f'    <group {attrs(id=pid + "Plane", x=px, y=py, anchorX=px, anchorY=py, z=z)}>')
    zkeys = [(t, 1 + zoom * v) for t, v in ZOOM]
    animate("scale.x", zkeys, "linear")
    animate("scale.y", zkeys, "linear")
    emit(f'      <group {attrs(id=pid + "Track")}>')
    animate("position.x", [(t, -factor * u) for t, u in PAN], "linear",
            indent="        ")
    animate("position.y", [(t, factor * v) for t, v in TILT],
            indent="        ")
    for i in range(tiles):
        emit(f'        <layer {attrs(id=f"{pid}Tile{i}", asset=asset, x=origin[0] + i * size[0], y=origin[1])}/>')
    for extra in extras:
        extra()
    emit("      </group>")
    emit("    </group>")


def fog(fid, factor, y, opacity, z, count, spacing, scale):
    """Screen-blended haze between planes, drifting with its own depth."""
    px, py = PIVOT
    emit(f'    <group {attrs(id=fid + "Plane", x=px, y=py, anchorX=px, anchorY=py, z=z)}>')
    emit(f'      <group {attrs(id=fid + "Track")}>')
    animate("position.x", [(t, -factor * u) for t, u in PAN], "linear",
            indent="        ")
    animate("position.y", [(t, factor * v) for t, v in TILT],
            indent="        ")
    for i in range(count):
        emit(f'        <layer {attrs(id=f"{fid}{i}", asset="fog", x=-500 + i * spacing, y=y, scaleX=scale, scaleY=scale * 0.8, opacity=opacity, blend="screen")}>')
        emit("          <deform>")
        emit(f'            <modifier {attrs(type="wave", amount=18, frequency=1.5, phase=i * 1.7, axis="y")}>')
        animate("phase", [(0, i * 1.7), (DURATION, i * 1.7 + 6.2832)],
                "linear", indent="              ")
        emit("            </modifier>")
        emit("          </deform>")
        emit("        </layer>")
    emit("      </group>")
    emit("    </group>")


def fireflies(pid, count, y_range, size, rng_seed):
    """Stateless scattered emitters; they swell during the feature beat."""
    def extra():
        state = rng_seed
        for i in range(count):
            state = (state * 6364136223846793005 + 1442695040888963407) % 2**64
            fx = 40 + (state >> 33) % 3500
            fy = y_range[0] + (state >> 13) % (y_range[1] - y_range[0])
            emit(f'        <particleEmitter {attrs(id=f"{pid}Fly{i}", preset="dust", x=fx, y=fy, rate=0.9, lifetime=3.6, speed=110, spread=180, size=size, color="#FFE9A8E6", blend="add")}>')
            animate("rate", [(0, 0.9), (33.5, 0.9), (36, 3.2), (41, 3.2),
                             (44, 1.4)], indent="          ")
            emit("        </particleEmitter>")
    return extra


def plane_tag(pid, name, factor_text, screen_x, screen_y, t_show):
    """A pill pinned inside a plane's track, so it slides with that plane."""
    factor = PLANE_FACTOR[pid]
    x = screen_x - plane_offset(factor, t_show)
    nw = text(f"tag{pid}Name", name, 26, "semibold", "#FFFFFFFF")
    fw = text(f"tag{pid}Factor", factor_text, 26, "medium", "#FFD48AFF")
    w, h = 58 + nw + fw, 54
    top = max(text_top(name, 26, "semibold"), text_top(factor_text, 26, "medium"))
    ny = 10 + top - text_top(name, 26, "semibold")
    fy = 10 + top - text_top(factor_text, 26, "medium")

    def extra():
        emit(f'        <group {attrs(id=f"tag{pid}", x=round(x), y=screen_y, start=t_show - 0.3, end=25.2)}>')
        animate("opacity", fade(t_show - 0.3, 25.2, 0.5), indent="          ")

        def body(ind):
            emit(f'{ind}<shape {attrs(id=f"tag{pid}Dot", shape="ellipse", width=14, height=14, x=20, y=20, fill="#FFD48AFF")}/>')
            emit(f'{ind}<layer {attrs(id=f"tag{pid}NameL", asset=f"tag{pid}Name", x=44, y=round(ny, 2))}/>')
            emit(f'{ind}<layer {attrs(id=f"tag{pid}FactorL", asset=f"tag{pid}Factor", x=44 + nw, y=round(fy, 2))}/>')
        panel(f"tag{pid}Pill", 0, 0, w, h, 27, "#140A1ED9", "#FFD48A88", 2,
              indent="          ", extra=body)
        emit("        </group>")
    return extra


def bird_flock():
    emit(f'    <group {attrs(id="flock", z=5, start=29, end=47)}>')
    animate("position.x", [(29, -260), (47, 2150)], "linear")
    animate("position.y", [(29, 330), (38, 230), (47, 280)])
    formation = [(0, 0, 1.0), (-70, 34, 0.8), (-60, -30, 0.85),
                 (-140, 60, 0.7), (-150, -8, 0.65), (-215, 30, 0.6)]
    for i, (bx, by, s) in enumerate(formation):
        emit(f'      <layer {attrs(id=f"bird{i}", asset="bird", x=bx, y=by, anchorX=32, anchorY=12, scaleX=s, scaleY=s)}>')
        phase = i * 0.07
        keys = []
        t, k = 29.0 + phase, 0
        while t < 47:
            keys.append((round(t, 3), s * (1.0 if k % 2 == 0 else 0.35)))
            t, k = t + 0.24, k + 1
        animate("scale.y", keys, indent="        ")
        emit("      </layer>")
    emit("    </group>")


def title_beat():
    def body():
        label("titleL", "titleT", "scene-render", 150, "semibold",
              "#FFF6EEFF", W / 2, 470, center=True)
        label("subL", "subT", "a deterministic video engine, written in C17",
              44, "light", "#FBE3F0FF", W / 2, 668, center=True)
        emit(f'      <group {attrs(id="tagline", start=2.6, end=6.9)}>')
        animate("opacity", fade(2.6, 6.9, 0.6), indent="        ")
        label("taglineL", "taglineT",
              "Nothing here was filmed. Every frame comes from one XML file.",
              32, "regular", "#FFD48AFF", W / 2, 750, center=True,
              indent="        ")
        emit("      </group>")
    section("titleBeat", 0.8, 6.9, 60, body, rise=40)


def xml_beat():
    def body():
        xml_card()
        rx = 1170
        label("fileHead", "fileHeadT", "A film is a text file.", 58,
              "semibold", "#FFFFFFFF", rx, 250)
        for i, line in enumerate(["Assets, layers, keyframes, effects",
                                  "and audio are declared in XML,",
                                  "validated line by line, then",
                                  "rendered one frame at a time."]):
            label(f"filePara{i}", f"fileParaT{i}", line, 32, "regular",
                  "#EFE2F7FF", rx, 360 + i * 46)
        label("fileNote", "fileNoteT", "This card shows the real track",
              28, "regular", "#FFD48AFF", rx, 580)
        label("fileNote2", "fileNoteT2", "moving the forest behind it.",
              28, "regular", "#FFD48AFF", rx, 620)
    section("xmlBeat", 7.4, 15.2, 60, body)


def parallax_beat():
    def body():
        label("pxHead", "pxHeadT",
              "Depth is just layers moving at different speeds", 54,
              "semibold", "#FFFFFFFF", W / 2, 70, center=True)
        label("pxSub", "pxSubT",
              "The nearer the plane, the faster it slides past the camera.",
              32, "regular", "#FBE3F0FF", W / 2, 150, center=True)
    section("parallaxBeat", 15.6, 25.0, 60, body)


STEPS = [
    ("Parse", ["Expat + schema", "line-exact errors"]),
    ("Resolve", ["shared assets", "stable z-order"]),
    ("Time", ["t = frame ÷ fps", "never accumulated"]),
    ("Simulate", ["fixed-step", "rigid & soft bodies"]),
    ("Render", ["layers · 3D · text", "particles · lights"]),
    ("Composite", ["6 blend modes", "masks · effects"]),
    ("Encode", ["RGBA + PCM", "piped to FFmpeg"]),
]


def pipeline_beat():
    bw, bh, gap, by = 226, 170, 32, 400
    x0 = (W - (len(STEPS) * bw + (len(STEPS) - 1) * gap)) / 2
    travel = (27.0, 32.4)

    def body():
        label("pipeHead", "pipeHeadT", "How one frame is made", 58,
              "semibold", "#FFFFFFFF", W / 2, 230, center=True)
        centers = [x0 + i * (bw + gap) + bw / 2 for i in range(len(STEPS))]
        emit(f'      <shape {attrs(id="pipeTrack", shape="rect", width=round(centers[-1] - centers[0]), height=3, x=round(centers[0]), y=by + bh + 40, fill="#B79AD966")}/>')
        for i, (name, detail) in enumerate(STEPS):
            bx = round(x0 + i * (bw + gap))
            appear = 25.4 + i * 0.16
            t_hit = travel[0] + i * (travel[1] - travel[0]) / (len(STEPS) - 1)
            emit(f'      <group {attrs(id=f"step{i}", x=bx, y=by)}>')
            animate("opacity", [(appear, 0), (appear + 0.4, 1)], "linear",
                    indent="        ")
            animate("position.y", [(appear, by + 26), (appear + 0.6, by)],
                    "ease-out", indent="        ")

            def box(ind, i=i, name=name, detail=detail, t_hit=t_hit):
                emit(f'{ind}<shape {attrs(id=f"step{i}Glow", shape="rect", width=bw, height=bh, fill="#FFD48A30", stroke="#FFD48AFF", strokeWidth=6)}>')
                animate("opacity", [(0, 0), (t_hit - 0.25, 0), (t_hit, 1),
                                    (t_hit + 0.9, 0.45)], indent=ind + "  ")
                emit(f"{ind}</shape>")
                nw = text(f"step{i}NameT", name, 32, "semibold", "#FFFFFFFF")
                emit(f'{ind}<layer {attrs(id=f"step{i}Name", asset=f"step{i}NameT", x=round((bw - nw) / 2, 1), y=round(26 + text_top("Hd", 32, "semibold") - text_top(name, 32, "semibold"), 2))}/>')
                for j, line in enumerate(detail):
                    dw = text(f"step{i}D{j}T", line, 21, "regular",
                              "#D9C8EAFF")
                    if dw > bw - 12:
                        raise SystemExit(f"step detail too wide: {line!r}")
                    emit(f'{ind}<layer {attrs(id=f"step{i}D{j}", asset=f"step{i}D{j}T", x=round((bw - dw) / 2, 1), y=88 + j * 32)}/>')
            panel(f"step{i}Box", 0, 0, bw, bh, 20, "#1A1026EB", "#B79AD9AA",
                  3, indent="        ", extra=box)
            emit("      </group>")
            if i:
                emit(f'      <shape {attrs(id=f"arrow{i}", shape="rect", width=gap - 12, height=4, x=round(bx - gap + 6), y=by + bh / 2 - 2, fill="#B79AD9CC")}/>')
        emit(f'      <shape {attrs(id="pulse", shape="ellipse", width=30, height=30, anchorX=15, anchorY=15, x=round(centers[0]), y=by + bh + 41, fill="#FFE9A8FF", blend="add")}>')
        animate("position.x", [(travel[0], centers[0]),
                               (travel[1], centers[-1])], "linear",
                indent="        ")
        animate("opacity", [(travel[0] - 0.4, 0), (travel[0], 1),
                            (travel[1], 1), (travel[1] + 0.8, 0)], "linear",
                indent="        ")
        emit("      </shape>")
        label("pipeFoot", "pipeFootT",
              "Each frame is computed from its index alone, so any range "
              "renders in any order.", 30, "regular", "#FBE3F0FF", W / 2,
              by + bh + 110, center=True)
    section("pipelineBeat", 25.2, 33.8, 60, body)


CHIPS = [["UTF-8 text", "vector paths", "OBJ meshes", "masks",
          "6 blend modes"],
         ["particles", "rigid & soft bodies", "deformers", "360° video",
          "audio mix"]]


def features_beat():
    def body():
        label("featHead", "featHeadT", "Built into one C17 engine", 58,
              "semibold", "#FFFFFFFF", W / 2, 250, center=True)
        k = 0
        for r, row in enumerate(CHIPS):
            widths = [text(f"chip{r}_{c}T", s, 28, "medium", "#FFF6EEFF") + 40
                      for c, s in enumerate(row)]
            gap = 18
            x = (W - sum(widths) - gap * (len(row) - 1)) / 2
            for c, s in enumerate(row):
                appear = 34.5 + k * 0.12
                k += 1
                cw = widths[c]
                emit(f'      <group {attrs(id=f"chip{r}_{c}", x=round(x + cw / 2), y=400 + r * 80, anchorX=round(cw / 2), anchorY=29)}>')
                animate("opacity", [(appear, 0), (appear + 0.3, 1)], "linear",
                        indent="        ")
                for axis in ("x", "y"):
                    animate(f"scale.{axis}", [(appear, 0.7),
                                              (appear + 0.45, 1)],
                            "ease-out", indent="        ")

                def chip(ind, r=r, c=c, s=s):
                    dy = text_top("Hd", 28, "medium") - text_top(s, 28, "medium")
                    emit(f'{ind}<layer {attrs(id=f"chip{r}_{c}L", asset=f"chip{r}_{c}T", x=20, y=round(11 + dy, 2))}/>')
                panel(f"chip{r}_{c}Pill", 0, 0, cw, 58, 29, "#2A1838E6",
                      "#FF9EC7AA", 2, indent="        ", extra=chip)
                emit("      </group>")
                x += cw + gap
        label("featFoot", "featFootT",
              "The birds are vector paths, the fireflies are particles, the "
              "haze is a wave deformer.", 30, "regular", "#FFD48AFF", W / 2,
              600, center=True)
    section("featuresBeat", 34.2, 41.0, 60, body)


def determinism_beat():
    def body():
        label("detHead", "detHeadT",
              "Same XML + same seed = byte-identical frames", 56,
              "semibold", "#FFFFFFFF", W / 2, 400, center=True)
        label("detSub", "detSubT",
              "Golden tests hash lossless frames and compare 1-thread "
              "with 4-thread renders.", 30, "regular", "#FBE3F0FF", W / 2,
              500, center=True)
    section("determinismBeat", 41.2, 44.7, 60, body)


def end_card():
    def body():
        label("endTitle", "endTitleT", "scene-render", 110, "semibold",
              "#FFF6EEFF", W / 2, 300, center=True)
        label("endFile", "endFileT", "this film = examples/dusk-parallax.xml",
              30, "mono", "#FFD48AFF", W / 2, 470, center=True)
        credits = [
            "Art: “Mountain at Dusk” by ansimuz · fog by GrumpyDiamond "
            "— both CC0, via OpenGameArt",
            "Type: Inter (SIL OFL 1.1) · Engine: Apache-2.0",
        ]
        for i, line in enumerate(credits):
            label(f"credit{i}", f"creditT{i}", line, 26, "regular",
                  "#E6D6F0FF", W / 2, 580 + i * 44, center=True)
    section("endCard", 44.9, 48.0 + 0.7, 60, body, rise=16)


# ---- document -------------------------------------------------------------

def build():
    body_start = len(out)

    emit("  <composition>")
    for pid, asset, size, origin, tiles, factor, zoom, z in PLANES:
        extras = []
        if pid == "forest":
            extras = [fireflies("forest", 16, (840, 1000), 3.2, 11),
                      plane_tag("forest", "forest ", "×0.55", 1450, 830,
                                16.3)]
        elif pid == "foreground":
            extras = [fireflies("fg", 8, (900, 1060), 5.0, 29),
                      plane_tag("foreground", "foreground ", "×1.00", 1720,
                                960, 16.6)]
        elif pid == "sky":
            extras = [plane_tag("sky", "sky ", "×0.02", 150, 300, 15.8)]
        elif pid == "farPeaks":
            extras = [plane_tag("farPeaks", "far peaks ", "×0.10", 430, 540,
                                15.9)]
        elif pid == "mountains":
            extras = [plane_tag("mountains", "ridge ", "×0.25", 1060, 680,
                                16.1)]
        plane(pid, asset, size, origin, tiles, factor, zoom, z, extras)
    fog("fogFar", 0.16, 300, 0.30, 15, 4, 1250, 1.3)
    fog("fogNear", 0.4, 480, 0.26, 25, 4, 1350, 1.5)
    bird_flock()

    emit(f'    <shape {attrs(id="dim", shape="rect", width=W, height=H, z=50, fill="#0B0512FF", opacity=0)}>')
    # Composited in linear light: 0.8 here is roughly half the perceived
    # brightness, not a fifth.
    animate("opacity", [(0, 0), (7.2, 0), (7.9, 0.55), (15.2, 0.55),
                        (15.9, 0), (24.9, 0), (25.6, 0.84), (33.7, 0.84),
                        (34.4, 0.7), (40.9, 0.7), (41.5, 0.8),
                        (48, 0.86)])
    emit("    </shape>")

    title_beat()
    xml_beat()
    parallax_beat()
    pipeline_beat()
    features_beat()
    determinism_beat()
    end_card()

    emit(f'    <shape {attrs(id="blackout", shape="rect", width=W, height=H, z=100, fill="#000000FF")}>')
    animate("opacity", [(0, 1), (1.6, 0), (47.1, 0), (48, 1)])
    emit("    </shape>")
    emit("  </composition>")
    body = out[body_start:]
    del out[body_start:]

    emit('<?xml version="1.0" encoding="UTF-8"?>')
    emit("<!-- Generated by scripts/dusk-parallax/build_scene.py; edit that "
         "script, not this file. -->")
    emit('<scene version="1.0">')
    emit(f'  <project {attrs(width=W, height=H, fps=FPS, duration=DURATION, seed=SEED, linearLight=True, workingColorSpace="srgb", background="#000000FF", mode="standard")}/>')
    emit(f'  <output {attrs(path="../build/dusk-parallax.mp4", codec="h264", pixelFormat="yuv420p", preset="medium", crf=18, audioCodec="aac", audioBitrate=160000, colorSpace="srgb", colorRange="limited")}/>')
    emit("  <assets>")
    files = {"sky": "sky.png", "farPeaks": "far-peaks.png",
             "mountains": "mountains.png", "forest": "forest.png",
             "foreground": "foreground.png"}
    for pid, asset, size, *_ in PLANES:
        emit(f'    <image {attrs(id=asset, src=f"{ART}/{files[asset]}", width=size[0], height=size[1], colorSpace="srgb")}/>')
    emit(f'    <image {attrs(id="fog", src=f"{ART}/original/fog.png", width=1475, height=704, colorSpace="srgb")}/>')
    emit(f'    <vector {attrs(id="bird", shape="path", width=64, height=26, path="M 0 12 Q 16 0 32 14 Q 48 0 64 12 Q 48 7 32 22 Q 16 7 0 12 Z", fill="#1E1028EE")}/>')
    for aid, (value, size, weight, color, width) in TEXTS.items():
        if weight == "mono":
            font = {"font": MONO_FONT}
        else:
            font = {"fontFile": f"../assets/third-party/inter/{FONTS[weight]}"}
        emit(f'    <text {attrs(id=aid, text=value, width=width, height=text_height(size), size=size, color=color, **font)}/>')
    emit('    <audio id="hum" src="../assets/generated/audio/hum.wav"/>')
    emit('    <audio id="transmit" src="../assets/generated/audio/transmit.wav"/>')
    emit('    <audio id="shield" src="../assets/generated/audio/shield.wav"/>')
    emit("  </assets>")
    out.extend(body)
    emit("  <effects>")
    emit(f'    <effect {attrs(id="bloom", type="bloom", intensity=0.3, radius=10, threshold=0.78)}/>')
    emit(f'    <effect {attrs(id="grade", type="color-grade", intensity=0.6, saturation=1.06, contrast=1.04, brightness=0.004)}/>')
    emit(f'    <effect {attrs(id="vignette", type="vignette", intensity=0.42, radius=1)}/>')
    emit("  </effects>")
    emit('  <audioMix sampleRate="48000" channels="2">')
    emit(f'    <audioTrack {attrs(id="ambience", asset="hum", start=0, loop=2, volume=0.32, pan=0)}/>')
    emit(f'    <audioTrack {attrs(id="pipelinePing", asset="transmit", start=26.8, volume=0.18, pan=0)}/>')
    emit(f'    <audioTrack {attrs(id="determinismTone", asset="shield", start=41.2, volume=0.16, pan=0)}/>')
    emit("  </audioMix>")
    emit("</scene>")

    with open(OUT, "w", encoding="utf-8") as f:
        f.write("\n".join(out) + "\n")
    print(f"wrote {os.path.relpath(OUT, ROOT)} ({len(TEXTS)} text assets)")


if __name__ == "__main__":
    build()
