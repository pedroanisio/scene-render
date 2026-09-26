#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Derive a scene-format 1.0 animatic from a 1.1 scene, for the current engine.

The 1.0 engine renders only the 1.0 subset, so this keeps the timing, the
layout and the words of a 1.1 document and approximates the rest:

  kept          project, text (styles, spans and {{templates}} resolved),
                rect/ellipse shapes, 1.0 effects and blend modes, masks,
                particles, cameras, 3D primitives, lights, physics
  converted     tokens and paints to flat colours; relative lengths and
                layouts (row/column/grid) to pixels; rounded rects, polygons,
                stars and paths to vector paths; symbols, instances and
                repeats expanded; markers resolved; 1.1 curves to 1.0 curves
                (Penner eases as cubic-bezier, steps as step keys, ping-pong
                and loop unrolled); expressions and motion paths sampled into
                keyframes; text animator presets to opacity/slide keys;
                transitions to crossfades, slides or dips; bursts to rate
                spikes; captions burned in as text
  placeholders  video, image sequences, Lottie, charts (drawn as bars),
                counters, formulas, QR codes, audiograms, includes: a
                labelled panel of the right size
  dropped       everything else, each listed in the report

Usage:
  python3 tools/downlevel-1.0.py IN.xml OUT.xml [--variant ID] [--output PATH]
        [--report FILE] [--no-captions] [--sample-rate HZ]

The result is meant for previews: an animatic that shows the structure and
pacing of the 1.1 scene with today's renderer. Only the standard library is
needed; the expression parser comes from tools/validate-scene.py.
"""

import argparse
import copy
import importlib.util
import json
import math
import os
import re
import sys
import xml.etree.ElementTree as ET

HERE = os.path.dirname(os.path.abspath(__file__))
_spec = importlib.util.spec_from_file_location("validate_scene", os.path.join(HERE, "validate-scene.py"))
VS = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(VS)

# ------------------------------------------------------------------- 1.0 subset
BLENDS_10 = {"normal", "add", "multiply", "screen", "overlay", "difference"}
BLEND_NEAR = {"plus-lighter": "add", "linear-dodge": "add", "color-dodge": "screen", "lighten": "screen",
              "lighter-color": "screen", "soft-light": "overlay", "hard-light": "overlay",
              "darken": "multiply", "color-burn": "multiply", "linear-burn": "multiply",
              "darker-color": "multiply", "exclusion": "difference", "subtract": "difference"}
EFFECTS_10 = {"glow", "bloom", "blur", "color-grade", "vignette", "lens-flare", "drop-shadow", "lighting"}
EFFECT_ATTRS_10 = {"id", "type", "enabled", "intensity", "radius", "threshold", "saturation", "contrast",
                   "brightness", "color", "offsetX", "offsetY", "lights", "falloff", "relief"}
EFFECT_PROPS_10 = {"intensity", "radius", "threshold", "saturation", "contrast", "brightness",
                   "offsetX", "offsetY", "relief", "color"}
CURVES_10 = {"step", "linear", "ease-in", "ease-out", "ease-in-out", "cubic-bezier"}
# easings.net cubic-bezier fits; overshooting ones use y outside 0..1 (allowed by 1.0)
BEZIER = {
    "sine-in": "0.12,0,0.39,0", "sine-out": "0.61,1,0.88,1", "sine-in-out": "0.37,0,0.63,1",
    "quad-in": "0.11,0,0.5,0", "quad-out": "0.5,1,0.89,1", "quad-in-out": "0.45,0,0.55,1",
    "cubic-in": "0.32,0,0.67,0", "cubic-out": "0.33,1,0.68,1", "cubic-in-out": "0.65,0,0.35,1",
    "quart-in": "0.5,0,0.75,0", "quart-out": "0.25,1,0.5,1", "quart-in-out": "0.76,0,0.24,1",
    "quint-in": "0.64,0,0.78,0", "quint-out": "0.22,1,0.36,1", "quint-in-out": "0.83,0,0.17,1",
    "expo-in": "0.7,0,0.84,0", "expo-out": "0.16,1,0.3,1", "expo-in-out": "0.87,0,0.13,1",
    "circ-in": "0.55,0,1,0.45", "circ-out": "0,0.55,0.45,1", "circ-in-out": "0.85,0,0.15,1",
    "back-in": "0.36,0,0.66,-0.56", "back-out": "0.34,1.56,0.64,1", "back-in-out": "0.68,-0.6,0.32,1.6",
    "elastic-out": "0.34,1.56,0.64,1", "elastic-in": "0.36,0,0.66,-0.56", "elastic-in-out": "0.68,-0.6,0.32,1.6",
    "bounce-out": "0.34,1.4,0.64,1", "bounce-in": "0.36,0,0.66,-0.4", "bounce-in-out": "0.68,-0.4,0.32,1.4",
    "spring": "0.34,1.56,0.64,1", "catmull-rom": "0.37,0,0.63,1", "tcb": "0.37,0,0.63,1",
}
NODE_PROP_MAP = {"x": "position.x", "y": "position.y", "scaleX": "scale.x", "scaleY": "scale.y",
                 "anchorX": "anchor.x", "anchorY": "anchor.y", "rotation": "rotation", "opacity": "opacity",
                 "zDepth": "depth", "depth": "depth", "rotationX": "rotation.x", "rotationY": "rotation.y",
                 "position.x": "position.x", "position.y": "position.y", "scale.x": "scale.x",
                 "scale.y": "scale.y", "anchor.x": "anchor.x", "anchor.y": "anchor.y",
                 "rotation.x": "rotation.x", "rotation.y": "rotation.y"}
PARTICLE_PROPS = {"rate", "lifetime", "speed", "spread", "size", "direction"}
PARTICLE_COLOR_PROPS = {"color", "colorEnd"}
CAMERA_PROP_MAP = {"x": "position.x", "y": "position.y", "z": "position.z", "yaw": "yaw", "pitch": "pitch",
                   "roll": "roll", "fov": "fov", "focusDistance": "focusDistance", "aperture": "aperture",
                   "position.x": "position.x", "position.y": "position.y", "position.z": "position.z"}
OBJ_PROP_MAP = {"x": "position.x", "y": "position.y", "z": "position.z", "rotation": "rotation",
                "rotationX": "rotation.x", "rotationY": "rotation.y", "scaleX": "scale.x", "scaleY": "scale.y",
                "scaleZ": "scale.z"}
LIGHT_PROP_MAP = {"intensity": "intensity", "x": "position.x", "y": "position.y", "z": "position.z",
                  "yaw": "yaw", "pitch": "pitch"}
PRIMITIVES_10 = {"sphere", "box", "plane", "mesh"}
PRIMITIVE_NEAR = {"torus": "sphere", "capsule": "sphere", "cylinder": "box", "cone": "sphere",
                  "text": "box", "extrude": "box"}
LIGHT_TYPES_NEAR = {"ambient": "ambient", "directional": "directional", "point": "point", "spot": "spot",
                    "rect-area": "point", "disk-area": "point", "sphere-area": "point", "dome": "ambient"}
PRESETS_10 = {"smoke", "sparks", "dust", "rain"}
PRESET_NEAR = {"snow": "dust", "confetti": "sparks", "fire": "sparks", "bubbles": "dust",
               "bokeh": "dust", "glitter": "sparks"}
EMITTER_ATTRS_10 = ("preset", "rate", "lifetime", "lifetimeVariance", "speed", "speedVariance", "direction",
                    "spread", "gravityX", "gravityY", "size", "sizeEnd", "color", "colorEnd", "maxParticles",
                    "emitterWidth", "emitterHeight", "seed", "shape", "blend")    # ordered: output is byte-stable
FADE_PRESETS = {"fade-in", "word-by-word", "letter-by-letter", "line-by-line", "typewriter", "tracking-in",
                "blur-in", "scale-in", "pop", "mask-reveal", "scramble", "counter", "karaoke", "highlight",
                "ascend", "shift", "bounce", "spin"}
SLIDE_PRESETS = {"slide-up": (0, 28), "slide-down": (0, -28), "slide-left": (28, 0), "slide-right": (-28, 0)}


# ------------------------------------------------------------------- helpers
def num(s, default=0.0):
    try:
        v = float(s)
        return v if math.isfinite(v) else default
    except (TypeError, ValueError):
        return default


def fmt(v):
    if isinstance(v, str):
        return v
    if abs(v - round(v)) < 1e-9:
        return str(int(round(v)))
    return ("%.6f" % v).rstrip("0").rstrip(".")


class Report:
    def __init__(self):
        self.items = []

    def add(self, kind, where, what):
        self.items.append((kind, where, what))

    def summary(self):
        by = {}
        for k, w, t in self.items:
            by.setdefault(k, []).append((w, t))
        return by


# --------------------------------------------------------------- colours
def parse_color(s):
    s = s.strip()
    if s.startswith("#"):
        h = s[1:]
        r, g, b = int(h[0:2], 16) / 255, int(h[2:4], 16) / 255, int(h[4:6], 16) / 255
        a = int(h[6:8], 16) / 255 if len(h) == 8 else 1.0
        return [r, g, b, a]
    parts = [float(x) for x in s.split(",")]
    return parts + [1.0] * (4 - len(parts))


def hex_color(c):
    return "#%02X%02X%02X%02X" % tuple(max(0, min(255, round(v * 255))) for v in c)


def kelvin_rgb(k):
    t = k / 100.0
    r = 255 if t <= 66 else 329.698727446 * (t - 60) ** -0.1332047592
    g = 99.4708025861 * math.log(t) - 161.1195681661 if t <= 66 else 288.1221695283 * (t - 60) ** -0.0755148492
    b = 255 if t >= 66 else (0 if t <= 19 else 138.5177312231 * math.log(t - 10) - 305.0447927307)
    return [max(0, min(255, v)) / 255 for v in (r, g, b)] + [1.0]


# ----------------------------------------------------------------- lengths
REL = re.compile(r"^(-?[\d.]+)(%|vw|vh|vmin|vmax)$")


def length(s, box, axis, frame=(1920, 1080)):
    if s is None:
        return None
    s = str(s).strip()
    m = REL.match(s)
    if not m:
        return num(s)
    v, u = float(m.group(1)), m.group(2)
    if u == "%":
        return v / 100 * (box[0] if axis == "x" else box[1])
    return v / 100 * {"vw": frame[0], "vh": frame[1], "vmin": min(frame), "vmax": max(frame)}[u]


# ---------------------------------------------------------------- paths
def path_from_shape(kind, w, h, a):
    """1.1 procedural shapes as SVG path data in the shape's own box."""
    if kind == "rounded-rect":
        radii = [num(x) for x in a.get("cornerRadii", "").split()] or [num(a.get("radius", "0"))] * 4
        tl, tr, br, bl = [min(r, w / 2, h / 2) for r in (radii + radii * 3)[:4]]
        k = 0.5523                                       # cubic approximation of a quarter circle
        f = lambda v: fmt(round(v, 3))
        return ("M{a} 0 L{b} 0 C{c} 0 {w} {d} {w} {tr} L{w} {e} C{w} {g} {i} {h} {j} {h} L{bl} {h} "
                "C{m} {h} 0 {n} 0 {o} L0 {tl} C0 {q} {r} 0 {a} 0 Z").format(
            a=f(tl), b=f(w - tr), c=f(w - tr + tr * k), d=f(tr - tr * k), w=f(w), tr=f(tr), e=f(h - br),
            g=f(h - br + br * k), i=f(w - br + br * k), j=f(w - br), h=f(h), bl=f(bl), m=f(bl - bl * k),
            n=f(h - bl + bl * k), o=f(h - bl), tl=f(tl), q=f(tl - tl * k), r=f(tl - tl * k))
    cx, cy = w / 2, h / 2
    n = max(3, int(num(a.get("points", "5"), 5)))
    if kind == "polygon":
        pts = [(cx + cx * math.sin(2 * math.pi * i / n), cy - cy * math.cos(2 * math.pi * i / n)) for i in range(n)]
    elif kind == "star":
        ro = num(a.get("outerRadius"), min(cx, cy)) or min(cx, cy)
        ri = num(a.get("innerRadius"), ro * 0.5) or ro * 0.5
        pts = []
        for i in range(2 * n):
            r = ro if i % 2 == 0 else ri
            ang = math.pi * i / n
            pts.append((cx + r * math.sin(ang), cy - r * math.cos(ang)))
    elif kind == "line":
        return "M0 {0} L{1} {0}".format(fmt(cy), fmt(w))
    else:
        return a.get("path", "")
    return "M" + " L".join("%s %s" % (fmt(round(x, 2)), fmt(round(y, 2))) for x, y in pts) + " Z"


