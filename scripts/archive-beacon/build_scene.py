#!/usr/bin/env python3
"""Write examples/archive-beacon.xml, the 30 s "Archive Beacon" sequence.

The XML is authored once on a 3840x1920 panorama and rendered twice:

  scene-render --scene examples/archive-beacon.xml                 # UHD viewport
  scene-render --scene examples/archive-beacon.xml --mode equirectangular \
               --output build/archive-beacon-360.mp4               # 360 master

Debris is simulated by the engine. This script replays the engine's
semi-implicit Euler step (src/physics.c) up to each rock's first contact so
impact sparks and sounds can be keyed to the collisions the engine will
compute. It refuses to write a scene whose rocks touch something unplanned.

--render-time-uhd / --render-time-360 fill the end card after a measured
render pass (the built-in font supports A-Z, 0-9 and space only).
"""
import argparse
import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import layout as L  # noqa: E402
from layout import CX, CY  # noqa: E402

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
FIXED_STEP = 1.0 / 240.0
# Title and end card share a horizon spot at yaw -130, clear of the station.
TITLE_X = round(L.x_at_yaw(-130))

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
    return " ".join(f'{k.rstrip("_")}="{fmt(v)}"' for k, v in kw.items()
                    if v is not None)


def animate(prop, keys, interp="ease-in-out", indent="      "):
    """keys: list of (time, value) or (time, value, interpolation)."""
    emit(f'{indent}<animate property="{prop}" defaultInterpolation="{interp}">')
    for key in sorted(keys, key=lambda k: k[0]):
        extra = f' interpolation="{key[2]}"' if len(key) > 2 else ""
        emit(f'{indent}  <key time="{fmt(float(key[0]))}" '
             f'value="{fmt(float(key[1]))}"{extra}/>')
    emit(f"{indent}</animate>")


def clip_keys(keys):
    """Drop keys before t=0, inserting the interpolated value at t=0."""
    keys = sorted(keys, key=lambda k: k[0])
    kept = [k for k in keys if k[0] >= 0]
    before = [k for k in keys if k[0] < 0]
    if before and kept and kept[0][0] > 0:
        a, b = before[-1], kept[0]
        if a[2] == "step":
            value = a[1]
        else:
            value = a[1] + (b[1] - a[1]) * (0 - a[0]) / (b[0] - a[0])
        kept.insert(0, (0.0, value, a[2]))
    return kept


FONT_DIR = os.path.join(ROOT, "assets", "third-party", "inter")
FONTS = {"light": "Inter-Light.ttf", "regular": "Inter-Regular.ttf",
         "medium": "Inter-Medium.ttf", "semibold": "Inter-SemiBold.ttf"}
_metrics = {}


def font_metrics(weight):
    """(advance widths by codepoint, units per em) read with fontTools."""
    if weight not in _metrics:
        try:
            from fontTools.ttLib import TTFont
        except ImportError:
            raise SystemExit("build_scene.py needs fontTools (pip install fonttools)")
        font = TTFont(os.path.join(FONT_DIR, FONTS[weight]))
        cmap, hmtx = font.getBestCmap(), font["hmtx"]
        _metrics[weight] = ({cp: hmtx[name][0] for cp, name in cmap.items()},
                            font["head"].unitsPerEm)
    return _metrics[weight]


def text_width(text, size, weight):
    """Unkerned advance width; drawtext's HarfBuzz kerning only tightens it."""
    advances, upem = font_metrics(weight)
    missing = [c for c in text if ord(c) not in advances]
    if missing:
        raise SystemExit(f"Inter has no glyph for {missing!r} in {text!r}")
    return sum(advances[ord(c)] for c in text) * size / upem


# --------------------------------------------------------------------------
# Debris planning and contact prediction
# --------------------------------------------------------------------------

def rock_start(approach_deg, offset, speed, t_contact, radius):
    """Start state for a rock that reaches the station ring at t_contact.

    approach_deg is where the rock comes *from*, measured around the station
    (0 = right, 90 = down). offset is the lateral miss distance of its line
    from the station centre; |offset| < RING_R + radius hits the ring.
    """
    a = math.radians(approach_deg)
    ux, uy = -math.cos(a), -math.sin(a)          # travel direction
    nx, ny = -uy, ux
    reach = L.RING_R + radius
    along = math.sqrt(max(0.0, reach * reach - offset * offset))
    if abs(offset) >= reach:
        along = 0.0                              # closest approach instead
    cx = CX + nx * offset - ux * along
    cy = CY + ny * offset - uy * along
    return (cx - ux * speed * t_contact, cy - uy * speed * t_contact,
            ux * speed, uy * speed)


def panel_start(panel, dx, approach_deg, speed, t_contact, radius):
    """Start state for a rock that strikes the top face of a panel."""
    a = math.radians(approach_deg)
    ux, uy = -math.cos(a), -math.sin(a)
    cx = panel[0] + dx
    cy = panel[1] - L.PANEL_H / 2 - radius
    return (cx - ux * speed * t_contact, cy - uy * speed * t_contact,
            ux * speed, uy * speed)


ROCKS = [
    # id, radius, box?, fill, start-state builder
    ("rock1", 26, False, "#7A6F66FF", rock_start(300, 30, 430, 15.0, 26)),
    ("rock2", 34, False, "#8C8177FF", rock_start(215, -50, 380, 15.5, 34)),
    ("rock3", 22, False, "#6B625BFF", rock_start(128, 70, 460, 16.1, 22)),
    ("rock4", 30, False, "#9A8F80FF", rock_start(62, -20, 400, 16.6, 30)),
    ("rock5", 24, False, "#857A70FF", panel_start(L.PANEL_L, 30, 250, 420,
                                                  15.8, 24)),
    ("rock6", 20, True, "#5E5650FF", rock_start(272, 110, 350, 17.0, 20)),
    ("rock7", 16, False, "#A09484FF", rock_start(22, 0, 520, 16.9, 16)),
    ("shard1", 14, False, "#8A8078FF", rock_start(335, 420, 480, 15.4, 14)),
    ("shard2", 12, True, "#6F665FFF", rock_start(150, 400, 450, 15.2, 12)),
]