def sample_path(d, n=80):
    """Points along SVG path data (M, L, H, V, C, S, Q, T, Z; absolute and
    relative), by arc length."""
    toks = re.findall(r"[MmLlHhVvCcSsQqTtZzAa]|-?[\d.]+(?:e-?\d+)?", d)
    i, cmd, cur, start, pts, prev_c = 0, None, (0.0, 0.0), (0.0, 0.0), [], None

    def take(k):
        nonlocal i
        vals = [float(t) for t in toks[i:i + k]]
        i += k
        return vals
    while i < len(toks):
        if re.match(r"[A-Za-z]", toks[i]):
            cmd = toks[i]
            i += 1
        rel = cmd.islower()
        c = cmd.upper()
        ox, oy = cur if rel else (0.0, 0.0)
        if c == "M":
            x, y = take(2)
            cur = start = (ox + x, oy + y)
            pts.append(cur)
            cmd = "l" if rel else "L"
        elif c == "L":
            x, y = take(2)
            cur = (ox + x, oy + y)
            pts.append(cur)
        elif c == "H":
            x, = take(1)
            cur = ((ox if rel else 0) + x, cur[1])
            pts.append(cur)
        elif c == "V":
            y, = take(1)
            cur = (cur[0], (oy if rel else 0) + y)
            pts.append(cur)
        elif c in "CS":
            if c == "C":
                x1, y1, x2, y2, x, y = take(6)
                p1 = (ox + x1, oy + y1)
            else:
                x2, y2, x, y = take(4)
                p1 = (2 * cur[0] - prev_c[0], 2 * cur[1] - prev_c[1]) if prev_c else cur
            p2, p3, p0 = (ox + x2, oy + y2), (ox + x, oy + y), cur
            for k in range(1, 17):
                t = k / 16
                mt = 1 - t
                pts.append((mt ** 3 * p0[0] + 3 * mt * mt * t * p1[0] + 3 * mt * t * t * p2[0] + t ** 3 * p3[0],
                            mt ** 3 * p0[1] + 3 * mt * mt * t * p1[1] + 3 * mt * t * t * p2[1] + t ** 3 * p3[1]))
            prev_c, cur = p2, p3
            continue
        elif c in "QT":
            x1, y1, x, y = take(4) if c == "Q" else ([0, 0] + take(2))
            p1, p2, p0 = (ox + x1, oy + y1), (ox + x, oy + y), cur
            for k in range(1, 17):
                t = k / 16
                pts.append(((1 - t) ** 2 * p0[0] + 2 * (1 - t) * t * p1[0] + t * t * p2[0],
                            (1 - t) ** 2 * p0[1] + 2 * (1 - t) * t * p1[1] + t * t * p2[1]))
            cur = p2
        elif c == "A":
            vals = take(7)
            cur = (ox + vals[5], oy + vals[6])
            pts.append(cur)
        elif c == "Z":
            cur = start
            pts.append(cur)
        prev_c = None
    if len(pts) < 2:
        return pts
    seg = [math.dist(pts[j], pts[j + 1]) for j in range(len(pts) - 1)]
    total = sum(seg) or 1.0
    out, acc, j = [], 0.0, 0
    for k in range(n + 1):
        target = total * k / n
        while j < len(seg) - 1 and acc + seg[j] < target:
            acc += seg[j]
            j += 1
        f = (target - acc) / seg[j] if seg[j] else 0
        a, b = pts[j], pts[j + 1]
        out.append((a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f))
    return out


# ------------------------------------------------------------ expressions
def hash01(*v):
    h = 1469598103934665603
    for x in v:
        for ch in repr(x).encode():
            h = ((h ^ ch) * 1099511628211) & 0xFFFFFFFFFFFFFFFF
    return (h >> 11) / float(1 << 53)


def vnoise(x, seed):
    i = math.floor(x)
    f = x - i
    a, b = hash01(seed, i) * 2 - 1, hash01(seed, i + 1) * 2 - 1
    s = f * f * (3 - 2 * f)
    return a + (b - a) * s


class Evaluator:
    def __init__(self, conv):
        self.c = conv

    def run(self, src, env):
        e = VS.Expr(src)
        tree = e.parse()
        # const bindings are re-parsed in order
        binds = {}
        for m in re.finditer(r"const\s+([A-Za-z_$][\w$]*)\s*=\s*(.*?);", src, re.S):
            binds[m.group(1)] = self.eval(VS.Expr(m.group(2)).parse(), {**env, **binds})
        return self.eval(tree, {**env, **binds})

    def eval(self, n, env):
        k = n[0]
        if k == "num":
            return n[1]
        if k == "str":
            return n[1]
        if k == "id":
            name = n[1]
            if name in env:
                return env[name]
            if name in ("true", "false"):
                return name == "true"
            if name == "PI":
                return math.pi
            raise KeyError(name)
        if k == "array":
            return [self.eval(x, env) for x in n[1]]
        if k == "?":
            return self.eval(n[2], env) if self.eval(n[1], env) else self.eval(n[3], env)
        if k in ("u-", "u+", "u!"):
            v = self.eval(n[1], env)
            return -v if k == "u-" else (v if k == "u+" else not v)
        if k == "member":
            base = n[1]
            if base[0] == "id" and base[1] == "Math":
                return {"PI": math.pi, "E": math.e}.get(n[2], n[2])
            v = self.eval(base, env)
            return v.get(n[2]) if isinstance(v, dict) else None
        if k == "index":
            return self.eval(n[1], env)[int(self.eval(n[2], env))]
        if k == "call":
            f = n[1]
            name = f[2] if f[0] == "member" else f[1]
            args = [self.eval(a, env) for a in n[2]]
            return self.call(name, args, env)
        a, b = self.eval(n[1], env), self.eval(n[2], env)
        ops = {"+": lambda: a + b, "-": lambda: a - b, "*": lambda: a * b, "/": lambda: a / b if b else 0.0,
               "%": lambda: math.fmod(a, b) if b else 0.0, "**": lambda: a ** b, "<": lambda: a < b,
               "<=": lambda: a <= b, ">": lambda: a > b, ">=": lambda: a >= b, "==": lambda: a == b,
               "===": lambda: a == b, "!=": lambda: a != b, "!==": lambda: a != b,
               "&&": lambda: a and b, "||": lambda: a or b}
        return ops[k]()

    def call(self, name, a, env):
        t, seed = env["time"], env.get("seed", 0)
        if hasattr(math, name) and name not in ("clamp",):
            return getattr(math, name)(*a)
        if name in ("min", "max", "abs", "round"):
            return {"min": min, "max": max, "abs": abs, "round": round}[name](*a)
        if name == "sign":
            return (a[0] > 0) - (a[0] < 0)
        if name == "clamp":
            return max(a[1], min(a[2], a[0]))
        if name == "lerp":
            return a[0] + (a[1] - a[0]) * a[2]
        if name == "smoothstep":
            x = max(0.0, min(1.0, (a[2] - a[0]) / ((a[1] - a[0]) or 1)))
            return x * x * (3 - 2 * x)
        if name in ("linear", "ease", "easeIn", "easeOut"):
            x, t0, t1, v0, v1 = (a if len(a) == 5 else [a[0], 0, 1, a[1], a[2]])
            p = max(0.0, min(1.0, (x - t0) / ((t1 - t0) or 1)))
            if name == "ease":
                p = p * p * (3 - 2 * p)
            elif name == "easeIn":
                p = p * p
            elif name == "easeOut":
                p = 1 - (1 - p) ** 2
            return v0 + (v1 - v0) * p
        if name == "wiggle":
            freq, amp = a[0], a[1]
            octv, mult = (int(a[2]) if len(a) > 2 else 1), (a[3] if len(a) > 3 else 0.5)
            s, w, tot = 0.0, 1.0, 0.0
            for o in range(max(1, octv)):
                s += w * vnoise(t * freq * (2 ** o), (seed, o))
                tot += w
                w *= mult
            return env.get("value", 0.0) + amp * s / tot
        if name == "noise":
            return vnoise(a[0], seed)
        if name == "random":
            r = hash01(seed, env.get("frame", 0))
            return r if not a else (a[0] + (a[1] - a[0]) * r if len(a) == 2 else a[0] * r)
        if name == "spring":
            tt, k, d, m = a
            w0 = math.sqrt(k / m)
            z = d / (2 * math.sqrt(k * m))
            if z < 1:
                wd = w0 * math.sqrt(1 - z * z)
                return 1 - math.exp(-z * w0 * tt) * (math.cos(wd * tt) + z * w0 / wd * math.sin(wd * tt))
            return 1 - math.exp(-w0 * tt) * (1 + w0 * tt)
        if name == "param":
            return self.c.param_value(a[0])
        if name == "markerTime":
            return self.c.marker_time(a[0])
        if name == "beat":
            return self.c.beat(t)
        if name in ("audioAmplitude",):
            return 0.0
        if name in ("valueAtTime", "loopIn", "loopOut"):
            return env.get("value", 0.0)
        if name == "prop":
            return 0.0
        raise KeyError(name)


# ------------------------------------------------------------- converter
class Converter:
    def __init__(self, src, args):
        self.args, self.report = args, Report()
        self.src_path = src
        self.doc = ET.parse(src).getroot()
        self.parent = {c: p for p in self.doc.iter() for c in p}
        self.ids = {e.get("id"): e for e in self.doc.iter() if e.get("id")}
        proj = self.doc.find("project")
        self.W, self.H = int(proj.get("width")), int(proj.get("height"))
        self.fps = eval_fps(proj.get("fps"))
        self.duration = num(proj.get("duration"))
        self.seed = int(num(proj.get("seed", "0")))
        self.tokens = {t.get("name"): t.get("value") for t in self.doc.iter("token")}
        self.params = {p.get("id"): p for p in self.doc.iter("param") if self.parent[p].tag == "parameters"}
        self.param_vals = {k: p.get("default") for k, p in self.params.items()}
        self.variant_overrides = []
        variant = args.variant or self.default_variant()
        if variant:
            v = self.ids.get(variant)
            if v is None or v.tag != "variant":
                raise SystemExit("error: no variant %r" % variant)
            for s in v:
                if s.tag == "set":
                    self.param_vals[s.get("param")] = s.get("value")
                elif s.tag == "override":
                    self.variant_overrides.append(s)
            self.report.add("converted", "variant", "applied variant %r" % variant)
        self.markers = {m.get("id"): num(m.get("time")) for m in self.doc.iter("marker") if m.get("id")}
        bg = next(iter(self.doc.iter("beatGrid")), None)
        self.bpm = num(bg.get("bpm"), 120) if bg is not None else 120.0
        self.beat_offset = num(bg.get("offset", "0")) if bg is not None else 0.0
        self.binds = [b for b in self.doc.iter("bind")]
        self.ev = Evaluator(self)
        self.assets_out, self.used_assets, self.effects_used = [], {}, set()
        self.clone_count = 0
        self.image_assets = set()
        self.effect_elems = {e.get("id"): e for e in self.doc.iter("effect")}
        self.frame_effects = []                    # effect ids to apply to the whole frame
        self.extra_effects = []

    def default_variant(self):
        outs = [o for o in self.doc.findall("output") if o.get("variant")]
        return outs[0].get("variant") if outs else None

    # ------------------------------------------------ value helpers
    def param_value(self, name):
        v = self.param_vals.get(name)
        p = self.params.get(name)
        if p is None:
            return None
        t = p.get("type")
        if t == "boolean":
            return v in ("true", "1")
        if t in ("number", "time"):
            return num(v)
        return v

    def marker_time(self, mid):
        if mid in self.markers:
            return self.markers[mid]
        m = re.match(r"(beat|bar)\.(\d+)$", mid or "")
        if m:
            per = 60.0 / self.bpm * (4 if m.group(1) == "bar" else 1)
            return self.beat_offset + int(m.group(2)) * per
        return 0.0

    def beat(self, t):
        return (t - self.beat_offset) * self.bpm / 60.0

    def color(self, s, where):
        """A colour or paint as a 1.0 colour (#RRGGBBAA)."""
        if s is None:
            return None
        s = s.strip()
        for _ in range(4):
            m = re.fullmatch(r"var\(--([\w-]+)\)", s)
            if not m:
                break
            s = self.tokens.get(m.group(1), "#FF00FFFF")
        m = re.fullmatch(r"url\(#([\w.-]+)\)", s)
        if m:
            p = self.ids.get(m.group(1))
            c = self.paint_average(p)
            self.report.add("converted", where, "paint url(#%s) flattened to %s" % (m.group(1), hex_color(c)))
            return hex_color(c)
        try:
            return hex_color(parse_color(s))
        except (ValueError, IndexError):
            return "#FF00FFFF"

    def paint_average(self, p):
        if p is None:
            return [1, 0, 1, 1]
        if p.tag == "meshGradient":
            cols = [parse_color(self.color(pt.get("color"), "")) for pt in p if pt.tag == "point"]
        elif p.tag == "pattern":
            a = self.ids.get(p.get("asset"))
            return parse_color(self.color(a.get("paint", "#808080FF"), "")) if a is not None else [0.5] * 3 + [1]
        else:
            stops = sorted(((num(s.get("offset")), parse_color(self.color(s.get("color"), ""))) for s in p if s.tag == "stop"))
            if not stops:
                return [1, 0, 1, 1]
            cols, prev = [], None
            weights = []
            for i, (o, c) in enumerate(stops):
                lo = stops[i - 1][0] if i else 0.0
                hi = stops[i + 1][0] if i + 1 < len(stops) else 1.0
                weights.append(max(1e-6, (hi - lo) / 2 + (o - lo if i == 0 else 0) + (hi - o if i == len(stops) - 1 else 0)))
                cols.append(c)
            tot = sum(weights)
            return [sum(w * c[k] for w, c in zip(weights, cols)) / tot for k in range(4)]
        return [sum(c[k] for c in cols) / len(cols) for k in range(4)] if cols else [1, 0, 1, 1]

    def substitute(self, text, row=None, var=None):
        def rep(m):
            name = m.group(1).strip()
            base, _, field = name.partition(".")
            if var and base == var and row is not None:
                return str(row.get(field, "") if field else row)
            if name in self.param_vals:
                return str(self.param_vals[name] or "")
            return m.group(0)
        return VS.TEMPLATE_RE.sub(rep, text)

    # ---------------------------------------------------- time maps
    @staticmethod
    def tmap(tm, t):
        return tm[0] * t + tm[1]

    def resolve_time(self, e, attr, tm, marker_attr=None):
        """A time attribute in composition time, with markers and E10 offsets."""
        base = 0.0
        if marker_attr and e.get(marker_attr) in self.markers:
            base = self.markers[e.get(marker_attr)]
            v = num(e.get(attr, "0"))
            return base + v                              # markers are composition times
        if e.get(attr) is None:
            return None
        return self.tmap(tm, num(e.get(attr)))

    def window(self, e, tm):
        s = self.resolve_time(e, "start", tm, "startMarker")
        en = self.resolve_time(e, "end", tm, "endMarker")
        return (s if s is not None else self.tmap(tm, 0.0)), en

    # ------------------------------------------------------ keyframes
    def keys_of(self, anim, tm, host_start):
        base = anim.get("defaultInterpolation", "linear")
        tb = anim.get("timeBase", "composition")
        keys = []
        for k in anim:
            if k.tag != "key":
                continue
            t = num(k.get("time"))
            if k.get("marker"):
                t = self.marker_time(k.get("marker")) + t
            elif tb == "local":
                t = host_start + t * tm[0]
            else:
                t = self.tmap(tm, t)
            keys.append({"t": t, "v": k.get("value"), "i": k.get("interpolation") or base, "k": k})
        keys.sort(key=lambda x: x["t"])
        return keys

    CSS_EASE = {"ease-in": "0.42,0,1,1", "ease-out": "0,0,0.58,1", "ease-in-out": "0.42,0,0.58,1"}

    def progress(self, key, u):
        c = key["i"]
        if c in ("step", "hold"):
            return 0.0
        if c == "steps":
            n = max(1, int(num(key["k"].get("steps"), 1)))
            return math.floor(u * n) / n
        if c == "linear":
            return u
        b = key["k"].get("bezier") if c == "cubic-bezier" else (self.CSS_EASE.get(c) or BEZIER.get(c))
        if not b:
            return u
        x1, y1, x2, y2 = [float(v) for v in b.split(",")]
        lo, hi, t = 0.0, 1.0, u
        for _ in range(30):
            bx = 3 * (1 - t) ** 2 * t * x1 + 3 * (1 - t) * t * t * x2 + t ** 3
            lo, hi = (t, hi) if bx < u else (lo, t)
            t = (lo + hi) / 2
        return 3 * (1 - t) ** 2 * t * y1 + 3 * (1 - t) * t * t * y2 + t ** 3

    def eval_anim(self, anim, t, tm, host_start):
        keys = self.keys_of(anim, tm, host_start)
        if not keys:
            return None
        t0, t1 = keys[0]["t"], keys[-1]["t"]
        span = t1 - t0
        ex = anim.get("extrapolateAfter", "hold")
        if t > t1 and span > 0 and ex in ("loop", "ping-pong"):
            c, r = divmod(t - t0, span)
            t = t0 + (span - r if ex == "ping-pong" and int(c) % 2 else r)
        if t <= t0:
            return num(keys[0]["v"])
        if t >= t1:
            return num(keys[-1]["v"])
        for a, b in zip(keys, keys[1:]):
            if a["t"] <= t <= b["t"]:
                u = (t - a["t"]) / ((b["t"] - a["t"]) or 1)
                return num(a["v"]) + (num(b["v"]) - num(a["v"])) * self.progress(a, u)
        return num(keys[-1]["v"])

    def value_at(self, e, prop, t, tm=(1.0, 0.0), default=0.0):
        """A 1.1 property's value at composition time t (keys, then expression)."""
        host_start = self.window(e, tm)[0]
        v = num(e.get(prop), default) if e.get(prop) is not None else default
        for a in e:
            if a.get("property") != prop:
                continue
            if a.tag == "animate":
                r = self.eval_anim(a, t, tm, host_start)
                v = v if r is None else r
            elif a.tag == "expression":
                try:
                    v = float(self.ev.run(a.text or "", {"time": t, "frame": int(t * self.fps), "value": v,
                                                        "seed": num(a.get("seed")) if a.get("seed") else hash01(e.get("id"), self.seed)}))
                except Exception:
                    pass
        return v

    def camera_for(self, window):
        s = window[0] or 0.0
        cams = [c for c in self.doc.iter("camera") if c.get("active", "true") != "false"]
        best = None
        for c in cams:
            cs, ce = self.window(c, (1.0, 0.0))
            if (cs or 0) <= s + 1e-6 and (ce is None or ce > s):
                best = c
        return best if best is not None else (cams[0] if cams else None)

    def curve10(self, key, where):
        c = key["i"]
        if c in CURVES_10 and c != "cubic-bezier":
            return c, None
        if c == "cubic-bezier":
            b = key["k"].get("bezier")
            if b:
                return "cubic-bezier", b
            eo = key["k"].get("easeOut")
            if eo:
                inf = num(eo.split(",")[0], 0.33)
                return "cubic-bezier", "%s,0,%s,1" % (fmt(min(1, inf)), fmt(1 - min(1, inf)))
            return "ease-in-out", None
        if c == "hold":
            return "step", None
        if c in BEZIER:
            if c.startswith(("elastic", "bounce", "spring")):
                self.report.add("approximated", where, "%s curve as a single overshoot" % c)
            return "cubic-bezier", BEZIER[c]
        return "linear", None

    def emit_animate(self, parent, prop, keys, where, value_fn=None, host_window=None):
        """Write a 1.0 <animate> from key dicts, expanding steps and extrapolation."""
        if not keys:
            return
        out = []
        for i, k in enumerate(keys):
            v = value_fn(k["v"]) if value_fn else k["v"]
            if k["i"] == "steps" and i + 1 < len(keys):
                n = max(1, int(num(k["k"].get("steps"), 1)))
                v0, v1 = num(v), num(value_fn(keys[i + 1]["v"]) if value_fn else keys[i + 1]["v"])
                t0, t1 = k["t"], keys[i + 1]["t"]
                end = k["k"].get("stepPosition", "end") == "end"
                for s in range(n):
                    frac = (s if end else s + 1) / n
                    out.append((t0 + (t1 - t0) * s / n, fmt(v0 + (v1 - v0) * frac), "step", None))
                continue
            curve, bez = self.curve10(k, where)
            out.append((k["t"], v, curve, bez))
        anim = self.parent.get(keys[0]["k"])
        extra = anim.get("extrapolateAfter") if anim is not None else None
        if extra in ("ping-pong", "loop") and len(out) >= 2 and host_window:
            # A key's curve shapes the segment that starts at it. Unrolled
            # segments reuse the first key's curve (exact for two-key tracks).
            t0, span = out[0][0], out[-1][0] - out[0][0]
            c0, b0 = out[0][2], out[0][3]
            end = host_window[1] if host_window[1] is not None else self.duration
            seq, c = list(out), 1
            while span > 0 and t0 + c * span < end and c < 400:
                base = t0 + c * span
                if extra == "ping-pong":
                    seq[-1] = (seq[-1][0], seq[-1][1], c0, b0)
                    vals = [x[1] for x in (reversed(out) if c % 2 else out)]
                    for i in range(1, len(out)):
                        seq.append((base + (out[i][0] - t0), vals[i], c0, b0))
                else:                                    # loop: hold, jump to the first value, replay
                    seq[-1] = (seq[-1][0], seq[-1][1], "step", None)
                    seq.append((base + 1.0 / self.fps, out[0][1], c0, b0))
                    for i in range(1, len(out)):
                        seq.append((base + (out[i][0] - t0), out[i][1], out[i][2], out[i][3]))
                c += 1
            out = seq
            self.report.add("converted", where, "%s extrapolation unrolled to %d keys" % (extra, len(out)))
        a = ET.SubElement(parent, "animate", {"property": prop})
        seen = set()
        for t, v, curve, bez in out:
            tt = round(t, 6)
            if tt in seen:
                tt = round(tt + 1e-4, 6)
            seen.add(tt)
            attrs = {"time": fmt(tt), "value": str(v)}
            if curve != "linear":
                attrs["interpolation"] = curve
            if bez:
                attrs["bezier"] = bez
            ET.SubElement(a, "key", attrs)

    def sample(self, parent, prop, src, window, where, base=0.0, seed=None, rate=None, value_fn=None):
        """Sample an expression over [start, end) into linear keys."""
        s, e = window
        e = e if e is not None else self.duration
        rate = rate or self.args.sample_rate
        n = max(2, int(math.ceil((e - s) * rate)) + 1)
        seed = seed if seed is not None else hash01(where, self.seed)
        vals = []
        for i in range(n):
            t = s + (e - s) * i / (n - 1)
            try:
                v = self.ev.run(src, {"time": t, "frame": int(t * self.fps), "value": base, "seed": seed,
                                      "index": 0, "count": 1})
            except Exception as ex:                      # unsupported construct: keep the static value
                self.report.add("dropped", where, "expression %r (%s)" % (src.strip()[:40], ex))
                return
            if isinstance(v, bool):
                v = 1.0 if v else 0.0
            if not isinstance(v, (int, float)):
                self.report.add("dropped", where, "non-numeric expression %r" % src.strip()[:40])
                return
            vals.append((t, value_fn(v) if value_fn else v))
        span = max(v for _, v in vals) - min(v for _, v in vals)
        if span < 1e-9:
            return vals[0][1]
        keep = [vals[0]]
        for i in range(1, len(vals) - 1):                # drop points on the line between neighbours
            (t0, v0), (t1, v1), (t2, v2) = keep[-1], vals[i], vals[i + 1]
            if abs(v0 + (v2 - v0) * (t1 - t0) / ((t2 - t0) or 1) - v1) > span * 1e-4:
                keep.append(vals[i])
        vals = keep + [vals[-1]]
        a = ET.SubElement(parent, "animate", {"property": prop})
        for t, v in vals:
            ET.SubElement(a, "key", {"time": fmt(round(t, 5)), "value": fmt(round(v, 5))})
        self.report.add("converted", where, "expression on %s sampled at %g Hz" % (prop, rate))
        return None

    # -------------------------------------------- animations of one host
    def animations(self, src, dst, prop_map, tm, window, where, box, color_props=(), length_props=(),
                   value_scale=None):
        host_start = window[0]
        for a in list(src):
            if a.tag not in ("animate", "expression", "link", "motionPath"):
                continue
            prop = a.get("property")
            if a.tag == "motionPath":
                self.motion_path(a, dst, tm, where)
                continue
            if a.tag == "link":
                self.report.add("dropped", where, "link on %s (source %s)" % (prop, a.get("source")))
                continue
            target = prop_map.get(prop) if prop not in color_props else prop
            if target is None:
                self.report.add("dropped", where, "%s of %r (no 1.0 equivalent)" % (a.tag, prop))
                continue
            if prop in color_props:
                vf = lambda v, w=where: self.color(v, w)
            elif prop in length_props:
                axis = "x" if prop in ("x", "width", "anchorX", "position.x") else "y"
                vf = lambda v, ax=axis: fmt(length(v, box, ax, (self.W, self.H)))
            elif value_scale and prop in value_scale:
                vf = lambda v, f=value_scale[prop]: fmt(f(num(v)))
            else:
                vf = None
            if a.tag == "animate":
                self.emit_animate(dst, target, self.keys_of(a, tm, host_start), where, vf, window)
            else:
                base = num(src.get(prop, "0")) if src.get(prop) else (1.0 if prop.startswith("scale") or prop == "opacity" else 0.0)
                if prop in length_props and src.get(prop):
                    base = length(src.get(prop), box, "x" if prop in ("x", "anchorX") else "y", (self.W, self.H))
                seed = num(a.get("seed")) if a.get("seed") else None
                static = self.sample(dst, target, a.text or "", window, where + "@" + prop, base, seed,
                                     value_fn=(value_scale or {}).get(prop))
                if static is not None:
                    self.report.add("converted", where, "constant expression %s = %s" % (prop, fmt(static)))

    def motion_path(self, mp, dst, tm, where):
        pts = sample_path(mp.get("path", ""), 60)
        if len(pts) < 2:
            return
        s = self.tmap(tm, num(mp.get("start", "0")))
        e = self.tmap(tm, num(mp.get("end"), 0)) if mp.get("end") else s + 1
        ax = ET.SubElement(dst, "animate", {"property": "position.x"})
        ay = ET.SubElement(dst, "animate", {"property": "position.y"})
        ar = ET.SubElement(dst, "animate", {"property": "rotation"}) if mp.get("autoOrient") == "true" else None
        ease = mp.get("interpolation", "linear")
        for i, (x, y) in enumerate(pts):
            u = i / (len(pts) - 1)
            if ease == "ease-in-out":
                u2 = u * u * (3 - 2 * u)
            else:
                u2 = u
            t = s + (e - s) * u2
            ET.SubElement(ax, "key", {"time": fmt(round(t, 5)), "value": fmt(round(x, 3))})
            ET.SubElement(ay, "key", {"time": fmt(round(t, 5)), "value": fmt(round(y, 3))})
            if ar is not None:
                j = min(i + 1, len(pts) - 1)
                k = max(j - 1, 0)
                ang = math.degrees(math.atan2(pts[j][1] - pts[k][1], pts[j][0] - pts[k][0])) + num(mp.get("orientOffset", "0"))
                ET.SubElement(ar, "key", {"time": fmt(round(t, 5)), "value": fmt(round(ang, 3))})
        self.report.add("converted", where, "motion path sampled into %d position keys" % len(pts))

    # ------------------------------------------------------ assets
    def text_asset(self, src_id, text_override=None, row=None, var=None):
        """A 1.0 text asset for the 1.1 text asset src_id; clones when the
        text differs (templates, overrides, rows)."""
        a = self.ids[src_id]
        style = self.style_chain(a.get("style"))
        if a.get("text") is not None:
            text = a.get("text")
        else:
            text = "".join((s.text or "") for s in a if s.tag == "span")
            if any(s.tag == "span" for s in a):
                self.report.add("approximated", src_id, "styled spans flattened to plain text")
        if text_override is not None:
            text = text_override
        text = self.substitute(text, row, var)
        text = VS.TEMPLATE_RE.sub("", text)
        tf = style.get("textTransform") or a.get("textTransform")
        if tf == "uppercase":
            text = text.upper()
        elif tf == "lowercase":
            text = text.lower()
        key = (src_id, text)
        if key in self.used_assets:
            return self.used_assets[key]
        new_id = src_id if not any(k[0] == src_id for k in self.used_assets) else "%s--%d" % (src_id, len(self.used_assets))
        size = num(a.get("size") or style.get("size"), 40)
        if (a.get("autoFit") or "none") in ("shrink", "fit") and text:
            w, h = num(a.get("width")), num(a.get("height"))
            lines = max(1, min(int(num(a.get("maxLines"), 99)), int(h / (size * 1.2)) or 1))
            fit = w * lines / (len(text) * 0.56)
            lo = num(a.get("minSize"), 1)
            if fit < size:
                size = max(lo, fit)
                self.report.add("approximated", src_id, "autoFit estimated as size %s" % fmt(round(size, 1)))
        attrs = {"id": new_id, "text": text or " ", "width": a.get("width"), "height": a.get("height"),
                 "size": fmt(size), "color": self.color(a.get("color") or style.get("color") or "#FFFFFFFF", src_id)}
        font = a.get("font") or style.get("font")
        if font:
            attrs["font"] = font
        for k in ("align", "lineHeight", "direction", "language", "verticalAlign"):
            v = a.get(k) or style.get(k)
            if v and not (k == "align" and v not in ("start", "center", "end", "justify")):
                attrs[k] = v
        tr = a.get("tracking") or style.get("tracking")
        if tr:
            attrs["letterSpacing"] = fmt(round(max(-size, min(4 * size, num(tr) * size / 1000)), 3))
        if a.get("language") and len(a.get("language")) > 35:
            del attrs["language"]
        el = ET.Element("text", attrs)
        self.assets_out.append(el)
        self.used_assets[key] = new_id
        return new_id

    def style_chain(self, sid, seen=()):
        if not sid or sid in seen or sid not in self.ids:
            return {}
        s = self.ids[sid]
        base = self.style_chain(s.get("basedOn"), seen + (sid,))
        return {**base, **{k: v for k, v in s.attrib.items() if k not in ("id", "basedOn")}}

    def vector_asset(self, vid, kind, w, h, a, fill, stroke, sw, where):
        path = path_from_shape(kind, w, h, a) if kind != "path" else a.get("path", "")
        if kind in ("rect", "ellipse"):
            attrs = {"id": vid, "shape": kind, "width": fmt(max(1, round(w))), "height": fmt(max(1, round(h)))}
        else:
            attrs = {"id": vid, "shape": "path", "width": fmt(max(1, round(w))), "height": fmt(max(1, round(h))),
                     "path": path, "fillRule": a.get("fillRule", "nonzero")}
        attrs["fill"] = fill
        if stroke:
            attrs["stroke"] = stroke
        if sw:
            attrs["strokeWidth"] = sw
        self.assets_out.append(ET.Element("vector", attrs))
        return vid

    def uid(self, base):
        self.clone_count += 1
        return "%s--c%d" % (base, self.clone_count)

    # -------------------------------------------------- node helpers
    def node_attrs(self, e, tm, box, where, pos=None, keep_id=True, id_prefix=""):
        out = {}
        nid = (id_prefix + e.get("id")) if e.get("id") else self.uid("node")
        out["id"] = nid
        s, en = self.window(e, tm)
        if s and abs(s) > 1e-9:
            out["start"] = fmt(round(s, 6))
        if en is not None:
            out["end"] = fmt(round(en, 6))
        for k in ("z", "visible", "opacity"):
            if e.get(k) is not None:
                out[k] = e.get(k)
        x = length(e.get("x"), box, "x", (self.W, self.H)) if e.get("x") else 0.0
        y = length(e.get("y"), box, "y", (self.W, self.H)) if e.get("y") else 0.0
        if pos:
            x, y = pos
        if x:
            out["x"] = fmt(round(x, 4))
        if y:
            out["y"] = fmt(round(y, 4))
        for k in ("rotation", "scaleX", "scaleY"):
            if e.get(k) is not None:
                out[k] = e.get(k)
        for k in ("anchorX", "anchorY"):
            if e.get(k) is not None:
                out[k] = fmt(length(e.get(k), box, "x" if k == "anchorX" else "y", (self.W, self.H)))
        if e.get("threeD") == "true" or e.get("zDepth"):
            self.report.add("approximated", where, "2.5D card drawn flat (1.0 card space differs)")
        for k in ("skewX", "skewY", "matte", "parent", "motionBlur", "alignX", "alignY", "tags", "name"):
            if e.get(k) and k not in ("name", "tags"):
                self.report.add("dropped", where, "@%s" % k)
        if e.get("condition"):
            try:
                ok = self.ev.run(e.get("condition"), {"time": 0, "frame": 0, "value": 0, "seed": 0})
            except Exception:
                ok = True
            if not ok:
                return None
            self.report.add("converted", where, "condition %r evaluated to true" % e.get("condition"))
        return out

    def blend(self, e, where):
        b = e.get("blend", "normal")
        if b in BLENDS_10:
            return b
        near = BLEND_NEAR.get(b, "normal")
        self.report.add("approximated", where, "blend %s as %s" % (b, near))
        return near

    def masks(self, src, dst, tm, window, box, where):
        for m in src:
            if m.tag != "mask":
                continue
            t = m.get("type")
            if t not in ("rect", "ellipse", "rounded-rect"):
                self.report.add("dropped", where, "%s mask" % t)
                continue
            attrs = {"type": t}
            for k, ax in (("x", "x"), ("y", "y"), ("width", "x"), ("height", "y")):
                if m.get(k):
                    attrs[k] = fmt(max(1e-3, length(m.get(k), box, ax, (self.W, self.H))) if k in ("width", "height")
                                   else length(m.get(k), box, ax, (self.W, self.H)))
            if m.get("radius"):
                attrs["radius"] = m.get("radius")
            if m.get("invert") == "true":
                attrs["invert"] = "true"
            if m.get("feather") or m.get("mode", "intersect") != "intersect":
                self.report.add("dropped", where, "mask feather/mode")
            me = ET.SubElement(dst, "mask", attrs)
            self.animations(m, me, {"x": "x", "y": "y", "width": "width", "height": "height", "radius": "radius"},
                            tm, window, where + "/mask", box, length_props=("x", "y", "width", "height"))

    def wrap_effects(self, el, e, where):
        """1.0 only puts effects on groups: wrap the element when needed."""
        ids = [x for x in (e.get("effects") or "").split() if self.effect_ok(x, where)]
        if not ids:
            return el
        g = ET.Element("group", {"id": el.get("id") + "--fx", "effects": " ".join(ids)})
        for k in ("start", "end", "z"):
            if el.get(k) is not None:
                g.set(k, el.get(k))
        g.append(el)
        return g

    def effect_ok(self, eid, where):
        e = self.effect_elems.get(eid)
        if e is None:
            return False
        if e.get("type") not in EFFECTS_10:
            self.report.add("dropped", where, "effect %s (%s)" % (eid, e.get("type")))
            return False
        self.effects_used.add(eid)
        return True

    # ------------------------------------------------------- nodes
    def convert_children(self, parent_src, dst, tm, box, prefix, ctx):
        kids = [c for c in parent_src if c.tag in VS.NODE or c.tag in ("transition", "skeleton")]
        layout = parent_src.get("layout", "none") if parent_src.tag in ("group", "sequence") else "none"
        positions = self.layout(parent_src, kids, box) if layout != "none" else {}
        for c in kids:
            for el in self.node(c, tm, box, prefix, ctx, positions.get(id(c))):
                dst.append(el)

    def size_of(self, e, box):
        if e.tag == "shape" or (e.tag == "group" and e.get("width")):
            return (length(e.get("width"), box, "x", (self.W, self.H)) or 0, length(e.get("height"), box, "y", (self.W, self.H)) or 0)
        if e.tag == "layer":
            a = self.ids.get(e.get("asset"))
            if a is not None and a.get("width"):
                sx, sy = num(e.get("scaleX", "1"), 1), num(e.get("scaleY", "1"), 1)
                return num(a.get("width")) * sx, num(a.get("height")) * sy
        return (0, 0)

    def layout(self, g, kids, box):
        gb = (length(g.get("width"), box, "x", (self.W, self.H)) or box[0], length(g.get("height"), box, "y", (self.W, self.H)) or box[1])
        gap = length(g.get("gap", "0"), gb, "x", (self.W, self.H)) or 0
        pad = length(g.get("padding", "0"), gb, "x", (self.W, self.H)) or 0
        mode = g.get("layout")
        sizes = [self.size_of(k, gb) for k in kids]
        pos = {}
        if mode == "grid":
            cols = int(num(g.get("gridColumns", "2"), 2))
            cw = (gb[0] - 2 * pad - gap * (cols - 1)) / cols
            rows = math.ceil(len(kids) / cols)
            rh = (gb[1] - 2 * pad - gap * (rows - 1)) / max(1, rows)
            for i, (k, (w, h)) in enumerate(zip(kids, sizes)):
                r, c = divmod(i, cols)
                pos[id(k)] = (pad + c * (cw + gap) + (cw - w) / 2, pad + r * (rh + gap) + (rh - h) / 2)
        else:
            horiz = mode in ("row", "stack")
            main = [s[0] if horiz else s[1] for s in sizes]
            cross = [s[1] if horiz else s[0] for s in sizes]
            avail = (gb[0] if horiz else gb[1]) - 2 * pad
            used = sum(main) + gap * (len(kids) - 1)
            j = g.get("justify", "start")
            n = len(kids)
            free = max(0.0, avail - used)
            start, between = 0.0, gap
            if j == "center":
                start = free / 2
            elif j == "end":
                start = free
            elif j == "space-between" and n > 1:
                between = gap + free / (n - 1)
            elif j == "space-around":
                between, start = gap + free / n, free / n / 2
            elif j == "space-evenly":
                between, start = gap + free / (n + 1), free / (n + 1)
            cur = pad + start
            cavail = (gb[1] if horiz else gb[0]) - 2 * pad
            for k, m, c in zip(kids, main, cross):
                al = g.get("alignItems", "start")
                off = pad + ((cavail - c) / 2 if al == "center" else (cavail - c if al == "end" else 0))
                pos[id(k)] = (cur, off) if horiz else (off, cur)
                cur += m + between
        self.report.add("converted", g.get("id"), "%s layout resolved to absolute positions" % mode)
        return pos

    def matte_sources(self):
        if not hasattr(self, "_mattes"):
            self._mattes = {}
            for n in self.doc.iter():
                if n.get("matte"):
                    self._mattes.setdefault(n.get("matte"), []).append(n)
        return self._mattes

    def node(self, e, tm, box, prefix, ctx, pos=None):
        where = prefix + (e.get("id") or e.tag)
        users = self.matte_sources().get(e.get("id"))
        if users and not any(u.get("matteVisible") == "true" for u in users):
            self.report.add("converted", where, "matte source not drawn (1.1 hides mattes)")
            return []
        h = getattr(self, "n_" + e.tag.replace("3D", "3d"), None)
        if h is None:
            self.report.add("dropped", where, "<%s>" % e.tag)
            return []
        return h(e, tm, box, prefix, ctx, pos, where) or []

    def n_group(self, e, tm, box, prefix, ctx, pos, where):
        attrs = self.node_attrs(e, tm, box, where, pos, id_prefix=prefix)
        if attrs is None:
            return []
        ov = self.overrides_for(e, ctx)
        for k, v in ov.items():
            if k in ("rotation", "scaleX", "scaleY", "opacity"):
                attrs[k] = v
        g = ET.Element("group", attrs)
        b = self.blend(e, where)
        if b != "normal":
            g.set("blend", b)
        ids = [x for x in (e.get("effects") or "").split() if self.effect_ok(x, where)]
        if ids:
            g.set("effects", " ".join(ids))
        for k in ("isolate", "collapse", "clip"):
            if e.get(k) == "true":
                self.report.add("dropped", where, "@%s" % k)
        tscale = num(e.get("timeScale", "1"), 1)
        if e.get("timeOffset") or tscale != 1:
            self.report.add("dropped", where, "timeOffset/timeScale")
        cbox = (length(e.get("width"), box, "x", (self.W, self.H)) or box[0], length(e.get("height"), box, "y", (self.W, self.H)) or box[1])
        window = self.window(e, tm)
        self.masks(e, g, tm, window, box, where)
        self.animations(e, g, NODE_PROP_MAP, tm, window, where, box, length_props=("x", "y", "anchorX", "anchorY"))
        self.convert_children(e, g, tm, cbox, prefix, ctx)
        return [g]

    n_sequence = n_group

    def overrides_for(self, e, ctx):
        out = {}
        eid = e.get("id")
        for o in ctx.get("overrides", []) + self.variant_overrides:
            if o.get("target") == eid or o.get("target", "").endswith("/" + (eid or "")):
                out[o.get("property")] = o.get("value")
        for b in self.binds:
            if b.get("target") == eid:
                v = self.param_vals.get(b.get("param"))
                if b.get("map"):
                    mp = dict(p.split("=", 1) for p in b.get("map").split(";") if "=" in p)
                    v = mp.get(v, v)
                out[b.get("property")] = v
        return out

    def n_layer(self, e, tm, box, prefix, ctx, pos, where):
        a = self.ids.get(e.get("asset"))
        if a is None:
            return []
        if e.get("matte"):
            src = self.ids.get(e.get("matte"))
            geo = src is not None and src.tag == "shape" and src.get("shape") in ("rect", "ellipse", "rounded-rect") \
                and not any(c.tag in ("animate", "expression") for c in src) and e.get("matteMode", "alpha") in ("alpha", "alpha-inverted")
            if not geo:
                self.report.add("dropped", where, "track-matted layer (shown only through %s in 1.1)" % e.get("matte"))
                return []
            inner = dict(e.attrib)
            inner.pop("matte")
            clone = ET.Element(e.tag, inner)
            clone.extend(list(e))
            self.parent.update({c: clone for c in clone})
            els = self.n_layer(clone, tm, box, prefix, ctx, pos, where)
            if not els:
                return []
            gs, ge = self.window(e, tm)
            g = ET.Element("group", {"id": (prefix + e.get("id")) + "--matte"})
            if gs:
                g.set("start", fmt(round(gs, 6)))
            if ge is not None:
                g.set("end", fmt(round(ge, 6)))
            m = {"type": src.get("shape"), "x": fmt(length(src.get("x", "0"), box, "x", (self.W, self.H))),
                 "y": fmt(length(src.get("y", "0"), box, "y", (self.W, self.H))),
                 "width": fmt(length(src.get("width"), box, "x", (self.W, self.H))),
                 "height": fmt(length(src.get("height"), box, "y", (self.W, self.H)))}
            if src.get("radius"):
                m["radius"] = src.get("radius")
            if e.get("matteMode") == "alpha-inverted":
                m["invert"] = "true"
            ET.SubElement(g, "mask", m)
            for el in els:
                g.append(el)
            self.report.add("converted", where, "alpha matte %s as a %s mask" % (e.get("matte"), src.get("shape")))
            return [g]
        if any(c.tag == "transformConstraint" and c.get("type") == "track" for c in e):
            self.report.add("dropped", where, "layer pinned to tracking data")
            return []
        attrs = self.node_attrs(e, tm, box, where, pos, id_prefix=prefix)
        if attrs is None:
            return []
        window = self.window(e, tm)
        ov = self.overrides_for(e, ctx)
        if a.tag == "text":
            aid = self.text_asset(a.get("id"), ov.get("text"), ctx.get("row"), ctx.get("var"))
            el = ET.Element("layer", {**attrs, "asset": aid})
            self.text_animators(e, el, tm, where, attrs)
        elif a.tag == "vector":
            fill = self.color(ov.get("fill") or a.get("fill", "#FFFFFFFF"), where)
            stroke = self.color(ov.get("stroke") or a.get("stroke"), where) if (ov.get("stroke") or a.get("stroke")) else None
            key = ("vector", a.get("id"), fill, stroke)
            if key not in self.used_assets:
                first = not any(isinstance(k, tuple) and k[:2] == ("vector", a.get("id")) for k in self.used_assets)
                vid = a.get("id") if first else self.uid(a.get("id"))
                self.vector_asset(vid, a.get("shape"), num(a.get("width")), num(a.get("height")), a, fill, stroke,
                                  a.get("strokeWidth") if num(a.get("strokeWidth", "0")) > 0 else None, where)
                if a.get("shape") == "svg":
                    self.report.add("placeholder", where, "svg vector as a rectangle")
                self.used_assets[key] = vid
            el = ET.Element("layer", {**attrs, "asset": self.used_assets[key]})
        elif self.image_src(a):
            el, vscale = self.image_layer(e, a, attrs, box, where)
            b = self.blend(e, where)
            if b != "normal":
                el.set("blend", b)
            self.masks(e, el, tm, window, box, where)
            self.animations(e, el, NODE_PROP_MAP, tm, window, where, box, length_props=("x", "y", "anchorX", "anchorY"),
                            value_scale=vscale)
            for k in ("stabilize", "frameBlend", "freezeAt", "flipX", "flipY"):
                if e.get(k) and e.get(k) not in ("none", "false"):
                    self.report.add("dropped", where, "@%s" % k)
            return [self.wrap_effects(el, e, where)]
        elif a.tag == "generator" and a.get("kind") in ("solid", "gradient"):
            return self.generator_rect(e, a, attrs, tm, box, where, window)
        else:
            if a.tag == "generator":
                bw = length(e.get("boxWidth"), box, "x", (self.W, self.H)) if e.get("boxWidth") else num(a.get("width"))
                bh = length(e.get("boxHeight"), box, "y", (self.W, self.H)) if e.get("boxHeight") else num(a.get("height"))
                if bw >= self.W - 0.5 and bh >= self.H - 0.5:  # full-frame overlay (fog, rain, grain, rays)
                    self.report.add("dropped", where, "full-frame %s generator overlay" % a.get("kind"))
                    return []
            return self.placeholder(e, a, attrs, tm, box, where, window)
        b = self.blend(e, where)
        if b != "normal":
            el.set("blend", b)
        for k in ("clipIn", "clipOut", "loop", "reverse", "speed"):
            if e.get(k) and a.tag in ("image", "video", "audio"):
                el.set(k, e.get(k))
        self.masks(e, el, tm, window, box, where)
        self.animations(e, el, NODE_PROP_MAP, tm, window, where, box, length_props=("x", "y", "anchorX", "anchorY"))
        for body in e:
            if body.tag in ("rigidBody", "softBody"):
                el.append(self.body(body, where))
            elif body.tag == "deform":
                d = self.deform(body, tm, window, where)
                if d is not None:
                    el.append(d)
            elif body.tag in ("transformConstraint", "timeRemap", "textPath", "shapeModifier"):
                self.report.add("dropped", where, "<%s>" % body.tag)
        for k in ("fit", "stabilize", "frameBlend", "freezeAt", "cropLeft", "flipX", "flipY", "volume", "mute", "audioBus"):
            if e.get(k) and e.get(k) not in ("none", "false"):
                self.report.add("dropped", where, "@%s" % k)
        return [self.wrap_effects(el, e, where)]

    def text_animators(self, e, el, tm, where, attrs):
        has_opacity = any(c.tag in ("animate", "expression") and c.get("property") == "opacity" for c in e)
        for ta in e:
            if ta.tag != "textAnimator":
                continue
            p = ta.get("preset")
            if not p:
                self.report.add("dropped", where, "custom text animator %r" % (ta.get("name") or ""))
                continue
            s = self.tmap(tm, num(ta.get("presetStart"), 0)) if ta.get("presetStart") else self.window(e, tm)[0]
            d = num(ta.get("presetDuration"), 0.8) * tm[0]
            if p in SLIDE_PRESETS or p in FADE_PRESETS or p == "fade-out":
                if has_opacity:
                    self.report.add("dropped", where, "text animator %s (layer already animates opacity)" % p)
                    continue
                a = ET.SubElement(el, "animate", {"property": "opacity"})
                v0, v1 = ("1", "0") if p == "fade-out" else ("0", "1")
                ET.SubElement(a, "key", {"time": fmt(round(s, 5)), "value": v0, "interpolation": "ease-out"})
                ET.SubElement(a, "key", {"time": fmt(round(s + d, 5)), "value": v1})
                has_opacity = True
                if p in SLIDE_PRESETS:
                    dx, dy = SLIDE_PRESETS[p]
                    for prop, off, base in (("position.x", dx, num(attrs.get("x", "0"))), ("position.y", dy, num(attrs.get("y", "0")))):
                        if off:
                            ak = ET.SubElement(el, "animate", {"property": prop})
                            ET.SubElement(ak, "key", {"time": fmt(round(s, 5)), "value": fmt(base + off),
                                                      "interpolation": "cubic-bezier", "bezier": BEZIER["cubic-out"]})
                            ET.SubElement(ak, "key", {"time": fmt(round(s + d, 5)), "value": fmt(base)})
                self.report.add("approximated", where, "text animator %s as a %s" % (p, "slide + fade" if p in SLIDE_PRESETS else "fade"))
            else:
                self.report.add("dropped", where, "text animator preset %s" % p)

    def image_src(self, a):
        """The file behind an image-like asset, if it exists (images, and
        generated images whose cache was resolved)."""
        src = a.get("src") if a.tag == "image" else (a.get("cache") if a.tag == "generated" and a.get("kind") == "image" else None)
        if src and os.path.exists(os.path.join(os.path.dirname(os.path.abspath(self.src_path)), src)):
            return src
        return None

    def image_layer(self, e, a, attrs, box, where):
        src = self.image_src(a)
        if a.get("id") not in self.image_assets:
            self.image_assets.add(a.get("id"))
            self.assets_out.append(ET.Element("image", {"id": a.get("id"), "src": src, "width": a.get("width"),
                                                        "height": a.get("height"), "colorSpace": "srgb"}))
            if a.tag == "generated":
                self.report.add("converted", a.get("id"), "generated image used from its cache %s" % src)
        el = ET.Element("layer", {**attrs, "asset": a.get("id")})
        vscale = None
        fit = e.get("fit", "none")
        if fit != "none" and (e.get("boxWidth") or e.get("boxHeight")):
            w, h = num(a.get("width")), num(a.get("height"))
            bw = length(e.get("boxWidth"), box, "x", (self.W, self.H)) if e.get("boxWidth") else w
            bh = length(e.get("boxHeight"), box, "y", (self.W, self.H)) if e.get("boxHeight") else h
            if fit == "fill":
                fx, fy = bw / w, bh / h
            else:
                f = (max if fit in ("cover", "contain-blur") else min)(bw / w, bh / h)
                if fit == "scale-down":
                    f = min(1.0, f)
                fx = fy = f
            sx, sy = num(e.get("scaleX", "1"), 1) * fx, num(e.get("scaleY", "1"), 1) * fy
            el.set("scaleX", fmt(round(sx, 6)))
            el.set("scaleY", fmt(round(sy, 6)))
            focus = (num(e.get("focusX", "0.5"), 0.5), num(e.get("focusY", "0.5"), 0.5))
            el.set("x", fmt(round(num(attrs.get("x", "0")) + (bw - w * sx) * focus[0], 3)))
            el.set("y", fmt(round(num(attrs.get("y", "0")) + (bh - h * sy) * focus[1], 3)))
            if (w * sx > bw + 0.5 or h * sy > bh + 0.5) and (bw < self.W - 0.5 or bh < self.H - 0.5):
                ET.SubElement(el, "mask", {"type": "rect", "x": fmt(round((w * sx - bw) * focus[0] / sx, 3)),
                                           "y": fmt(round((h * sy - bh) * focus[1] / sy, 3)),
                                           "width": fmt(round(bw / sx, 3)), "height": fmt(round(bh / sy, 3))})
            vscale = {"scaleX": lambda v, k=fx: v * k, "scaleY": lambda v, k=fy: v * k}
            self.report.add("converted", where, "fit=%s into %gx%g as scale %.4g" % (fit, bw, bh, fx))
        return el, vscale

    def generator_rect(self, e, a, attrs, tm, box, where, window):
        """Solid and gradient generators as a flat rectangle (gradients: the
        mean of the two paints)."""
        c1 = parse_color(self.color(a.get("paint", "#FFFFFFFF"), where))
        if a.get("kind") == "gradient":
            c2 = parse_color(self.color(a.get("paint2", "#000000FF"), where))
            c1 = [(x + y) / 2 for x, y in zip(c1, c2)]
            self.report.add("approximated", where, "gradient generator as its mean colour")
        w = length(e.get("boxWidth"), box, "x", (self.W, self.H)) if e.get("boxWidth") else num(a.get("width"))
        h = length(e.get("boxHeight"), box, "y", (self.W, self.H)) if e.get("boxHeight") else num(a.get("height"))
        el = ET.Element("shape", {**attrs, "shape": "rect", "width": fmt(round(w, 3)), "height": fmt(round(h, 3)),
                                  "fill": hex_color(c1)})
        b = self.blend(e, where)
        if b != "normal":
            el.set("blend", b)
        self.animations(e, el, NODE_PROP_MAP, tm, window, where, box, length_props=("x", "y"))
        return [self.wrap_effects(el, e, where)]

    def placeholder(self, e, a, attrs, tm, box, where, window):
        """A labelled panel of the asset's size (charts: real bars)."""
        w = length(e.get("boxWidth"), box, "x", (self.W, self.H)) if e.get("boxWidth") else num(a.get("width"), 400)
        h = length(e.get("boxHeight"), box, "y", (self.W, self.H)) if e.get("boxHeight") else num(a.get("height"), 300)
        g = ET.Element("group", attrs)
        self.animations(e, g, NODE_PROP_MAP, tm, window, where, box, length_props=("x", "y", "anchorX", "anchorY"))
        self.masks(e, g, tm, window, box, where)
        gid = attrs["id"]
        if a.tag == "chart" and a.get("kind") in ("column", "bar"):
            self.chart_bars(g, a, gid, w, h, tm)
            self.report.add("approximated", where, "chart %s drawn as bars" % a.get("id"))
            return [self.wrap_effects(g, e, where)]
        if a.tag == "chart" and a.get("kind") == "counter":
            series = [s for s in a if s.tag == "series"]
            final = num(series[0].get("values").split()[-1]) if series else 0
            txt = self.plain_text(gid + "-t", "{:,.0f}".format(final), w, h, 96, "#F4F6FBFF", "center")
            ET.SubElement(g, "layer", {"id": gid + "-l", "asset": txt})
            self.report.add("approximated", where, "counter shown at its final value")
            return [g]
        if a.tag == "formula":
            tex = a.get("tex", "")
            plain = re.sub(r"\\mathrm\{([^}]*)\}", r"\1", tex).replace("\\ ", " ").replace("\\,", " ")
            plain = re.sub(r"\\([A-Za-z]+)", r"\1", plain).replace("{", "").replace("}", "")
            txt = self.plain_text(gid + "-t", plain, w, h, num(a.get("size"), 48), self.color(a.get("color", "#FFFFFFFF"), where), "start")
            ET.SubElement(g, "layer", {"id": gid + "-l", "asset": txt})
            self.report.add("approximated", where, "formula as plain text")
            return [self.wrap_effects(g, e, where)]
        label = {"video": "VIDEO", "imageSequence": "IMAGE SEQUENCE", "lottie": "LOTTIE", "image": "IMAGE",
                 "code": "QR CODE" if a.get("kind") == "qr" else "BARCODE", "audiogram": "AUDIOGRAM",
                 "generator": a.get("kind", "generator").upper(), "chart": "CHART", "generated": "GENERATED",
                 "mesh": "MESH"}.get(a.tag, a.tag.upper())
        detail = os.path.basename(a.get("src", "")) or a.get("id")
        fx = [self.effect_elems[x].get("type") for x in (e.get("effects") or "").split() if x in self.effect_elems]
        if fx:
            detail += " · " + ", ".join(fx)
        panel = ET.SubElement(g, "shape", {"id": gid + "-panel", "shape": "rect", "width": fmt(round(w, 2)),
                                          "height": fmt(round(h, 2)), "fill": "#1B2338E6", "stroke": "#3DD6D0AA",
                                          "strokeWidth": "3"})
        if a.tag == "audiogram":
            bars = int(num(a.get("bars"), 32))
            bw = w / bars
            for i in range(bars):
                bh = h * (0.2 + 0.7 * abs(math.sin(i * 0.7) * math.cos(i * 0.23)))
                ET.SubElement(g, "shape", {"id": "%s-b%d" % (gid, i), "shape": "rect", "width": fmt(round(bw * 0.6, 2)),
                                           "height": fmt(round(bh, 2)), "x": fmt(round(i * bw + bw * 0.2, 2)),
                                           "y": fmt(round((h - bh) / 2, 2)), "fill": "#FF8A3DCC"})
        size = max(18, min(48, h / 5))
        txt = self.plain_text(gid + "-t", "%s · %s" % (label, detail), w, h, size, "#9FB0CCFF", "center")
        ET.SubElement(g, "layer", {"id": gid + "-l", "asset": txt})
        self.report.add("placeholder", where, "%s %s" % (a.tag, a.get("id")))
        return [self.wrap_effects(g, e, where)]

    def plain_text(self, tid, text, w, h, size, color, align):
        el = ET.Element("text", {"id": tid, "text": text, "width": fmt(max(1, round(w))), "height": fmt(max(1, round(h))),
                                 "size": fmt(round(size, 2)), "color": color, "align": align, "verticalAlign": "middle"})
        self.assets_out.append(el)
        return tid

    def chart_bars(self, g, a, gid, w, h, tm):
        series = [s for s in a if s.tag == "series"]
        labels = [l.strip() for l in (a.get("labels") or "").split(",")]
        vals = [[num(v) for v in s.get("values").split()] for s in series]
        n = max(len(v) for v in vals) if vals else 0
        vmax = max((max(v) for v in vals if v), default=1) or 1
        prog = next((c for c in a if c.tag == "animate" and c.get("property") == "progress"), None)
        pk = [(self.tmap(tm, num(k.get("time"))), num(k.get("value"))) for k in prog] if prog is not None else []
        group_w = w / max(1, n)
        bw = group_w * 0.8 / max(1, len(series))
        chart_h = h - 60
        for si, (s, vs) in enumerate(zip(series, vals)):
            col = self.color(s.get("color", "#3DD6D0FF"), gid)
            for i, v in enumerate(vs):
                bh = max(1, chart_h * v / vmax)
                bid = "%s-s%d-%d" % (gid, si, i)
                bar = ET.SubElement(g, "shape", {"id": bid, "shape": "rect", "width": fmt(round(bw, 2)), "height": fmt(round(bh, 2)),
                                                 "x": fmt(round(i * group_w + group_w * 0.1 + si * bw, 2)), "y": fmt(round(chart_h, 2)),
                                                 "anchorY": fmt(round(bh, 2)), "fill": col})
                if pk:
                    an = ET.SubElement(bar, "animate", {"property": "scale.y"})
                    for j, (t, p) in enumerate(pk):
                        attrs = {"time": fmt(round(t, 5)), "value": fmt(max(0.0001, p))}
                        if j == 0:
                            attrs.update({"interpolation": "cubic-bezier", "bezier": BEZIER["cubic-out"]})
                        ET.SubElement(an, "key", attrs)
        for i, lab in enumerate(labels[:n]):
            tid = self.plain_text("%s-lab%d" % (gid, i), lab, group_w, 40, 24, "#8A93A6FF", "center")
            ET.SubElement(g, "layer", {"id": "%s-labl%d" % (gid, i), "asset": tid, "x": fmt(round(i * group_w, 2)),
                                       "y": fmt(round(chart_h + 12, 2))})

    def n_shape(self, e, tm, box, prefix, ctx, pos, where):
        attrs = self.node_attrs(e, tm, box, where, pos, id_prefix=prefix)
        if attrs is None:
            return []
        ov = self.overrides_for(e, ctx)
        window = self.window(e, tm)
        kind = e.get("shape")
        w = length(e.get("width"), box, "x", (self.W, self.H))
        h = length(e.get("height"), box, "y", (self.W, self.H))
        fill = self.color(ov.get("fill") or e.get("fill", "#FFFFFFFF"), where)
        stroke = self.color(ov.get("stroke") or e.get("stroke"), where) if (ov.get("stroke") or e.get("stroke")) else None
        sw = e.get("strokeWidth") if num(e.get("strokeWidth", "0")) > 0 else None
        for k in ("trimStart", "trimEnd", "dash", "strokeCap", "strokeJoin", "cornerRadii", "strokePosition"):
            if e.get(k) is not None and k != "cornerRadii":
                self.report.add("dropped", where, "@%s" % k)
        if kind in ("rect", "ellipse") and not e.get("cornerRadii"):
            attrs.update({"shape": kind, "width": fmt(round(w, 4)), "height": fmt(round(h, 4)), "fill": fill})
            if stroke:
                attrs["stroke"] = stroke
            if sw:
                attrs["strokeWidth"] = sw
            el = ET.Element("shape", attrs)
            color_props = ("fill", "stroke")
        else:
            vid = self.vector_asset(self.uid((e.get("id") or "shape") + "-vec"), kind, w, h, e, fill, stroke, sw, where)
            el = ET.Element("layer", {**attrs, "asset": vid})
            color_props = ()
            if kind != "path":
                self.report.add("converted", where, "%s shape as a vector path" % kind)
        b = self.blend(e, where)
        if b != "normal":
            el.set("blend", b)
        self.masks(e, el, tm, window, box, where)
        self.animations(e, el, NODE_PROP_MAP, tm, window, where, box, color_props=color_props,
                        length_props=("x", "y", "anchorX", "anchorY"))
        for c in e:
            if c.tag in ("rigidBody", "softBody"):
                el.append(self.body(c, where))
            elif c.tag == "deform":
                d = self.deform(c, tm, window, where)
                if d is not None:
                    el.append(d)
            elif c.tag == "shapeModifier":
                if c.get("type") == "repeater":
                    return self.repeater(el, c, e, where)
                self.report.add("dropped", where, "shape modifier %s" % c.get("type"))
            elif c.tag == "transformConstraint":
                self.report.add("dropped", where, "transform constraint %s" % c.get("type"))
        return [self.wrap_effects(el, e, where)]

    def repeater(self, el, mod, e, where):
        copies = int(max(num(k.get("value")) for k in mod.iter("key")) if any(True for _ in mod.iter("key")) else num(mod.get("copies"), 3))
        rot, so, eo = num(mod.get("rotation", "0")), num(mod.get("startOpacity", "1"), 1), num(mod.get("endOpacity", "1"), 1)
        out = []
        for i in range(copies):
            c = copy.deepcopy(el)
            c.set("id", "%s-r%d" % (el.get("id"), i))
            c.set("rotation", fmt(num(el.get("rotation", "0")) + rot * i))
            op = so + (eo - so) * (i / max(1, copies - 1))
            c.set("opacity", fmt(round(num(el.get("opacity", "1"), 1) * op, 4)))
            out.append(c)
        self.report.add("converted", where, "repeater expanded to %d static copies" % copies)
        return out

    def n_particleEmitter(self, e, tm, box, prefix, ctx, pos, where):
        attrs = self.node_attrs(e, tm, box, where, pos, id_prefix=prefix)
        if attrs is None:
            return []
        window = self.window(e, tm)
        for k in EMITTER_ATTRS_10:
            v = e.get(k)
            if v is None:
                continue
            if k in ("color", "colorEnd"):
                v = self.color(v, where)
            elif k == "preset" and v not in PRESETS_10:
                v2 = PRESET_NEAR.get(v)
                self.report.add("approximated", where, "preset %s as %s" % (v, v2))
                v = v2
                if v is None:
                    continue
            elif k == "shape" and v not in ("disc", "square"):
                self.report.add("approximated", where, "particle shape %s as disc" % v)
                v = "disc"
            elif k == "blend":
                v = self.blend(e, where)
            elif k in ("emitterWidth", "emitterHeight"):
                v = fmt(length(v, box, "x" if k == "emitterWidth" else "y", (self.W, self.H)))
            attrs[k] = v
        if e.get("emitterShape") == "point":
            attrs["emitterWidth"] = attrs["emitterHeight"] = "0"
        elif e.get("emitterShape") == "line":
            attrs["emitterHeight"] = "0"
        el = ET.Element("particleEmitter", attrs)
        self.animations(e, el, {**NODE_PROP_MAP, **{p: p for p in PARTICLE_PROPS}}, tm, window, where, box,
                        color_props=PARTICLE_COLOR_PROPS, length_props=("x", "y"))
        bursts = [b for b in e if b.tag == "burst"]
        if bursts and not any(a.tag == "animate" and a.get("property") == "rate" for a in el):
            base = num(e.get("rate", "10"), 10)
            an = ET.SubElement(el, "animate", {"property": "rate"})
            keys = [(window[0], base)]
            dt = 1.0 / self.fps
            for b in bursts:
                for r in range(int(num(b.get("repeat", "0"))) + 1):
                    t = self.tmap(tm, num(b.get("time"))) + r * num(b.get("interval", "1"), 1)
                    keys += [(t - 1e-3, base), (t, num(b.get("count")) / dt), (t + dt, base)]
            for t, v in sorted(keys):
                ET.SubElement(an, "key", {"time": fmt(round(t, 5)), "value": fmt(round(v, 3)), "interpolation": "step"})
            self.report.add("converted", where, "%d burst(s) as one-frame rate spikes" % len(bursts))
        for k in ("drag", "turbulence", "trail", "collide", "forceFields", "preroll", "opacityEnd", "sprite",
                  "angularVelocity", "orientToVelocity", "emitterShape"):
            if e.get(k) and k != "emitterShape":
                self.report.add("dropped", where, "@%s" % k)
        return [self.wrap_effects(el, e, where)]

    def n_camera(self, e, tm, box, prefix, ctx, pos, where):
        attrs = {"id": prefix + e.get("id"), "near": "1", "far": e.get("far", "20000")}
        self.report.add("converted", where, "camera fixed at the origin; its motion is baked into the 3D objects")
        for k in ("projection", "yaw", "pitch", "roll", "focusDistance"):
            if e.get(k) is not None:
                attrs[k] = e.get(k)
        sensor = num(e.get("sensorHeight", "24"), 24)
        fov_of = lambda f: 2 * math.degrees(math.atan(sensor / (2 * f)))
        if e.get("focalLength"):
            attrs["fov"] = fmt(round(fov_of(num(e.get("focalLength"))), 4))
        elif e.get("fov"):
            attrs["fov"] = e.get("fov")
        if e.get("depthOfField") == "true":
            attrs["aperture"] = fmt(round(min(20.0, 16.0 / max(0.5, num(e.get("fStop", "2.8"), 2.8))), 3))
        el = ET.Element("camera", attrs)
        self.animations(e, el, {"fov": "fov", "focalLength": "fov", "roll": "roll"},
                        tm, self.window(e, tm), where, box, value_scale={"focalLength": fov_of})
        tgt = self.ids.get(e.get("target", ""))
        if tgt is not None:                              # look-at: aim the fixed camera at the target
            s, en = self.window(e, tm)
            en = en if en is not None else self.duration
            n = max(2, int((en - s) * self.args.sample_rate) + 1)
            yk, pk = [], []
            for i in range(n):
                t = s + (en - s) * i / (n - 1)
                cx, cy, cz = (self.value_at(e, k, t) for k in ("x", "y", "z"))
                tx, ty, tz = (self.value_at(tgt, k, t) for k in ("x", "y", "z"))
                dz = max(1e-6, cz - tz)
                yk.append((t, math.degrees(math.atan2(tx - cx, dz))))
                pk.append((t, math.degrees(math.atan2(cy - ty, math.hypot(dz, tx - cx)))))
            for prop, vals in (("yaw", yk), ("pitch", pk)):
                an = ET.SubElement(el, "animate", {"property": prop})
                keep = [vals[0]] + [v for j, v in enumerate(vals[1:-1], 1)
                                    if abs(vals[j - 1][1] + vals[j + 1][1] - 2 * v[1]) > 1e-3] + [vals[-1]]
                for t, v in keep:
                    ET.SubElement(an, "key", {"time": fmt(round(t, 4)), "value": fmt(round(v, 3))})
            self.report.add("converted", where, "look-at %s baked into yaw/pitch keys" % tgt.get("id"))
        for k in ("start", "end", "focusTarget", "shutterAngle", "exposure", "lensDistortion"):
            if e.get(k):
                self.report.add("dropped", where, "camera @%s" % k)
        if any(c.tag == "shake" for c in e):
            self.report.add("dropped", where, "camera shake")
        return [el]

    def n_object3d(self, e, tm, box, prefix, ctx, pos, where):
        prim = e.get("primitive")
        if prim == "mesh":
            mesh = self.ids.get(e.get("mesh"))
            src = mesh.get("src", "") if mesh is not None else ""
            if not src.lower().endswith(".obj") or not os.path.exists(os.path.join(os.path.dirname(self.src_path), src)):
                self.report.add("placeholder", where, "mesh %s as a box" % os.path.basename(src))
                prim = "box"
        if prim not in PRIMITIVES_10:
            self.report.add("approximated", where, "primitive %s as %s" % (prim, PRIMITIVE_NEAR.get(prim, "box")))
            prim = PRIMITIVE_NEAR.get(prim, "box")
        attrs = {"id": prefix + e.get("id"), "primitive": prim}
        if e.get("material"):
            attrs["material"] = e.get("material")
        for k in ("x", "y", "z", "rotation", "rotationX", "rotationY", "scaleX", "scaleY", "scaleZ", "castShadow", "receiveShadow"):
            if e.get(k) is not None:
                attrs[k] = e.get(k)
        r = num(e.get("radius"), 0)
        if e.get("width") and prim in ("box", "plane"):
            k = num(e.get("width")) / 100.0
            attrs["scaleX"] = fmt(num(attrs.get("scaleX", "1"), 1) * k)
            attrs["scaleY"] = fmt(num(attrs.get("scaleY", "1"), 1) * num(e.get("height", e.get("width"))) / 100.0)
            if prim == "box":
                attrs["scaleZ"] = fmt(num(attrs.get("scaleZ", "1"), 1) * k)
        elif r:
            attrs["radius"] = fmt(r)
        if prim == "plane":
            self.report.add("dropped", where, "plane (1.0 draws planes as camera-facing sprites)")
            return []
        cam = self.camera_for(self.window(e, tm))
        el = ET.Element("object3D", attrs)
        window = self.window(e, tm)
        rot_map = {k: v for k, v in OBJ_PROP_MAP.items() if not k in ("x", "y", "z")}
        self.animations(e, el, rot_map, tm, window, where, box)
        if cam is not None:
            self.bake_camera(e, el, cam, window, ctx, where)
        # 1.0 objects have no time window: hide them outside it by scale
        s, en = window
        grp_s, grp_e = ctx.get("window", (0.0, None))
        s = max(s or 0.0, grp_s or 0.0)
        en = min(x for x in (en, grp_e, self.duration) if x is not None)
        if s > 0 or en < self.duration:
            if any(a.get("property", "").startswith("scale.") for a in el):
                self.report.add("approximated", where, "scale animation kept; object not hidden outside its window")
            else:
                for ax in ("x", "y", "z"):
                    base = attrs.get("scale" + ax.upper(), "1")
                    an = ET.SubElement(el, "animate", {"property": "scale." + ax})
                    keys = [(0.0, "0.0001")] if s > 0 else []
                    keys += [(s, base)]
                    if en < self.duration:
                        keys += [(en, "0.0001")]
                    for t, v in keys:
                        ET.SubElement(an, "key", {"time": fmt(round(t, 5)), "value": v, "interpolation": "step"})
                self.report.add("converted", where, "shown only during %g-%g s (scale keys)" % (s, en))
        return [el]

    def bake_camera(self, e, el, cam, window, ctx, where):
        """1.0 cameras look down +z from the origin; the 1.1 camera looks down
        -z from its own moving position. Keep the 1.0 camera at the origin and
        key each object's position relative to the 1.1 camera over time."""
        s = max(window[0] or 0.0, (ctx.get("window") or (0.0, None))[0] or 0.0)
        en = min(x for x in (window[1], (ctx.get("window") or (0, None))[1], self.duration) if x is not None)
        n = max(2, int((en - s) * self.args.sample_rate) + 1)
        tracks = {"position.x": [], "position.y": [], "position.z": []}
        for i in range(n):
            t = s + (en - s) * i / (n - 1)
            cx, cy, cz = (self.value_at(cam, k, t) for k in ("x", "y", "z"))
            ox, oy, oz = (self.value_at(e, k, t) for k in ("x", "y", "z"))
            tracks["position.x"].append((t, ox - cx))
            tracks["position.y"].append((t, oy - cy))                 # 1.0 camera space: +y up
            tracks["position.z"].append((t, cz - oz))
        for prop, vals in tracks.items():
            el.set(prop.split(".")[1], fmt(round(vals[0][1], 3)))
            span = max(v for _, v in vals) - min(v for _, v in vals)
            if span < 1e-6:
                continue
            keep = [vals[0]]
            for j in range(1, len(vals) - 1):
                (t0, v0), (t1, v1), (t2, v2) = keep[-1], vals[j], vals[j + 1]
                if abs(v0 + (v2 - v0) * (t1 - t0) / ((t2 - t0) or 1) - v1) > span * 1e-3:
                    keep.append(vals[j])
            keep.append(vals[-1])
            an = ET.SubElement(el, "animate", {"property": prop})
            for t, v in keep:
                ET.SubElement(an, "key", {"time": fmt(round(t, 4)), "value": fmt(round(v, 3))})
        self.report.add("converted", where, "position relative to camera %s baked into keys" % cam.get("id"))

    def n_instance(self, e, tm, box, prefix, ctx, pos, where):
        sym = self.ids.get(e.get("symbol"))
        if sym is None:
            return []
        attrs = self.node_attrs(e, tm, box, where, pos, id_prefix=prefix)
        if attrs is None:
            return []
        s, _ = self.window(e, tm)
        speed = num(e.get("speed", "1"), 1) or 1
        clip = num(e.get("clipIn", "0"))
        # local symbol time l maps to composition time s + (l - clipIn) / speed
        inner = ((tm[0] / speed), s - clip * tm[0] / speed)
        if sym.get("duration") and "end" not in attrs:
            attrs["end"] = fmt(round(s + num(sym.get("duration")) * tm[0] / speed, 5))
        g = ET.Element("group", attrs)
        window = self.window(e, tm)
        self.animations(e, g, NODE_PROP_MAP, tm, (window[0], window[1]), where, box, length_props=("x", "y"))
        ids = [x for x in (e.get("effects") or "").split() if self.effect_ok(x, where)]
        if ids:
            g.set("effects", " ".join(ids))
        sub = {**ctx, "overrides": [o for o in e if o.tag == "override"] + ctx.get("overrides", [])}
        sbox = (num(sym.get("width"), box[0]), num(sym.get("height"), box[1]))
        self.convert_children(sym, g, inner, sbox, attrs["id"] + "--", sub)
        self.report.add("converted", where, "instance of %s expanded" % sym.get("id"))
        return [g]

    def n_repeat(self, e, tm, box, prefix, ctx, pos, where):
        attrs = self.node_attrs(e, tm, box, where, pos, id_prefix=prefix)
        if attrs is None:
            return []
        g = ET.Element("group", attrs)
        window = self.window(e, tm)
        self.animations(e, g, NODE_PROP_MAP, tm, window, where, box, length_props=("x", "y"))
        rows = None
        if e.get("over"):
            src = self.ids.get(e.get("over"))
            if src is not None and src.tag == "data" and not src.get("src"):
                rows = json.loads(src.text or "[]")
            elif src is not None and src.tag == "param":
                rows = [r.strip() for r in (self.param_vals.get(src.get("id")) or "").split(",")]
        count = len(rows) if rows is not None else int(num(e.get("count"), 0))
        start = window[0]
        for i in range(count):
            delay = i * num(e.get("timeStep", "0"))
            cattrs = {"id": "%s--%d" % (attrs["id"], i)}
            x, y = i * num(e.get("offsetX", "0")), i * num(e.get("offsetY", "0"))
            if x:
                cattrs["x"] = fmt(x)
            if y:
                cattrs["y"] = fmt(y)
            if e.get("rotationStep"):
                cattrs["rotation"] = fmt(i * num(e.get("rotationStep")))
            sc = num(e.get("scaleStep", "1"), 1) ** i
            if sc != 1:
                cattrs["scaleX"] = cattrs["scaleY"] = fmt(sc)
            op = 1 - i * num(e.get("opacityStep", "0"))
            if op != 1:
                cattrs["opacity"] = fmt(max(0, op))
            cg = ET.SubElement(g, "group", cattrs)
            # children run on the repeat's clock (local 0 = repeat start), each copy delayed
            inner = (tm[0], start + delay) if e.get("over") else (tm[0], tm[1] + delay)
            sub = {**ctx, "row": rows[i] if rows is not None else None, "var": e.get("var", "item"), "repeat": True}
            self.convert_children(e, cg, inner, box, cattrs["id"] + "--", sub)
        self.report.add("converted", where, "repeat expanded to %d copies" % count)
        return [g]

    def n_include(self, e, tm, box, prefix, ctx, pos, where):
        attrs = self.node_attrs(e, tm, box, where, pos, id_prefix=prefix)
        g = ET.Element("group", attrs)
        text = next((o.get("value") for o in e if o.tag == "override" and o.get("property") == "text"), None)
        panel = ET.SubElement(g, "shape", {"id": attrs["id"] + "-panel", "shape": "rect", "width": "1100", "height": "90",
                                          "fill": "#161D30E6", "stroke": "#FF5A36FF", "strokeWidth": "3"})
        tid = self.plain_text(attrs["id"] + "-t", text or "include: %s" % e.get("src"), 1060, 90, 34, "#F4F6FBFF", "start")
        ET.SubElement(g, "layer", {"id": attrs["id"] + "-l", "asset": tid, "x": "24"})
        self.report.add("placeholder", where, "include %s#%s as a lower third" % (e.get("src"), e.get("symbol")))
        return [g]

    def n_adjustment(self, e, tm, box, prefix, ctx, pos, where):
        ids = [x for x in (e.get("effects") or "").split() if self.effect_ok(x, where)]
        s, en = self.window(e, tm)
        full = (not s) and (en is None or en >= self.duration) and not any(c.tag == "mask" for c in e) and not ctx.get("in_group")
        if ids and full:
            self.frame_effects += ids
            self.report.add("converted", where, "adjustment over the whole frame: %s as frame effects" % ", ".join(ids))
        elif ids:
            self.report.add("dropped", where, "timed or masked adjustment (%s)" % ", ".join(ids))
        return []

    def n_transition(self, e, tm, box, prefix, ctx, pos, where):
        return []                                        # applied to the chapter groups up front

    def n_skeleton(self, e, tm, box, prefix, ctx, pos, where):
        self.report.add("dropped", where, "skeleton, bones and IK")
        return []

    # --------------------------------------------------- bodies
    def body(self, b, where):
        ppm = num(self.doc.find("physics").get("pixelsPerMeter", "100"), 100) if self.doc.find("physics") is not None else 100
        if b.tag == "rigidBody":
            attrs = {}
            for k in ("type", "mass", "friction", "restitution", "linearDamping", "angularDamping", "angularVelocity"):
                if b.get(k) is not None:
                    attrs[k] = b.get(k)
            sh = b.get("shape", "box")
            attrs["shape"] = sh if sh in ("box", "circle") else "box"
            if sh not in ("box", "circle"):
                self.report.add("approximated", where, "collider %s as box" % sh)
            if b.get("radius"):
                attrs["radius"] = fmt(num(b.get("radius")) * ppm)
            if b.get("velocityX"):
                attrs["velocityX"] = fmt(num(b.get("velocityX")) * ppm)
            if b.get("velocityY"):
                attrs["velocityY"] = fmt(-num(b.get("velocityY")) * ppm)
            for k in ("activateAt", "bullet", "sensor", "collisionGroup", "fixedRotation"):
                if b.get(k):
                    self.report.add("dropped", where, "rigidBody @%s" % k)
            return ET.Element("rigidBody", attrs)
        attrs = {k: b.get(k) for k in ("mass", "stiffness", "damping", "pressure", "rows", "cols", "pin") if b.get(k)}
        if b.get("kind"):
            self.report.add("approximated", where, "soft body kind %s as the 1.0 spring grid" % b.get("kind"))
        return ET.Element("softBody", attrs)

    def deform(self, d, tm, window, where):
        out = ET.Element("deform")
        for m in d:
            if m.tag != "modifier":
                continue
            if m.get("type") not in ("bend", "twist", "wave", "squash", "stretch", "mesh-warp"):
                self.report.add("dropped", where, "%s modifier" % m.get("type"))
                continue
            mm = ET.SubElement(out, "modifier", {k: v for k, v in m.attrib.items() if k in ("type", "amount", "frequency", "phase", "axis", "rows", "cols")})
            self.animations(m, mm, {"amount": "amount", "frequency": "frequency", "phase": "phase"}, tm, window, where + "/modifier", (self.W, self.H))
        return out if len(out) else None

    # ------------------------------------------------ transitions
    def transitions(self):
        """Crossfades, slides or dips on the chapter groups (1.0 has no
        transitions). Returns {group id: (new start, new end, animates)}."""
        plan = {}
        for tr in self.doc.iter("transition"):
            f, t = self.ids.get(tr.get("from", "")), self.ids.get(tr.get("to", ""))
            if f is None or t is None:
                continue
            d = num(tr.get("duration", "0.5"), 0.5)
            cut = num(t.get("start", "0"))
            al = tr.get("alignment", "center")
            lo, hi = {"start": (cut, cut + d), "end": (cut - d, cut)}.get(al, (cut - d / 2, cut + d / 2))
            typ = tr.get("type")
            fp, tp = plan.setdefault(f.get("id"), {"anims": []}), plan.setdefault(t.get("id"), {"anims": []})
            if typ == "dip-to-color":
                mid = (lo + hi) / 2
                fp["anims"].append(("opacity", [(lo, 1), (mid, 0)]))
                tp["anims"].append(("opacity", [(mid, 0), (hi, 1)]))
                fp["end"], tp["start"] = mid, mid
                kind = "dip"
            elif typ in ("push", "slide", "whip-pan", "cover", "reveal"):
                dx = {"left": -self.W, "right": self.W}.get(tr.get("direction", "left"), 0)
                dy = {"up": -self.H, "down": self.H}.get(tr.get("direction", "left"), 0)
                if dx:
                    fp["anims"].append(("position.x", [(lo, 0), (hi, dx)]))
                    tp["anims"].append(("position.x", [(lo, -dx), (hi, 0)]))
                if dy:
                    fp["anims"].append(("position.y", [(lo, 0), (hi, dy)]))
                    tp["anims"].append(("position.y", [(lo, -dy), (hi, 0)]))
                fp["end"], tp["start"] = hi, lo
                kind = "slide"
            else:
                fp["anims"].append(("opacity", [(lo, 1), (hi, 0)]))
                tp["anims"].append(("opacity", [(lo, 0), (hi, 1)]))
                fp["end"], tp["start"] = hi, lo
                kind = "crossfade"
            self.report.add("approximated", tr.get("id"), "%s transition as a %s" % (typ, kind))
        return plan

    # --------------------------------------------------- captions
    def captions(self, comp):
        tracks = list(self.doc.iter("captionTrack"))
        if not tracks or self.args.no_captions:
            return
        tr = tracks[0]
        style = self.style_chain(tr.get("style"))
        u = self.W / 1920.0                              # caption geometry is laid out for 1920 and scaled
        size = num(style.get("size"), 40 * u) if style.get("size") else 40 * u
        bw, bh, bx, by = 1560 * u, 112 * u, 180 * u, self.H - 178 * u
        g = ET.SubElement(comp, "group", {"id": "captions", "z": "980"})
        for i, c in enumerate(x for x in tr if x.tag == "cue"):
            text = c.get("text") or " ".join(w.get("text") for w in c if w.tag == "word")
            s, e = num(c.get("start")), num(c.get("end"))
            cg = ET.SubElement(g, "group", {"id": "cue-%d" % i, "start": fmt(s), "end": fmt(e)})
            ET.SubElement(cg, "shape", {"id": "cue-%d-bg" % i, "shape": "rect", "width": fmt(round(bw, 2)), "height": fmt(round(bh, 2)),
                                       "x": fmt(round(bx, 2)), "y": fmt(round(by, 2)), "fill": "#0B0F1AD9"})
            tid = self.plain_text("cue-%d-t" % i, text, bw - 60 * u, bh, size, self.color(style.get("color", "#F4F6FBFF"), "captions"), "center")
            ET.SubElement(cg, "layer", {"id": "cue-%d-l" % i, "asset": tid, "x": fmt(round(bx + 30 * u, 2)), "y": fmt(round(by, 2))})
        self.report.add("converted", "captions", "%s burned in as %d text cues" % (tr.get("id"), len(g)))

    # --------------------------------------------------------- run
    def run(self):
        out = ET.Element("scene", {"version": "1.0"})
        proj = self.doc.find("project")
        pa = {k: proj.get(k) for k in ("width", "height", "fps", "duration", "seed", "linearLight", "antialias3d", "mode") if proj.get(k)}
        pa["background"] = self.color(proj.get("background", "#000000FF"), "project")
        ws = proj.get("workingColorSpace", "srgb")
        pa["workingColorSpace"] = ws if ws in ("srgb", "rec709", "display-p3", "rec2020") else "rec709"
        if ws != pa["workingColorSpace"]:
            self.report.add("approximated", "project", "working space %s as %s" % (ws, pa["workingColorSpace"]))
        k = self.args.scale
        if k != 1:
            pa["width"], pa["height"] = str(int(round(self.W * k))), str(int(round(self.H * k)))
            self.report.add("converted", "project", "rendered at %gx: %sx%s (composition wrapped in a scaled group)"
                            % (k, pa["width"], pa["height"]))
        ET.SubElement(out, "project", pa)
        o = next((x for x in self.doc.findall("output") if x.get("codec") in ("h264", "h265", "ffv1")), None)
        oa = {"path": self.args.output or "renders/animatic_1.0.mp4", "codec": o.get("codec") if o is not None else "h264"}
        if o is not None:
            for k in ("crf", "preset", "pixelFormat", "colorRange"):
                if o.get(k):
                    oa[k] = o.get(k)
            if o.get("colorSpace") in ("srgb", "rec709", "display-p3", "rec2020"):
                oa["colorSpace"] = o.get("colorSpace")
        ET.SubElement(out, "output", oa)
        n_out = len(self.doc.findall("output"))
        if n_out > 1:
            self.report.add("dropped", "outputs", "%d of %d outputs (1.0 writes one)" % (n_out - 1, n_out))
        assets = ET.SubElement(out, "assets")
        mats = self.materials()
        comp = ET.Element("composition")
        plan = self.transitions()
        for c in self.doc.find("composition"):
            if c.tag not in VS.NODE and c.tag not in ("transition", "skeleton"):
                continue
            ctx = {"window": self.window(c, (1.0, 0.0))}
            els = self.node(c, (1.0, 0.0), (self.W, self.H), "", ctx)
            p = plan.get(c.get("id"))
            if p and els:
                g = els[0]
                if "start" in p:
                    g.set("start", fmt(round(p["start"], 5)))
                if "end" in p:
                    g.set("end", fmt(round(p["end"], 5)))
                merged = {}                              # a chapter can be both "to" and "from": one track
                for prop, keys in p["anims"]:
                    merged.setdefault(prop, []).extend(keys)
                for prop, keys in merged.items():
                    if any(a.get("property") == prop for a in g.findall("animate")):
                        self.report.add("dropped", c.get("id"), "transition on %s (already animated)" % prop)
                        continue
                    an = ET.SubElement(g, "animate", {"property": prop, "defaultInterpolation": "ease-in-out"})
                    for t, v in sorted(dict(keys).items()):
                        ET.SubElement(an, "key", {"time": fmt(round(t, 5)), "value": fmt(v)})
            for el in els:
                comp.append(el)
        self.captions(comp)
        if self.args.scale != 1:                         # one scaled root group: positions, sizes, masks, text
            root = ET.Element("composition")
            g = ET.SubElement(root, "group", {"id": "scale-root", "scaleX": fmt(self.args.scale), "scaleY": fmt(self.args.scale)})
            for c in list(comp):
                if c.tag in ("camera", "object3D"):
                    root.append(c)
                else:
                    g.append(c)
            comp = root
        lights, effects, phys, audio = self.lights(), self.effects(), self.physics(), self.audio()
        if self.args.scale != 1 and effects is not None:
            for ef in effects:
                for att in ("radius", "offsetX", "offsetY"):
                    if ef.get(att):
                        ef.set(att, fmt(round(num(ef.get(att)) * self.args.scale, 4)))
                for an in ef.findall("animate"):
                    if an.get("property") in ("radius", "offsetX", "offsetY"):
                        for key in an:
                            key.set("value", fmt(round(num(key.get("value")) * self.args.scale, 4)))
        for el in self.assets_out:
            assets.append(el)
        if not len(assets):
            out.remove(assets)
        for el in (mats, comp, lights, effects, phys, audio):
            if el is not None:
                out.append(el)
        return out

    def materials(self):
        mats = list(self.doc.iter("material"))
        if not mats:
            return None
        el = ET.Element("materials")
        for m in mats:
            a = {"id": m.get("id")}
            for k in ("baseColor", "metallic", "roughness", "emissive"):
                if m.get(k):
                    a[k] = self.color(m.get(k), m.get("id")) if k in ("baseColor", "emissive") else m.get(k)
            ET.SubElement(el, "material", a)
            extra = [k for k in m.attrib if k not in ("id", "baseColor", "metallic", "roughness", "emissive")]
            if extra:
                self.report.add("dropped", m.get("id"), "material " + ", ".join(extra))
        return el

    def lights(self):
        """Lights move into the baked camera frame (see bake_camera): positions
        relative to the 3D camera at the middle of its window. Spot aims do
        not survive the axis flip, so spots become points; the 1.0 falloff is
        (1 - d/range)^falloff, so ranges are widened to reach the set."""
        ls = list(self.doc.iter("light"))
        if not ls:
            return None
        cams = [c for c in self.doc.iter("camera") if c.get("active", "true") != "false"]
        cam = cams[0] if cams else None
        if cam is not None:
            cs, ce = self.window(cam, (1.0, 0.0))
            tm = ((cs or 0.0) + (ce if ce is not None else self.duration)) / 2
            C = [self.value_at(cam, k, tm) for k in ("x", "y", "z")]
        peak = max((num(l.get("intensity", "1"), 1) for l in ls if l.get("type") not in ("ambient", "dome")), default=1) or 1
        k = 1.4 / peak
        el = ET.Element("lights")
        for l in ls:
            t = LIGHT_TYPES_NEAR.get(l.get("type"), "point")
            if t == "spot" and cam is not None:
                t = "point"
            if t == "ambient":
                inten = 0.35
            else:
                inten = num(l.get("intensity", "1"), 1) * k
            a = {"id": l.get("id"), "type": t, "intensity": fmt(round(inten, 4))}
            if l.get("color"):
                a["color"] = self.color(l.get("color"), l.get("id"))
            elif l.get("colorTemperature"):
                a["color"] = hex_color(kelvin_rgb(num(l.get("colorTemperature"))))
            if t != "ambient":
                x, y, z = (num(l.get(q, "0")) for q in ("x", "y", "z"))
                if cam is not None:
                    x, y, z = x - C[0], y - C[1], C[2] - z
                a.update({"x": fmt(round(x, 2)), "y": fmt(round(y, 2)), "z": fmt(round(z, 2)),
                          "range": fmt(max(4000.0, num(l.get("range"), 0))), "falloff": "1"})
                if t in ("directional",):
                    for q in ("yaw", "pitch"):
                        if l.get(q):
                            a[q] = l.get(q)
                if l.get("castShadow") == "true" and t == "directional":
                    a["castShadow"] = "true"
            if t != l.get("type"):
                self.report.add("approximated", l.get("id"), "%s light as %s" % (l.get("type"), t))
            ET.SubElement(el, "light", a)
        self.report.add("converted", "lights", "moved into the baked camera frame; point intensities scaled to peak 1.4, ambient 0.35")
        return el

    def effects(self):
        ids = list(dict.fromkeys(self.frame_effects))
        used = [e for e in self.doc.iter("effect") if e.get("id") in self.effects_used and e.get("id") not in ids]
        if not used and not ids:
            return None
        el = ET.Element("effects")
        for e in used + [self.effect_elems[i] for i in ids]:
            a = {k: v for k, v in e.attrib.items() if k in EFFECT_ATTRS_10}
            if a.get("color"):
                a["color"] = self.color(a["color"], e.get("id"))
            ee = ET.SubElement(el, "effect", a)
            self.animations(e, ee, {p: p for p in EFFECT_PROPS_10}, (1.0, 0.0), (0.0, None), e.get("id"), (self.W, self.H),
                            color_props=("color",))
            extra = [k for k in e.attrib if k not in EFFECT_ATTRS_10]
            if extra:
                self.report.add("dropped", e.get("id"), "effect " + ", ".join(extra))
        return el

    def physics(self):
        p = self.doc.find("physics")
        if p is None:
            return None
        ppm = num(p.get("pixelsPerMeter", "100"), 100)
        el = ET.Element("physics", {"fixedStep": p.get("fixedStep", "0.008333333333333333"),
                                    "gravityX": fmt(num(p.get("gravityX", "0")) * ppm),
                                    "gravityY": fmt(-num(p.get("gravityY", "-9.80665"), -9.80665) * ppm)})
        if p.get("start") or p.get("bounds"):
            self.report.add("dropped", "physics", "@start/@bounds: 1.0 simulates from t=0")
        for f in p:
            if f.tag == "forceField":
                t = {"directional": "directional", "wind": "directional", "radial": "radial", "vortex": "vortex"}.get(f.get("type"))
                if t is None or f.get("affects") == "particles":
                    self.report.add("dropped", f.get("id"), "%s field%s" % (f.get("type"), " (particles only)" if f.get("affects") == "particles" else ""))
                    continue
                a = {"id": f.get("id"), "type": t}
                for k in ("x", "y", "forceX", "forceY", "strength", "falloff"):
                    if f.get(k):
                        a[k] = f.get(k)
                fe = ET.SubElement(el, "forceField", a)
                self.animations(f, fe, {k: k for k in ("x", "y", "forceX", "forceY", "strength")}, (1.0, 0.0), (0.0, None), f.get("id"), (self.W, self.H))
            elif f.tag == "constraint":
                t = {"spring": "spring", "distance": "distance", "pin": "pin", "rope": "distance"}.get(f.get("type"))
                if t is None:
                    self.report.add("dropped", f.get("id"), "%s constraint" % f.get("type"))
                    continue
                a = {"id": f.get("id"), "type": t, "a": f.get("a")}
                if f.get("b") and t != "pin":
                    a["b"] = f.get("b")
                for k in ("x", "y", "stiffness", "damping"):
                    if f.get(k):
                        a[k] = f.get(k)
                if f.get("restLength"):
                    a["restLength"] = fmt(num(f.get("restLength")) * ppm)
                ET.SubElement(el, "constraint", a)
                if f.get("type") == "rope" or f.get("breakForce"):
                    self.report.add("approximated", f.get("id"), "rope as distance / breakForce dropped")
        return el

    def audio(self):
        mix = self.doc.find("audioMix")
        if mix is None:
            return None
        base = os.path.dirname(os.path.abspath(self.src_path))
        el = ET.Element("audioMix", {k: mix.get(k) for k in ("sampleRate", "channels") if mix.get(k)})
        for t in mix.iter("audioTrack"):
            a = self.ids.get(t.get("asset"))
            src = a.get("src") or a.get("cache") if a is not None else None
            if not src or not os.path.exists(os.path.join(base, src)):
                self.report.add("dropped", t.get("id"), "audio track (file %s missing)" % src)
                continue
            aid = "aud-" + t.get("id")
            self.assets_out.append(ET.Element("audio", {"id": aid, "src": src}))
            s = num(t.get("start", "0")) + (self.markers.get(t.get("startMarker"), 0.0) if t.get("startMarker") else 0.0)
            at = {"id": t.get("id"), "asset": aid, "start": fmt(s)}
            for k in ("clipIn", "clipOut", "loop", "volume", "pan", "fadeIn", "fadeOut", "speed", "reverse"):
                if t.get(k):
                    at[k] = t.get(k)
            ET.SubElement(el, "audioTrack", at)
        return el if len(el) else None