def circle_box(cx, cy, r, bx, by, hw, hh):
    qx = min(max(cx, bx - hw), bx + hw)
    qy = min(max(cy, by - hh), by + hh)
    return math.hypot(cx - qx, cy - qy) < r, (qx, qy)


def predict_contacts():
    """Replay the engine integrator (no forces, no damping) until contact."""
    statics = [("ring", "circle", CX, CY, L.RING_R)]
    for side, (px, py) in (("L", L.PANEL_L), ("R", L.PANEL_R)):
        statics.append((f"panel{side}", "box", px, py,
                        (L.PANEL_W / 2, L.PANEL_H / 2)))
        statics.append((f"anchor{side}", "circle", px, py + L.ANCHOR_DY, 4))
    rocks = [dict(id=i, r=r, x=s[0], y=s[1], vx=s[2], vy=s[3], hit=None)
             for i, r, _, _, s in ROCKS]
    steps = int(24.0 / FIXED_STEP)
    for step in range(1, steps + 1):
        t = step * FIXED_STEP
        for rock in rocks:
            if rock["hit"]:
                continue
            rock["x"] += rock["vx"] * FIXED_STEP
            rock["y"] += rock["vy"] * FIXED_STEP
        for rock in rocks:
            if rock["hit"]:
                continue
            for name, kind, sx, sy, size in statics:
                if kind == "circle":
                    d = math.hypot(rock["x"] - sx, rock["y"] - sy)
                    if d < rock["r"] + size:
                        nx, ny = (rock["x"] - sx) / d, (rock["y"] - sy) / d
                        rock["hit"] = (t, name, sx + nx * size, sy + ny * size,
                                       math.degrees(math.atan2(ny, nx)))
                        break
                else:
                    inside, (qx, qy) = circle_box(rock["x"], rock["y"],
                                                  rock["r"], sx, sy, *size)
                    if inside:
                        ang = math.degrees(math.atan2(rock["y"] - qy,
                                                      rock["x"] - qx))
                        rock["hit"] = (t, name, qx, qy, ang)
                        break
        live = [r for r in rocks if not r["hit"]]
        for i, a in enumerate(live):
            for b in live[i + 1:]:
                if math.hypot(a["x"] - b["x"], a["y"] - b["y"]) < a["r"] + b["r"]:
                    raise SystemExit(f"{a['id']} meets {b['id']} at t={t:.2f}s")
    return {r["id"]: r["hit"] for r in rocks}


# --------------------------------------------------------------------------
# Scene
# --------------------------------------------------------------------------

def build(render_uhd, render_360):
    contacts = predict_contacts()
    expected = {"rock1": "ring", "rock2": "ring", "rock3": "ring",
                "rock4": "ring", "rock5": "panelL", "rock6": "ring",
                "rock7": "ring", "shard1": None, "shard2": None}
    for rid, want in expected.items():
        got = contacts[rid][1] if contacts[rid] else None
        if got != want:
            raise SystemExit(f"{rid}: planned contact {want}, predicted {got}")
        if contacts[rid] and contacts[rid][0] < 13.0:
            raise SystemExit(f"{rid} collides before it becomes visible")

    # id -> (text, font size px, Inter weight, colour)
    texts = {
        "titleText": ("THE ARCHIVE BEACON", 60, "light", "#EAF6FFFF"),
        "subtitleText": ("Transmission log 01", 26, "regular", "#7FC8F8FF"),
        "onlineText": ("ARCHIVE STATION ONLINE", 20, "medium", "#7FF3FFFF"),
        "warnText": ("IMPACT WARNING", 34, "semibold", "#FF5A5AFF"),
        "sigA": ("SIGNAL A", 18, "medium", "#7FE3FFFF"),
        "sigB": ("SIGNAL B", 18, "medium", "#FFC46BFF"),
        "sigC": ("SIGNAL C", 18, "medium", "#FF8FE0FF"),
        "card0": ("Scene generated from XML", 50, "semibold", "#EAF6FFFF"),
        "card1": ("UHD 3840 × 2160  ·  360° 3840 × 1920", 28, "regular",
                  "#9FD8FFFF"),
        "card2": (f"30 fps  ·  900 frames  ·  seed {L.SEED}", 28, "regular",
                  "#9FD8FFFF"),
        "card3": (f"Render time  ·  UHD {render_uhd}  ·  360° {render_360}", 28,
                  "medium", "#FFD37AFF"),
        "card4": ("scene-render 1.1.0  ·  CPU  ·  deterministic", 20,
                  "regular", "#6E8FB0FF"),
    }

    def width_of(aid):
        text, size, weight, _ = texts[aid]
        return text_width(text, size, weight)

    emit('<?xml version="1.0"?>')
    emit("<!-- Generated by scripts/archive-beacon/build_scene.py; edit the")
    emit("     generator, not this file. One panorama-space scene renders both")
    emit("     the UHD viewport cut and the 3840x1920 equirectangular master. -->")
    emit('<scene version="1.0">')
    emit(f'  <project {attrs(width=3840, height=2160, fps=L.FPS, duration=L.DURATION, seed=L.SEED, linearLight=True, background="#02040AFF", mode="viewport")}/>')
    emit(f'  <output {attrs(path="../build/archive-beacon-uhd.mp4", codec="h264", pixelFormat="yuv420p", preset="medium", crf=18, audioCodec="aac", audioBitrate=192000)}/>')

    # ---------------------------------------------------------------- assets
    emit("  <assets>")
    g = "../assets/generated"
    emit(f'    <image {attrs(id="stars", src=f"{g}/starfield/starfield.png", width=L.PANO_W, height=L.PANO_H)}/>')
    emit(f'    <video {attrs(id="telemetry", src=f"{g}/telemetry/telemetry.mp4", width=640, height=360, fps=30, duration=10)}/>')
    for name in ("hum", "burst", "impact", "alarm", "shield", "transmit"):
        emit(f'    <audio {attrs(id=f"sfx_{name}", src=f"{g}/audio/{name}.wav")}/>')
    for aid, (text, size, weight, color) in texts.items():
        font = f"../assets/third-party/inter/{FONTS[weight]}"
        emit(f'    <text {attrs(id=aid, text=text, width=math.ceil(width_of(aid)) + 8, height=math.ceil(size * 1.3), size=size, color=color, fontFile=font)}/>')
    emit("  </assets>")

    # ------------------------------------------------------------- materials
    emit("  <materials>")
    emit(f'    <material {attrs(id="hullMat", baseColor="#2F4D72FF", metallic=0.45, roughness=0.5, emissive="#03060CFF")}/>')
    emit(f'    <material {attrs(id="wornMat", baseColor="#7A838EFF", metallic=0.3, roughness=0.7, emissive="#050505FF")}/>')
    emit(f'    <material {attrs(id="dishMat", baseColor="#A8B4C2FF", metallic=0.3, roughness=0.4, emissive="#0A1420FF")}/>')
    emit(f'    <material {attrs(id="beaconMat", baseColor="#FF6A5AFF", metallic=0.0, roughness=0.2, emissive="#C03020FF")}/>')
    emit(f'    <material {attrs(id="planetMat", baseColor="#2E6FA6FF", metallic=0.0, roughness=0.85, emissive="#030C18FF")}/>')
    emit(f'    <material {attrs(id="moonMat", baseColor="#9A9690FF", metallic=0.0, roughness=0.9, emissive="#040404FF")}/>')
    emit("  </materials>")
    emit(f'  <scene360 {attrs(layout="equirectangular", width=L.PANO_W, height=L.PANO_H, viewportCamera="tour")}/>')

    emit("  <composition>")
    # ------------------------------------------------------------ camera
    # Inactive so 3D primitives stay in panorama pixel space; scene360 still
    # selects it for viewport extraction by id.
    cam = [  # time, yaw, pitch, fov, roll
        (0.0, -130, 0, 58, 0), (2.6, -126, 0.5, 58, 0),
        (5.2, -45, 14, 88, -4), (8.0, 5, -5, 86, 0),
        (12.8, 8, -5, 82, 0), (15.5, 2, 0, 80, 0), (17.8, 0, 0, 84, 0),
        (22.6, -4, 3, 80, 0), (23.4, -5, 6, 72, 0),
        (24.2, -24, 24, 66, -6), (25.2, -36, 33, 62, -3),
        (26.7, -38, 34, 62, 0), (27.6, -130, 0, 60, 0),
        (30.0, -130, 0, 58, 0)]
    emit(f'    <camera {attrs(id="tour", active=False, projection="perspective", yaw=cam[0][1], pitch=cam[0][2], roll=0, fov=cam[0][3])}>')
    for idx, prop in ((1, "yaw"), (2, "pitch"), (3, "fov"), (4, "roll")):
        animate(prop, [(k[0], k[idx]) for k in cam])
    emit("    </camera>")

    # ------------------------------------------------------------ starfield
    emit(f'    <layer {attrs(id="starfield", asset="stars", z=-100, opacity=0)}>')
    animate("opacity", [(0.2, 0), (2.4, 1)])
    emit("    </layer>")
    dust = [(600, 500), (3300, 700), (2900, 1400), (300, 1300), (2000, 350)]
    for i, (x, y) in enumerate(dust):
        emit(f'    <particleEmitter {attrs(id=f"stardust{i}", z=-90, preset="dust", x=x, y=y, rate=5, lifetime=24, speed=110, spread=360, size=2, color="#CFE0FFAA", blend="screen")}/>')
    # Near dust drifts against the camera pan for a parallax cue.
    emit(f'    <group {attrs(id="nearDust", z=-80)}>')
    animate("position.x", [(3.0, 0), (8.0, -260), (30.0, -420)])
    for i, (x, y) in enumerate([(1100, 650), (2500, 1250), (1700, 1350)]):
        emit(f'      <particleEmitter {attrs(id=f"neardust{i}", preset="dust", x=x, y=y, rate=4, lifetime=30, speed=160, spread=360, size=3, color="#E8F0FFCC", blend="add")}/>')
    emit("    </group>")

    # ------------------------------------------------------------ planet
    px, py = L.PLANET_X, L.PLANET_Y
    emit(f'    <object3D {attrs(id="planet", primitive="sphere", material="planetMat", x=px, y=py, z=-200, radius=L.PLANET_R, scaleX=L.PLANET_SX, castShadow=False)}/>')
    emit(f'    <object3D {attrs(id="moon", primitive="sphere", material="moonMat", x=2980, y=1300, z=-300, radius=58, scaleX=1.2, castShadow=False)}/>')
    # Cloud bands cross the disc and foreshorten at the limbs so the planet
    # reads as rotating.
    clouds = [(-190, 110, 16, 0.0), (-120, 170, 22, 3.1), (-50, 140, 18, 7.4),
              (20, 190, 24, 1.7), (90, 150, 20, 5.2), (160, 120, 16, 9.8),
              (-150, 90, 14, 11.3), (60, 100, 16, 12.6)]
    period = 14.0
    for i, (dy, w, h, phase) in enumerate(clouds):
        half = L.PLANET_RX * math.sqrt(max(0.0, 1 - (dy / L.PLANET_RY) ** 2)) - w * 0.3
        emit(f'    <shape {attrs(id=f"cloud{i}", shape="ellipse", width=w, height=h, x=px - half, y=py + dy, anchorX=w / 2, anchorY=h / 2, fill="#DDEBFFFF", blend="screen", opacity=0)}>')
        xs, ops, sxs = [], [], []
        t0 = -phase
        while t0 < L.DURATION:
            t1 = t0 + period
            for frac, xf, op, sx in ((0, -1, 0, .25), (.15, -.62, .28, .75),
                                     (.5, 0, .32, 1), (.85, .62, .28, .75),
                                     (.999, 1, 0, .25)):
                tt = t0 + frac * period
                if tt <= L.DURATION + period:
                    last = frac == .999
                    xs.append((tt, px + xf * half, "step" if last else "linear"))
                    ops.append((tt, op, "linear"))
                    sxs.append((tt, sx, "linear"))
            t0 = t1
        animate("position.x", clip_keys(xs), "linear", "      ")
        animate("opacity", clip_keys(ops), "linear", "      ")
        animate("scale.x", clip_keys(sxs), "linear", "      ")
        emit("    </shape>")
    emit(f'    <shape {attrs(id="atmosphere", shape="ellipse", width=L.PLANET_RX * 2 + 24, height=L.PLANET_RY * 2 + 24, x=px, y=py, anchorX=L.PLANET_RX + 12, anchorY=L.PLANET_RY + 12, fill="#00000000", stroke="#5FB8FF99", strokeWidth=10, blend="screen")}/>')

    # ------------------------------------------------------------ station 3D
    parts = [
        ("module", "box", "hullMat", CX, CY, 60, 2.4, 0.45),
        ("trussL", "box", "wornMat", CX - 237, CY, 50, 1.86, 0.12),
        ("trussR", "box", "wornMat", CX + 237, CY, 50, 1.86, 0.12),
        ("antenna", "box", "wornMat", CX, CY - 115, 40, 0.1, 1.0),
        ("podL", "sphere", "wornMat", CX - 130, CY, 34, 1, 1),
        ("podR", "sphere", "wornMat", CX + 130, CY, 34, 1, 1),
        ("hub", "sphere", "hullMat", CX, CY, 78, 1, 1),
        ("dish", "sphere", "dishMat", CX, CY + 85, 26, 1.3, 0.8),
        ("beaconLamp", "sphere", "beaconMat", CX, CY - 160, 12, 1, 1),
    ]
    for pid, prim, mat, x, y, r, sx, sy in parts:
        # v1.1 boxes have real depth; keep them no deeper than they are tall
        # so the module does not bury the pods.
        sz = min(1.0, sy) if prim == "box" else 1.0
        emit(f'    <object3D {attrs(id=pid, primitive=prim, material=mat, x=x, y=y, z=0, radius=r, scaleX=sx, scaleY=sy, scaleZ=sz, castShadow=True)}/>')

    # ------------------------------------------------------------ title
    tw = width_of("titleText")
    emit(f'    <group {attrs(id="title", x=TITLE_X, y=L.CY, anchorX=tw / 2, anchorY=39, opacity=0)}>')
    animate("opacity", [(0.6, 0), (1.8, 1), (3.6, 1), (4.6, 0)])
    animate("scale.x", [(0.6, 0.94), (2.4, 1.0)], "ease-out")
    animate("scale.y", [(0.6, 0.94), (2.4, 1.0)], "ease-out")
    emit(f'      <layer {attrs(id="titleLayer", asset="titleText", x=0, y=0)}/>')
    sw = width_of("subtitleText")
    emit(f'      <layer {attrs(id="subtitleLayer", asset="subtitleText", x=(tw - sw) / 2, y=100, opacity=0)}>')
    animate("opacity", [(1.4, 0), (2.4, 1)], indent="        ")
    emit("      </layer>")
    emit(f'      <shape {attrs(id="titleRule", shape="rect", width=tw, height=3, x=0, y=86, fill="#4FC3F7CC", blend="add")}/>')
    emit("    </group>")

    # ------------------------------------------------------------ station 2D
    rd = L.RING_R * 2
    emit(f'    <shape {attrs(id="ring", shape="ellipse", width=rd, height=rd, x=CX, y=CY, anchorX=L.RING_R, anchorY=L.RING_R, fill="#00000000", stroke="#6FA8DCFF", strokeWidth=6)}>')
    emit(f'      <rigidBody {attrs(type="static", shape="circle", radius=L.RING_R, mass=1, friction=0.4, restitution=0.55)}/>')
    emit("    </shape>")
    emit(f'    <shape {attrs(id="ringGlow", shape="ellipse", width=rd + 8, height=rd + 8, x=CX, y=CY, anchorX=L.RING_R + 4, anchorY=L.RING_R + 4, fill="#00000000", stroke="#7FF3FFFF", strokeWidth=12, blend="add", opacity=0)}>')
    animate("opacity", [(7.9, 0), (8.3, 1), (9.2, 0.45), (13.0, 0.45),
                        (13.4, 1), (14.4, 0.45), (22.8, 0.45), (23.2, 1),
                        (26.8, 1), (27.4, 0.3)])
    emit("    </shape>")
    emit(f'    <shape {attrs(id="beaconHalo", shape="ellipse", width=60, height=60, x=CX, y=CY - 160, anchorX=30, anchorY=30, fill="#FF6A5AFF", blend="add", opacity=0)}>')
    blink = []
    t = 8.0
    while t < 30:
        blink += [(t, 0), (t + 0.08, 0.9), (t + 0.5, 0)]
        t += 1.2
    animate("opacity", blink, "linear")
    emit("    </shape>")
    ow = width_of("onlineText")
    emit(f'    <layer {attrs(id="onlineLabel", asset="onlineText", x=CX - ow / 2, y=CY + L.RING_R + 26, opacity=0)}>')
    animate("opacity", [(8.4, 0), (9.0, 1), (12.6, 1), (13.2, 0)])
    emit("    </layer>")
    emit(f'    <shape {attrs(id="reticle", shape="rect", width=120, height=120, x=CX, y=CY, anchorX=60, anchorY=60, fill="#00000000", stroke="#7FF3FFCC", strokeWidth=3, blend="add", rotation=45, opacity=0)}>')
    animate("opacity", [(8.0, 0), (8.4, 0.9), (12.6, 0.9), (13.1, 0)])
    animate("rotation", [(8.0, 45), (13.0, 225)], "linear")
    emit("    </shape>")

    # Solar panels on springs: dynamic boxes, flexed during the shield hit.
    for side, (x, y) in (("L", L.PANEL_L), ("R", L.PANEL_R)):
        emit(f'    <shape {attrs(id=f"anchor{side}", shape="ellipse", width=8, height=8, x=x, y=y + L.ANCHOR_DY, anchorX=4, anchorY=4, fill="#00000000", opacity=0)}>')
        emit(f'      <rigidBody {attrs(type="static", shape="circle", radius=4, mass=1)}/>')
        emit("    </shape>")
        emit(f'    <shape {attrs(id=f"panel{side}", shape="rect", width=L.PANEL_W, height=L.PANEL_H, x=x, y=y, anchorX=L.PANEL_W / 2, anchorY=L.PANEL_H / 2, fill="#1B3B6EFF", stroke="#9CC3F0FF", strokeWidth=4)}>')
        emit(f'      <rigidBody {attrs(type="dynamic", shape="box", mass=3, friction=0.4, restitution=0.2, linearDamping=1.0, angularDamping=1)}/>')
        emit("      <deform>")
        sign = 1 if side == "L" else -1
        emit(f'        <modifier {attrs(type="bend", amount=0, axis="y")}>')
        animate("amount", [(19.2, 0), (19.4, 14 * sign), (19.75, -10 * sign),
                           (20.15, 7 * sign), (20.6, -4 * sign), (21.2, 2 * sign),
                           (22.0, 0)], indent="          ")
        emit("        </modifier>")
        emit("      </deform>")
        emit("    </shape>")
        emit(f'    <shape {attrs(id=f"panelSheen{side}", shape="rect", width=L.PANEL_W - 40, height=6, x=x, y=y - 20, anchorX=(L.PANEL_W - 40) / 2, anchorY=3, fill="#9CC3F055", blend="add")}/>')

    # ------------------------------------------------------------ telemetry
    displays = [
        # id, centre, scale, colour, label, blend, extra layer attrs
        ("A", (2390, 730), 0.5, "#3FD0FF", "sigA", "screen",
         dict(speed=1.0), (20, 20, 600, 320), None, 8.0),
        ("B", (2404, 1141), 0.45, "#FFB347", "sigB", "add",
         dict(speed=2.5, reverse=True, loop=3), None, None, 8.35),
        ("C", (1584, 691), 0.45, "#FF5FD2", "sigC", "normal",
         dict(opacity=0.92), (0, 60, 640, 240),
         [(8.7, 0.0), (10.0, 6.0), (11.0, 2.0), (13.0, 9.5), (17.6, 9.9)], 8.7),
    ]
    for did, (cx, cy), sc, color, label, blend, extra, mask, remap, t_in in displays:
        w, h = 640 * sc, 360 * sc
        emit(f'    <group {attrs(id=f"display{did}", start=7.9, end=17.6, x=cx, y=cy, anchorX=w / 2, anchorY=h / 2)}>')
        animate("scale.y", [(t_in, 0.02), (t_in + 0.35, 1.0)], "ease-out")
        animate("opacity", [(t_in, 0), (t_in + 0.2, 1), (13.25, 1), (13.35, 0.35),
                            (13.5, 1), (16.9, 1), (17.5, 0)], "linear")
        emit(f'      <shape {attrs(id=f"displayBg{did}", shape="rect", width=w + 16, height=h + 16, x=-8, y=-8, fill="#04121FB0")}/>')
        emit(f'      <layer {attrs(id=f"feed{did}", asset="telemetry", start=7.9, x=0, y=0, scaleX=sc, scaleY=sc, blend=blend, **extra)}>')
        if mask:
            mx, my, mw, mh = mask
            emit(f'        <mask {attrs(type="rect", x=mx, y=my, width=mw, height=mh)}/>')
        if remap:
            animate("source.time", remap, "linear", "        ")
        emit("      </layer>")
        emit(f'      <shape {attrs(id=f"tint{did}", shape="rect", width=w, height=h, x=0, y=0, fill=color + "FF", blend="multiply")}/>')
        emit(f'      <shape {attrs(id=f"frame{did}", shape="rect", width=w + 16, height=h + 16, x=-8, y=-8, fill="#00000000", stroke=color + "FF", strokeWidth=3)}/>')
        emit(f'      <layer {attrs(id=f"label{did}", asset=label, x=0, y=-34)}/>')
        emit("    </group>")

    # ------------------------------------------------------------ light burst
    emit(f'    <group {attrs(id="burstBand", start=12.9, end=14.5, x=-700, y=0)}>')
    animate("position.x", [(12.9, -700), (14.4, 4500)], "ease-in-out")
    for i, (w, color) in enumerate([(600, "#3FA8FF14"), (140, "#CFEFFF24")]):
        emit(f'      <shape {attrs(id=f"burst{i}", shape="ellipse", width=w, height=1900, x=0, y=960, anchorX=w / 2, anchorY=950, fill=color, blend="add")}/>')
    emit("    </group>")
    emit(f'    <particleEmitter {attrs(id="burstSparks", preset="sparks", start=13.4, end=14.6, x=CX, y=CY, rate=260, lifetime=1.0, speed=520, spread=360, size=4, color="#DFF3FFEE", blend="add")}>')
    animate("opacity", [(13.4, 1), (14.6, 0)], "linear")
    emit("    </particleEmitter>")

    warn_w = width_of("warnText")
    emit(f'    <layer {attrs(id="warning", asset="warnText", x=CX - warn_w / 2, y=CY - L.RING_R - 60, start=16.3, end=18.6)}>')
    warn = []
    t = 16.3
    while t < 18.6:
        warn += [(t, 1, "step"), (t + 0.2, 0, "step")]
        t += 0.4
    animate("opacity", warn, "step")
    emit("    </layer>")

    # ------------------------------------------------------------ debris
    for rid, r, is_box, fill, (x0, y0, vx, vy) in ROCKS:
        shape = "rect" if is_box else "ellipse"
        emit(f'    <shape {attrs(id=rid, shape=shape, width=2 * r, height=2 * r, x=x0, y=y0, anchorX=r, anchorY=r, fill=fill, stroke="#3B3530FF", strokeWidth=3, start=13.0, end=24.0)}>')
        body = dict(type="dynamic", shape="box" if is_box else "circle",
                    mass=round(r / 40, 3), friction=0.3, restitution=0.6,
                    velocityX=round(vx, 3), velocityY=round(vy, 3),
                    angularVelocity=90 if is_box else 25)
        if not is_box:
            body["radius"] = r
        emit(f'      <rigidBody {attrs(**body)}/>')
        animate("opacity", [(13.0, 0), (13.5, 1), (22.5, 1), (23.5, 0)])
        emit("    </shape>")

    impacts = []
    for rid, hit in contacts.items():
        if not hit:
            continue
        t, target, hx, hy, normal = hit
        impacts.append((t, hx, hy))
        emit(f'    <particleEmitter {attrs(id=f"sparks_{rid}", preset="sparks", start=round(t, 4), end=round(t + 0.55, 4), x=hx, y=hy, rotation=normal + 90, rate=240, lifetime=0.5, speed=380, spread=80, size=4, color="#FFD28AFF", blend="add")}>')
        animate("opacity", [(t, 1), (t + 0.55, 0)], "linear")
        emit("    </particleEmitter>")
        emit(f'    <shape {attrs(id=f"flash_{rid}", shape="ellipse", width=90, height=90, x=hx, y=hy, anchorX=45, anchorY=45, fill="#FFE2A8CC", blend="add", start=round(t, 4), end=round(t + 0.5, 4))}>')
        animate("opacity", [(t, 1), (t + 0.5, 0)], "ease-out")
        animate("scale.x", [(t, 0.3), (t + 0.5, 1.4)], "ease-out")
        animate("scale.y", [(t, 0.3), (t + 0.5, 1.4)], "ease-out")
        emit("    </shape>")

    # ------------------------------------------------------------ shield
    sw_, sh_ = 1360, 700
    emit(f'    <shape {attrs(id="shield", shape="ellipse", width=sw_, height=sh_, x=CX, y=CY, anchorX=sw_ / 2, anchorY=sh_ / 2, fill="#5FD4FF1F", stroke="#9FEAFFCC", strokeWidth=8, blend="screen", start=17.9, end=27.4, opacity=0)}>')
    animate("opacity", [(17.9, 0), (18.5, 1), (22.6, 1), (23.4, 0.3), (26.8, 0.3), (27.3, 0)])
    animate("scale.x", [(17.9, 0.85), (18.5, 1.0)], "ease-out")
    animate("scale.y", [(17.9, 0.85), (18.5, 1.0)], "ease-out")
    emit(f'      <softBody {attrs(mass=1, stiffness=30, damping=0.05, pressure=3)}/>')
    emit("      <deform>")
    emit(f'        <modifier {attrs(type="wave", amount=0, frequency=2, phase=0, axis="x")}>')
    animate("amount", [(18.0, 0), (18.6, 5), (19.2, 5), (19.3, 30), (19.55, -22),
                       (19.85, 15), (20.2, -9), (20.6, 6), (21.2, -3), (21.8, 5),
                       (23.0, 4)], indent="          ")
    animate("phase", [(18.0, 0), (23.0, 14)], "linear", "          ")
    emit("        </modifier>")
    emit(f'        <modifier {attrs(type="bend", amount=0, axis="x")}>')
    animate("amount", [(19.2, 0), (19.32, -55), (19.6, 36), (19.95, -22),
                       (20.35, 11), (20.8, -5), (21.5, 0)], indent="          ")
    emit("        </modifier>")
    emit("      </deform>")
    emit("    </shape>")
    emit(f'    <shape {attrs(id="shieldInner", shape="ellipse", width=sw_ - 60, height=sh_ - 40, x=CX, y=CY, anchorX=(sw_ - 60) / 2, anchorY=(sh_ - 40) / 2, fill="#00000000", stroke="#9FEAFF66", strokeWidth=3, blend="add", start=17.9, end=23.5, opacity=0)}>')
    animate("opacity", [(18.2, 0), (18.7, 1), (22.6, 1), (23.4, 0)])
    emit("    </shape>")

    # Shield impactor: keyframed (non-physics) because the shield is not a
    # collider before 18 s; it arrives at the shell at 19.25 s and rebounds.
    hit_t = 19.25
    th = math.radians(125)
    hx, hy = CX + sw_ / 2 * math.cos(th), CY - sh_ / 2 * math.sin(th)
    emit(f'    <shape {attrs(id="impactor", shape="ellipse", width=56, height=56, x=1150, y=330, anchorX=28, anchorY=28, fill="#8A7F74FF", stroke="#3B3530FF", strokeWidth=3, start=18.4, end=20.6)}>')
    animate("position.x", [(18.4, 1150, "linear"), (hit_t, hx - 22, "ease-out"), (20.5, 1260)])
    animate("position.y", [(18.4, 330, "linear"), (hit_t, hy - 22, "ease-out"), (20.5, 230)])
    animate("opacity", [(18.4, 0), (18.6, 1), (20.0, 1), (20.5, 0)])
    emit("    </shape>")
    emit(f'    <shape {attrs(id="shieldFlash", shape="ellipse", width=220, height=220, x=hx, y=hy, anchorX=110, anchorY=110, fill="#BFF4FFFF", blend="add", start=hit_t, end=hit_t + 0.7)}>')
    animate("opacity", [(hit_t, 1), (hit_t + 0.7, 0)], "ease-out")
    animate("scale.x", [(hit_t, 0.2), (hit_t + 0.7, 1.6)], "ease-out")
    animate("scale.y", [(hit_t, 0.2), (hit_t + 0.7, 1.6)], "ease-out")
    emit("    </shape>")
    emit(f'    <shape {attrs(id="shieldRipple", shape="ellipse", width=300, height=300, x=hx, y=hy, anchorX=150, anchorY=150, fill="#00000000", stroke="#9FEAFFFF", strokeWidth=6, blend="add", start=hit_t, end=hit_t + 1.2)}>')
    animate("opacity", [(hit_t, 1), (hit_t + 1.2, 0)], "linear")
    animate("scale.x", [(hit_t, 0.2), (hit_t + 1.2, 3.0)], "ease-out")
    animate("scale.y", [(hit_t, 0.2), (hit_t + 1.2, 3.0)], "ease-out")
    emit("    </shape>")
    emit(f'    <particleEmitter {attrs(id="shieldSparks", preset="sparks", start=hit_t, end=hit_t + 0.6, x=hx, y=hy, rotation=math.degrees(math.atan2(hy - CY, hx - CX)) + 90, rate=300, lifetime=0.55, speed=420, spread=90, size=4, color="#BFF4FFFF", blend="add")}>')
    animate("opacity", [(hit_t, 1), (hit_t + 0.6, 0)], "linear")
    emit("    </particleEmitter>")

    # ------------------------------------------------------------ beam
    bx0, by0 = CX, CY + 85
    tx, ty = 1500, 1290
    blen = math.hypot(tx - bx0, ty - by0)
    bang = math.degrees(math.atan2(ty - by0, tx - bx0))
    emit(f'    <shape {attrs(id="dishCharge", shape="ellipse", width=54, height=54, x=bx0, y=by0, anchorX=27, anchorY=27, fill="#9FF4FFFF", blend="add", start=22.7, end=27.3, opacity=0)}>')
    animate("opacity", [(22.7, 0), (23.2, 0.75), (26.7, 0.75), (27.2, 0)])
    animate("scale.x", [(22.7, 0.2), (23.2, 1.3), (23.5, 1.0)], "ease-out")
    animate("scale.y", [(22.7, 0.2), (23.2, 1.3), (23.5, 1.0)], "ease-out")
    emit("    </shape>")
    emit(f'    <group {attrs(id="beam", start=23.1, end=27.3, x=bx0, y=by0, rotation=bang)}>')
    animate("opacity", [(23.1, 0), (23.25, 1), (26.7, 1), (27.2, 0)], "linear")
    for i, (hgt, color) in enumerate([(70, "#3FC8FF30"), (26, "#7FE3FFB0"),
                                      (7, "#FFFFFFFF")]):
        emit(f'      <shape {attrs(id=f"beam{i}", shape="rect", width=blen, height=hgt, x=0, y=0, anchorX=0, anchorY=hgt / 2, fill=color, blend="add", scaleX=0)}>')
        animate("scale.x", [(23.2, 0), (23.7, 1)], "ease-in", "        ")
        emit("      </shape>")
    for i in range(6):
        keys, t = [], 23.7 + i * 0.1
        while t < 27.2:
            keys += [(t, 0, "linear"), (t + 0.599, blen, "step")]
            t += 0.6
        emit(f'      <shape {attrs(id=f"pulse{i}", shape="ellipse", width=24, height=10, x=0, y=0, anchorX=12, anchorY=5, fill="#FFFFFFFF", blend="add", start=23.7)}>')
        animate("position.x", keys, "linear", "        ")
        emit("      </shape>")
    emit(f'      <particleEmitter {attrs(id="beamRain", preset="rain", start=23.4, x=0, y=0, rotation=-90, rate=160, lifetime=blen / 800, speed=800, size=3, color="#BFF4FFCC", blend="add")}/>')
    emit("    </group>")
    emit(f'    <shape {attrs(id="targetGlow", shape="ellipse", width=120, height=80, x=tx, y=ty, anchorX=60, anchorY=40, fill="#8FF0FFFF", blend="add", start=23.6, end=27.4, opacity=0)}>')
    animate("opacity", [(23.6, 0), (23.8, 0.55), (26.7, 0.55), (27.3, 0)])
    emit("    </shape>")
    for i, t in enumerate((23.8, 24.8, 25.8)):
        emit(f'    <shape {attrs(id=f"targetRing{i}", shape="ellipse", width=240, height=160, x=tx, y=ty, anchorX=120, anchorY=80, fill="#00000000", stroke="#9FF4FFFF", strokeWidth=5, blend="add", start=t, end=t + 1.2)}>')
        animate("opacity", [(t, 1), (t + 1.2, 0)], "linear")
        animate("scale.x", [(t, 0.2), (t + 1.2, 2.4)], "ease-out")
        animate("scale.y", [(t, 0.2), (t + 1.2, 2.4)], "ease-out")
        emit("    </shape>")
    emit(f'    <particleEmitter {attrs(id="targetSparks", preset="sparks", start=23.7, end=27.2, x=tx, y=ty, rotation=math.degrees(math.atan2(by0 - ty, bx0 - tx)) + 90, rate=120, lifetime=0.8, speed=260, spread=140, size=3, color="#BFF4FFCC", blend="add")}/>')

    # ------------------------------------------------------------ end card
    ccx, ccy = TITLE_X, L.CY
    emit(f'    <group {attrs(id="endCard", start=27.0, z=50)}>')
    emit(f'      <shape {attrs(id="cardPanel", shape="rect", width=880, height=420, x=ccx, y=ccy, anchorX=440, anchorY=210, fill="#050C18E6", stroke="#4FC3F7FF", strokeWidth=3, opacity=0)}>')
    animate("opacity", [(27.2, 0), (27.7, 1)], indent="        ")
    emit("      </shape>")
    ys = [ccy - 185, ccy - 78, ccy - 28, ccy + 30, ccy + 125]
    for i, y in enumerate(ys):
        w = width_of(f"card{i}")
        t = 27.6 + 0.2 * i
        emit(f'      <layer {attrs(id=f"cardLine{i}", asset=f"card{i}", x=ccx - w / 2, y=y, opacity=0)}>')
        animate("opacity", [(t, 0), (t + 0.4, 1)], indent="        ")
        emit("      </layer>")
    emit(f'      <shape {attrs(id="cardRule", shape="rect", width=760, height=4, x=ccx, y=ccy - 104, anchorX=380, anchorY=2, fill="#4FC3F7FF", blend="add", scaleX=0)}>')
    animate("scale.x", [(27.7, 0), (28.2, 1)], "ease-out", "        ")
    emit("      </shape>")
    emit("    </group>")
    emit("  </composition>")

    # ------------------------------------------------------------ lights
    emit("  <lights>")
    emit(f'    <light {attrs(id="ambient", type="ambient", color="#8FA8D8FF", intensity=0.08)}/>')
    emit(f'    <light {attrs(id="sun", type="directional", color="#FFF2DDFF", intensity=0.85, yaw=-44, pitch=30, castShadow=True)}/>')
    emit(f'    <light {attrs(id="rim", type="spot", color="#7FD3FFFF", intensity=0.45, x=2600, y=400, z=700, yaw=-44.6, pitch=-30, range=2200, falloff=1.0, spotAngle=40)}/>')
    emit(f'    <light {attrs(id="burstLight", type="point", color="#CFE8FFFF", intensity=0, x=-700, y=960, z=400, range=1500, falloff=1.2)}>')
    animate("intensity", [(12.9, 0), (13.6, 3), (14.4, 0)])
    animate("position.x", [(12.9, -700), (14.4, 4500)])
    emit("    </light>")
    emit(f'    <light {attrs(id="alarm", type="point", color="#FF3B30FF", intensity=0, x=CX, y=CY, z=250, range=900, falloff=1.0)}>')
    alarm = [(0.0, 0), (16.29, 0)]  # the first key's value holds before it
    t = 16.3
    while t < 18.6:
        alarm += [(t, 2.2), (t + 0.2, 0)]
        t += 0.4
    animate("intensity", alarm, "linear")
    emit("    </light>")
    emit(f'    <light {attrs(id="beaconLight", type="point", color="#6FF0FFFF", intensity=0, x=bx0, y=by0, z=150, range=1100, falloff=1.1)}>')
    animate("intensity", [(7.9, 0), (8.3, 0.9), (9.2, 0.15), (22.7, 0.15),
                          (23.3, 1.6), (26.7, 1.6), (27.3, 0.1)])
    emit("    </light>")
    emit("  </lights>")

    # ------------------------------------------------------------ effects
    emit("  <effects>")
    emit(f'    <effect {attrs(id="titleGlow", type="glow", intensity=0, radius=8, threshold=0.5)}>')
    animate("intensity", [(0.5, 0), (1.8, 0.9), (3.5, 0.5), (5.0, 0.15)])
    emit("    </effect>")
    emit(f'    <effect {attrs(id="bloom", type="bloom", intensity=0.55, radius=12, threshold=0.62)}>')
    animate("intensity", [(12.9, 0.55), (13.6, 0.85), (14.5, 0.55), (23.0, 0.55),
                          (23.5, 0.95), (26.7, 0.95), (27.3, 0.55)])
    emit("    </effect>")
    emit(f'    <effect {attrs(id="motionBlur", type="blur", intensity=0, radius=10)}>')
    animate("intensity", [(23.55, 0), (23.85, 0.6), (24.25, 0.45), (24.6, 0),
                          (26.75, 0), (27.05, 0.75), (27.35, 0.6), (27.7, 0)])
    emit("    </effect>")
    emit(f'    <effect {attrs(id="flare", type="lens-flare", intensity=0, radius=90, color="#CFE8FFFF")}>')
    animate("intensity", [(13.1, 0), (13.6, 1.4), (14.2, 0)])
    emit("    </effect>")
    emit(f'    <effect {attrs(id="grade", type="color-grade", intensity=0.8, saturation=1.12, contrast=1.08, brightness=0.0)}/>')
    emit(f'    <effect {attrs(id="vignette", type="vignette", intensity=0.6, radius=1)}/>')
    emit("  </effects>")

    # ------------------------------------------------------------ physics
    emit(f'  <physics {attrs(fixedStep=FIXED_STEP, gravityX=0, gravityY=0)}>')
    for side, (x, y) in (("L", L.PANEL_L), ("R", L.PANEL_R)):
        emit(f'    <constraint {attrs(id=f"hubSpring{side}", type="spring", a=f"panel{side}", b="ring", restLength=abs(x - CX), stiffness=90, damping=9)}/>')
        emit(f'    <constraint {attrs(id=f"mastSpring{side}", type="spring", a=f"panel{side}", b=f"anchor{side}", restLength=abs(L.ANCHOR_DY), stiffness=90, damping=9)}/>')
    emit("  </physics>")

    # ------------------------------------------------------------ audio
    emit(f'  <audioMix {attrs(sampleRate=48000, channels=2)}>')
    emit(f'    <audioTrack {attrs(id="humTrack", asset="sfx_hum", start=0, volume=0.5)}/>')
    emit(f'    <audioTrack {attrs(id="burstTrack", asset="sfx_burst", start=12.9, volume=0.8)}/>')
    for i, (t, hx_, _) in enumerate(sorted(impacts)):
        pan = max(-0.8, min(0.8, (hx_ - CX) / 600))
        emit(f'    <audioTrack {attrs(id=f"impact{i}", asset="sfx_impact", start=round(t, 4), volume=0.7, pan=round(pan, 3))}/>')
    emit(f'    <audioTrack {attrs(id="alarmTrack", asset="sfx_alarm", start=16.3, volume=0.4)}/>')
    emit(f'    <audioTrack {attrs(id="shieldTrack", asset="sfx_shield", start=hit_t, volume=0.8, pan=-0.3)}/>')
    emit(f'    <audioTrack {attrs(id="transmitTrack", asset="sfx_transmit", start=23.2, volume=0.6)}/>')
    emit("  </audioMix>")
    emit("</scene>")
    return contacts


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--render-time-uhd", default="PENDING")
    parser.add_argument("--render-time-360", default="PENDING")
    parser.add_argument("--output", default=os.path.join(ROOT, "examples",
                                                         "archive-beacon.xml"))
    args = parser.parse_args()
    contacts = build(args.render_time_uhd.upper(), args.render_time_360.upper())
    with open(args.output, "w") as f:
        f.write("\n".join(out) + "\n")
    for rid, hit in sorted(contacts.items()):
        where = f"{hit[1]} at {hit[0]:.3f}s" if hit else "no contact"
        print(f"{rid:7s} {where}")
    print(f"wrote {args.output}")


if __name__ == "__main__":
    main()