def eval_fps(s):
    a, _, b = s.partition("/")
    return int(a) / int(b or 1)


def main(argv=None):
    ap = argparse.ArgumentParser(description="Derive a scene-format 1.0 animatic from a 1.1 scene.")
    ap.add_argument("src")
    ap.add_argument("dst")
    ap.add_argument("--variant", help="parameter variant to apply (default: the first output's)")
    ap.add_argument("--output", help="video path written by the 1.0 scene (default renders/animatic_1.0.mp4)")
    ap.add_argument("--report", help="write the conversion report here (default DST + .report.md)")
    ap.add_argument("--no-captions", action="store_true", help="do not burn in the caption track")
    ap.add_argument("--sample-rate", type=float, default=10.0, help="keys per second for sampled expressions")
    ap.add_argument("--scale", type=float, default=1.0,
                    help="render size factor, e.g. 0.5 for a 1080p animatic of a 4K scene (the engine's --resolution only crops)")
    args = ap.parse_args(argv)
    conv = Converter(args.src, args)
    out = conv.run()
    ET.indent(out, "  ")
    body = ET.tostring(out, encoding="unicode")
    header = ('<?xml version="1.0" encoding="UTF-8"?>\n<!-- 1.0 animatic of %s, generated by tools/downlevel-1.0.py.\n'
              '     Structure, timing and words of the 1.1 scene; see the report for every approximation. -->\n'
              % os.path.basename(args.src))
    with open(args.dst, "w", encoding="utf-8") as f:
        f.write(header + body + "\n")
    rep = args.report or args.dst + ".report.md"
    by = conv.report.summary()
    with open(rep, "w", encoding="utf-8") as f:
        f.write("# 1.0 animatic report for %s\n\n" % os.path.basename(args.src))
        for kind in ("converted", "approximated", "placeholder", "dropped"):
            items = by.get(kind, [])
            f.write("## %s (%d)\n\n" % (kind.capitalize(), len(items)))
            for w, t in items:
                f.write("- `%s`: %s\n" % (w, t))
            f.write("\n")
    print("wrote %s and %s (%s)" % (args.dst, rep, ", ".join("%d %s" % (len(by.get(k, [])), k)
                                                       for k in ("converted", "approximated", "placeholder", "dropped"))))


if __name__ == "__main__":
    sys.exit(main())
