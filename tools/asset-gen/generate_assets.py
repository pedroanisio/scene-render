#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10"
# dependencies = ["openai>=2.0", "pillow>=10", "numpy>=1.26", "jsonschema>=4.20", "soxr>=0.3"]
# ///
# SPDX-License-Identifier: Apache-2.0
"""Generate a scene's media assets with the OpenAI API from an asset spec.

The spec (see asset-spec.schema.json) holds the creative prompt and the exact
technical requirements of every asset. For each asset the tool:

  1. plans a request the API accepts (size rules below) and a crop back to the
     declared aspect;
  2. calls the API: images.generate, or images.edit with reference images for
     consistency (sequences reference their own earlier frames);
  3. post-processes to the exact declared size, alpha, sample rate, channels and
     duration;
  4. verifies the files, writes asset_manifest.json with SHA-256 values and a
     generation log, and can pin the hashes into the scene (--pin-scene).

OpenAI has no seed parameter, so results are not reproducible. The generation
log and the pinned hashes are what make renders deterministic
(docs/schema-1.1-batch3-proposal.md D7). Outputs that already exist are kept
unless --force.

API facts this tool relies on (OpenAI docs, checked 2026-09-25):
  * GPT image models return base64 PNG/WebP/JPEG. Custom sizes need both edges
    as multiples of 16, an aspect ratio between 1:3 and 3:1, edges <= 3840 and
    655,360..8,294,400 pixels; above 2560x1440 is experimental.
  * background="transparent" works only with gpt-image-2.5-sunburst and
    gpt-image-2.5-flare, with png or webp output.
  * images.edit takes up to 16 reference images and has no moderation option.
  * Speech: audio.speech.create; response_format="pcm" is 24 kHz 16-bit mono.
  * There is no music or sound-effect endpoint: those assets are "external".

Usage:
  uv run tools/asset-gen/generate_assets.py SPEC.json --dry-run
  OPENAI_API_KEY=... uv run tools/asset-gen/generate_assets.py SPEC.json [--only a,b] [--force]
  uv run tools/asset-gen/generate_assets.py SPEC.json --provider mock --out-root /tmp/x

Exit status: 0 complete and verified, 1 failures, 2 spec or usage error,
3 incomplete (external assets missing, or stopped by --max-calls).
"""

import argparse
import base64
import concurrent.futures as cf
import datetime as dt
import hashlib
import io
import json
import math
import os
import re
import sys
import threading
import wave
import xml.etree.ElementTree as ET

HERE = os.path.dirname(os.path.abspath(__file__))
SCHEMA_PATH = os.path.join(HERE, "asset-spec.schema.json")
TRANSPARENT_MODELS = ("gpt-image-2.5-sunburst", "gpt-image-2.5-flare")
MIN_PIXELS, MAX_PIXELS, MAX_EDGE = 655_360, 8_294_400, 3840
STABLE_PIXELS = 2560 * 1440
TTS_RATE = 24_000
MAX_REFS = 16
DEFAULTS = {
    "image": {"model": "gpt-image-2.5-sunburst", "quality": "high", "moderation": "auto",
              "inputFidelity": "high", "maxPixels": MAX_PIXELS},
    "speech": {"model": "gpt-4o-mini-tts", "voice": "cedar", "speed": 1.0},
}
DEFAULT_AVOID = ["text", "letters", "watermark", "signature", "border", "frame", "UI elements"]


class SpecError(Exception):
    pass


def log(msg):
    print(msg, file=sys.stderr, flush=True)


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def atomic_write(path, data):
    os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
    tmp = path + ".tmp-%d" % os.getpid()
    with open(tmp, "wb") as f:
        f.write(data)
    os.replace(tmp, path)

# ------------------------------------------------------------------ spec


def load_spec(path):
    import jsonschema
    with open(path, encoding="utf-8") as f:
        spec = json.load(f)
    with open(SCHEMA_PATH, encoding="utf-8") as f:
        schema = json.load(f)
    errors = sorted(jsonschema.Draft202012Validator(schema).iter_errors(spec), key=lambda e: list(e.path))
    if errors:
        msgs = []
        for e in errors[:20]:
            where = "/".join(str(p) for p in e.path) or "(root)"
            if e.validator == "oneOf" and e.path and e.path[0] == "assets":
                a = spec["assets"][e.path[1]] if len(e.path) > 1 else {}
                msgs.append("%s: asset %r (kind %r) does not match its kind's schema: %s"
                            % (where, a.get("id"), a.get("kind"), best_match(e)))
            else:
                msgs.append("%s: %s" % (where, e.message))
        raise SpecError("spec does not match asset-spec.schema.json:\n  " + "\n  ".join(msgs))
    return spec


def best_match(err):
    import jsonschema
    sub = jsonschema.exceptions.best_match(err.context) if err.context else None
    return sub.message if sub is not None else err.message


class Asset:
    def __init__(self, raw, spec, root):
        self.raw, self.id, self.kind = raw, raw["id"], raw["kind"]
        self.root = root
        d = DEFAULTS["speech" if self.kind == "speech" else "image"]
        sd = spec.get("defaults", {}).get("speech" if self.kind == "speech" else "image", {})
        self.params = {**d, **sd, **raw.get("params", {})}

    def get(self, k, default=None):
        return self.raw.get(k, default)

    @property
    def frame_count(self):
        return self.raw["last"] - self.raw["first"] + 1 if self.kind == "imageSequence" else 1

    def frame_path(self, n):
        return os.path.join(self.root, self.raw["pattern"] % n)

    def outputs(self):
        if self.kind == "imageSequence":
            return [self.frame_path(self.raw["first"] + i) for i in range(self.frame_count)]
        return [os.path.join(self.root, self.raw["path"])]

    @property
    def is_image(self):
        return self.kind in ("image", "imageSequence")


def check_spec(spec, assets):
    ids, paths, errs = {}, {}, []
    for a in assets:
        if a.id in ids:
            errs.append("duplicate asset id %r" % a.id)
        ids[a.id] = a
        for p in a.outputs():
            if p in paths:
                errs.append("%s and %s both write %s" % (paths[p], a.id, p))
            paths[p] = a.id
        if a.is_image:
            if a.get("alpha") == "straight" and a.params["model"] not in TRANSPARENT_MODELS:
                errs.append("%s: alpha=straight needs one of %s, not %s" % (a.id, ", ".join(TRANSPARENT_MODELS), a.params["model"]))
            for r in a.get("references", []):
                if r not in {x.id for x in assets}:
                    errs.append("%s: reference %r is not an asset of this spec" % (a.id, r))
        if a.kind == "imageSequence":
            n = a.frame_count
            if n < 1:
                errs.append("%s: last < first" % a.id)
            prompts = a.get("frames", {}).get("prompts")
            if prompts is not None and len(prompts) != n:
                errs.append("%s: %d frame prompts for %d frames" % (a.id, len(prompts), n))
            stages = a.get("frames", {}).get("stages")
            if stages and stages[-1]["until"] < 1:
                errs.append("%s: the last stage must reach until=1" % a.id)
            if len(re.findall(r"%0?\d*d", a.get("pattern"))) != 1:
                errs.append("%s: pattern needs exactly one %%d" % a.id)
    if not errs:
        order(assets)                                   # raises on cycles
    if errs:
        raise SpecError("\n  ".join(["spec is inconsistent:"] + errs))


def order(assets):
    """Topological order by references; raises on cycles."""
    by_id, done, out, visiting = {a.id: a for a in assets}, set(), [], set()

    def visit(a, chain):
        if a.id in done:
            return
        if a.id in visiting:
            raise SpecError("reference cycle: " + " -> ".join(chain + [a.id]))
        visiting.add(a.id)
        for r in a.get("references", []):
            if r in by_id:                              # refs outside a --only subset already exist
                visit(by_id[r], chain + [a.id])
        visiting.discard(a.id)
        done.add(a.id)
        out.append(a)
    for a in assets:
        visit(a, [])
    return out

# ------------------------------------------------------------ scene check


def check_scene(scene_path, assets):
    """The spec must describe the scene's assets exactly: same ids, files,
    sizes, alpha, frame ranges and audio format."""
    root = ET.parse(scene_path).getroot()
    base = os.path.dirname(os.path.abspath(scene_path))
    scene_assets = {e.get("id"): e for e in (root.find("assets") if root.find("assets") is not None else [])}
    errs, warns = [], []
    spec_ids = {a.id: a for a in assets}
    for sid, e in scene_assets.items():
        a = spec_ids.get(sid)
        if e.tag not in ("image", "imageSequence", "audio"):
            continue
        if a is None:
            errs.append("scene asset %r (<%s>) has no spec entry" % (sid, e.tag))
            continue
        want = {"image": ("image",), "imageSequence": ("imageSequence",),
                "audio": ("speech", "music", "ambience", "soundEffect")}[e.tag]
        if a.kind not in want:
            errs.append("%s: scene has <%s>, spec kind is %s" % (sid, e.tag, a.kind))
            continue
        src = os.path.normpath(os.path.join(base, e.get("src")))
        mine = os.path.normpath(os.path.join(a.root, a.get("pattern") if a.kind == "imageSequence" else a.get("path")))
        if src != mine:
            errs.append("%s: scene src resolves to %s, spec writes %s" % (sid, src, mine))
        checks = []
        if a.is_image:
            checks += [("width", int), ("height", int)]
            salpha = e.get("alpha", "auto")
            if salpha not in ("auto", a.get("alpha")):
                errs.append("%s: scene alpha=%s, spec alpha=%s" % (sid, salpha, a.get("alpha")))
        if a.kind == "imageSequence":
            checks += [("first", int), ("last", int)]
            if e.get("fps") and a.get("fps") and float(e.get("fps")) != float(a.get("fps")):
                errs.append("%s: scene fps %s, spec fps %s" % (sid, e.get("fps"), a.get("fps")))
        if e.tag == "audio":
            for k in ("sampleRate", "channels"):
                if e.get(k) and int(e.get(k)) != a.get(k):
                    errs.append("%s: scene %s=%s, spec %s" % (sid, k, e.get(k), a.get(k)))
            if e.get("duration") and a.get("duration") and abs(float(e.get("duration")) - a.get("duration")) > 1e-6:
                errs.append("%s: scene duration %s, spec %s" % (sid, e.get("duration"), a.get("duration")))
        for k, conv in checks:
            if e.get(k) is not None and conv(e.get(k)) != a.get(k):
                errs.append("%s: scene %s=%s, spec %s" % (sid, k, e.get(k), a.get(k)))
    for a in assets:
        if a.id not in scene_assets:
            warns.append("spec asset %r is not used by the scene" % a.id)
    return errs, warns

# ---------------------------------------------------------------- planning


def ceil16(x):
    return int(math.ceil(x / 16.0)) * 16


def floor16(x):
    return max(16, int(math.floor(x / 16.0)) * 16)


def plan_size(w, h, max_pixels=MAX_PIXELS):
    """Smallest legal API size that covers the target after an aspect crop.
    Returns (api_w, api_h, note)."""
    a = w / h
    ac = min(3.0, max(1 / 3.0, a))
    if a >= ac:                                  # target wider (or equal): width drives
        W = float(w)
        H = W / ac
    else:                                        # target taller: height drives
        H = float(h)
        W = H * ac
    note = []
    if W * H < MIN_PIXELS:
        k = math.sqrt(MIN_PIXELS / (W * H))
        W, H = W * k, H * k
    cap = min(max_pixels, MAX_PIXELS)
    if W * H > cap or max(W, H) > MAX_EDGE:
        k = min(math.sqrt(cap / (W * H)), MAX_EDGE / max(W, H))
        W, H = W * k, H * k
        note.append("upscaled from the API maximum")
        Wi, Hi = floor16(W), floor16(H)
    else:
        Wi, Hi = ceil16(W), ceil16(H)
    # rounding may leave the 1:3..3:1 range or the pixel range: nudge the short edge
    while Wi / Hi > 3:
        Hi += 16
    while Hi / Wi > 3:
        Wi += 16
    while Wi * Hi < MIN_PIXELS:
        Wi, Hi = Wi + 16, Hi + 16 * max(1, round(Hi / Wi))
    while Wi * Hi > MAX_PIXELS or max(Wi, Hi) > MAX_EDGE:
        Wi, Hi = Wi - 16, max(16, Hi - 16)
    if Wi * Hi > STABLE_PIXELS:
        note.append("above 2560x1440 (experimental in the API)")
    return Wi, Hi, "; ".join(note)


def crop_box(src_w, src_h, dst_w, dst_h, focus):
    """Largest box of the target aspect inside the source, centred on focus."""
    a = dst_w / dst_h
    if src_w / src_h > a:
        cw, ch = src_h * a, src_h
    else:
        cw, ch = src_w, src_w / a
    fx, fy = focus
    x = min(max(fx * src_w - cw / 2, 0), src_w - cw)
    y = min(max(fy * src_h - ch / 2, 0), src_h - ch)
    return (x, y, x + cw, y + ch)


def band_fraction(api_w, api_h, w, h):
    """Fraction of the generated image that survives the crop, and its axis."""
    if api_w / api_h > w / h:
        return (api_h * (w / h)) / api_w, "vertical"
    return (api_w / (w / h)) / api_h, "horizontal"


def stage_text(stages, progress):
    for s in stages:
        if progress <= s["until"] + 1e-9:
            return s["text"]
    return stages[-1]["text"]


def build_prompt(spec, a, api_w, api_h, frame=None):
    style = spec.get("style", {})
    parts = []
    if style.get("prompt"):
        parts.append(style["prompt"].strip())
    if style.get("palette"):
        parts.append("Palette: " + ", ".join("%s %s" % (k, v) for k, v in style["palette"].items()) + ".")
    parts.append(a.get("prompt").strip())
    if frame is not None:
        i, n = frame
        progress = 0.0 if n == 1 else i / (n - 1)
        fr = a.get("frames", {})
        text = fr["prompts"][i] if "prompts" in fr else stage_text(fr["stages"], progress)
        fps = a.get("fps")
        parts.append("This is pose %d of %d of a replacement-animation sequence%s. %s "
                     "Keep camera, framing, scale, lighting and the object's identity identical to the reference "
                     "frames; change only what the animation needs."
                     % (i + 1, n, " played at %g poses per second" % fps if fps else "", text.strip()))
    w, h = a.get("width"), a.get("height")
    if a.get("alpha") == "straight":
        parts.append("Isolate the subject on a fully transparent background: no backdrop, no floor, no "
                     "cast shadow outside the object, clean anti-aliased edges.")
    else:
        parts.append("Full-bleed image with no transparent areas.")
    frac, axis = band_fraction(api_w, api_h, w, h)
    if frac < 0.97:
        parts.append("Composition: the image will be cropped to its central %s band covering %d%% of the %s "
                     "(final aspect %d:%d). Keep the whole subject inside that band with a small margin."
                     % ("horizontal" if axis == "vertical" else "vertical", round(frac * 100),
                        "height" if axis == "vertical" else "width", w, h))
    grid = a.get("grid")
    if grid:
        parts.append("Lay out exactly %d separate objects in an evenly spaced grid of %d columns by %d rows; "
                     "each object centred in its own cell, not touching the cell edges or each other."
                     % (grid["cols"] * grid["rows"], grid["cols"], grid["rows"]))
    avoid = list(style.get("avoid", []))
    if a.get("text"):
        parts.append('Render exactly this lettering, spelled exactly and nothing else: "%s".' % a.get("text"))
    else:
        avoid += DEFAULT_AVOID
    if avoid:
        parts.append("Avoid: " + ", ".join(dict.fromkeys(avoid)) + ".")
    return "\n\n".join(parts)

# --------------------------------------------------------------- providers


ENV_KEYS = ("OPENAI_API_KEY", "OPENAI_BASE_URL", "OPENAI_ORG_ID", "OPENAI_PROJECT_ID")


def load_env_file(path):
    """Read the OpenAI variables from a dotenv file into os.environ. Other
    keys in the file are ignored, variables already set win, and values are
    never printed. Returns the names that were loaded."""
    loaded = []
    with open(path, encoding="utf-8") as f:
        for line in f:
            m = re.match(r"^\s*(?:export\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.*?)\s*$", line)
            if not m or m.group(1) not in ENV_KEYS or m.group(1) in os.environ:
                continue
            value = m.group(2)
            if len(value) >= 2 and value[0] == value[-1] and value[0] in "'\"":
                value = value[1:-1]
            else:
                value = re.sub(r"\s+#.*$", "", value)
            if value:
                os.environ[m.group(1)] = value
                loaded.append(m.group(1))
    return loaded


class OpenAIProvider:
    name = "openai"

    def __init__(self, timeout, max_retries):
        from openai import OpenAI
        if not os.environ.get("OPENAI_API_KEY"):
            raise SpecError("OPENAI_API_KEY is not set (the key is read from the environment only)")
        self.client = OpenAI(timeout=timeout, max_retries=max_retries)

    def image(self, prompt, size, params, background, refs):
        from PIL import Image
        kw = dict(model=params["model"], prompt=prompt, size="%dx%d" % size, n=1,
                  quality=params["quality"], background=background, output_format="png")
        if refs:
            files = [open(p, "rb") for p in refs]
            try:
                if params.get("inputFidelity"):
                    kw["input_fidelity"] = params["inputFidelity"]
                r = self.client.images.edit(image=files, **kw)
            finally:
                for f in files:
                    f.close()
        else:
            r = self.client.images.generate(moderation=params["moderation"], **kw)
        d = r.data[0]
        img = Image.open(io.BytesIO(base64.b64decode(d.b64_json)))
        img.load()
        meta = {"revised_prompt": getattr(d, "revised_prompt", None),
                "usage": r.usage.model_dump() if getattr(r, "usage", None) else None,
                "returned_size": list(img.size)}
        return img, meta

    def speech(self, text, params):
        import numpy as np
        kw = dict(model=params["model"], voice=params["voice"], input=text, response_format="pcm",
                  speed=params.get("speed", 1.0))
        if params.get("instructions") and params["model"].startswith("gpt-"):
            kw["instructions"] = params["instructions"]
        with self.client.audio.speech.with_streaming_response.create(**kw) as resp:
            pcm = resp.read()
        return np.frombuffer(pcm, dtype="<i2").astype(np.float32) / 32768.0, TTS_RATE, {}


class MockProvider:
    """Offline stand-in that exercises planning, post-processing, manifests and
    pinning without API calls. Images are labelled placeholders."""
    name = "mock"

    def image(self, prompt, size, params, background, refs):
        from PIL import Image, ImageDraw
        seed = int(hashlib.sha256(prompt.encode()).hexdigest()[:8], 16)
        col = ((seed >> 16) & 255, (seed >> 8) & 255, seed & 255)
        W, H = size
        img = Image.new("RGBA", size, (0, 0, 0, 0 if background == "transparent" else 255))
        d = ImageDraw.Draw(img)
        d.ellipse([W * 0.2, H * 0.3, W * 0.8, H * 0.7], fill=col + (255,))
        d.text((W * 0.22, H * 0.47), "mock %dx%d refs=%d" % (W, H, len(refs)), fill=(255, 255, 255, 255))
        return img, {"revised_prompt": None, "usage": None, "returned_size": [W, H]}

    def speech(self, text, params):
        import numpy as np
        n = int(TTS_RATE * max(0.5, len(text) / 15.0))
        t = np.arange(n) / TTS_RATE
        return (0.2 * np.sin(2 * np.pi * 220 * t)).astype(np.float32), TTS_RATE, {}

# ---------------------------------------------------------- post-process


def to_target(img, a):
    """Crop to the target aspect around focus, resample to the exact size in
    premultiplied space, and set the declared alpha mode."""
    from PIL import Image
    w, h = a.get("width"), a.get("height")
    img = img.convert("RGBA")
    focus = tuple(a.get("focus", [0.5, 0.5]))
    if a.get("fit", "cover") == "cover":
        img = img.crop(tuple(round(v) for v in crop_box(img.width, img.height, w, h, focus)))
        img = img.convert("RGBa").resize((w, h), Image.LANCZOS, reducing_gap=3.0).convert("RGBA")
    else:
        k = min(w / img.width, h / img.height)
        inner = img.convert("RGBa").resize((max(1, round(img.width * k)), max(1, round(img.height * k))),
                                           Image.LANCZOS).convert("RGBA")
        bg = (0, 0, 0, 0) if a.get("alpha") == "straight" else (0, 0, 0, 255)
        img = Image.new("RGBA", (w, h), bg)
        img.alpha_composite(inner, ((w - inner.width) // 2, (h - inner.height) // 2))
    warn = []
    if a.get("alpha") == "straight":
        alpha = img.getchannel("A")
        hist = alpha.histogram()
        clear = sum(hist[:8]) / (w * h)
        if clear < 0.01:
            warn.append("the model returned an opaque image: less than 1% of pixels are transparent")
        grid = a.get("grid")
        if grid:
            warn += clear_grid(img, grid)
    else:
        bg = Image.new("RGBA", img.size, (0, 0, 0, 255))
        bg.alpha_composite(img)
        img = bg.convert("RGB")
    return img, warn


def clear_grid(img, grid):
    """Zero the alpha of the cell borders of a sprite sheet so no chunk bleeds
    into its neighbour, and report empty cells."""
    import numpy as np
    arr = np.array(img)
    H, W = arr.shape[:2]
    g = grid.get("gutter", 2)
    cols, rows = grid["cols"], grid["rows"]
    warn = []
    for c in range(1, cols):
        x = round(c * W / cols)
        arr[:, max(0, x - g):x + g, 3] = 0
    for r in range(1, rows):
        y = round(r * H / rows)
        arr[max(0, y - g):y + g, :, 3] = 0
    for r in range(rows):
        for c in range(cols):
            cell = arr[round(r * H / rows):round((r + 1) * H / rows), round(c * W / cols):round((c + 1) * W / cols), 3]
            if (cell > 32).mean() < 0.01:
                warn.append("sprite cell (col %d, row %d) is empty" % (c, r))
    from PIL import Image
    img.paste(Image.fromarray(arr, "RGBA"))
    return warn


def png_bytes(img):
    buf = io.BytesIO()
    img.save(buf, format="PNG", compress_level=9)     # no metadata: identical pixels give identical bytes
    return buf.getvalue()


def audio_to_target(x, rate, a):
    import numpy as np
    import soxr
    target, ch = a.get("sampleRate"), a.get("channels")
    y = soxr.resample(x.astype(np.float64), rate, target, quality="VHQ") if rate != target else x.astype(np.float64)
    warn = []
    dur = a.get("duration")
    if dur:
        n = int(round(dur * target))
        mode = a.get("fitDuration", "pad-or-trim")
        if len(y) > n:
            if mode == "pad":
                raise RuntimeError("speech is %.3f s, longer than duration %.3f s (fitDuration=pad)" % (len(y) / target, dur))
            fade = min(n, int(0.01 * target))
            y = y[:n].copy()
            if fade:
                y[-fade:] *= np.linspace(1, 0, fade)
            warn.append("trimmed to %.3f s" % dur)
        elif len(y) < n:
            if mode == "trim":
                raise RuntimeError("speech is %.3f s, shorter than duration %.3f s (fitDuration=trim)" % (len(y) / target, dur))
            y = np.concatenate([y, np.zeros(n - len(y))])
    y = np.clip(y, -1.0, 1.0)
    frames = np.repeat(y[:, None], ch, axis=1)
    bits = a.get("bitDepth", 24)
    if bits == 16:
        data = np.round(frames * 32767).astype("<i2").tobytes()
    else:
        v = np.round(frames * 8388607).astype("<i4").reshape(-1)
        data = np.stack([v & 255, (v >> 8) & 255, (v >> 16) & 255], axis=1).astype(np.uint8).tobytes()
    buf = io.BytesIO()
    with wave.open(buf, "wb") as wf:
        wf.setnchannels(ch)
        wf.setsampwidth(bits // 8)
        wf.setframerate(target)
        wf.writeframes(data)
    return buf.getvalue(), warn

# ------------------------------------------------------------ verification


def wav_info(path):
    with wave.open(path, "rb") as wf:
        return wf.getframerate(), wf.getnchannels(), wf.getsampwidth() * 8, wf.getnframes() / wf.getframerate()


def verify(a):
    """Check an asset's files against its spec. Returns a list of problems."""
    from PIL import Image
    probs = []
    for p in a.outputs():
        if not os.path.exists(p):
            probs.append("missing %s" % p)
            continue
        if a.is_image:
            with Image.open(p) as im:
                if im.size != (a.get("width"), a.get("height")):
                    probs.append("%s is %dx%d, spec %dx%d" % (p, im.width, im.height, a.get("width"), a.get("height")))
                has_alpha = im.mode in ("RGBA", "LA") or (im.mode == "P" and "transparency" in im.info)
                if a.get("alpha") == "straight" and not has_alpha:
                    probs.append("%s has no alpha channel (mode %s)" % (p, im.mode))
        else:
            try:
                rate, ch, bits, dur = wav_info(p)
            except (wave.Error, EOFError) as ex:
                probs.append("%s is not a PCM WAV: %s" % (p, ex))
                continue
            if rate != a.get("sampleRate") or ch != a.get("channels"):
                probs.append("%s is %d Hz %dch, spec %d Hz %dch" % (p, rate, ch, a.get("sampleRate"), a.get("channels")))
            if a.get("duration") and abs(dur - a.get("duration")) > 1.0 / rate:
                probs.append("%s lasts %.4f s, spec %.4f s" % (p, dur, a.get("duration")))
            if a.kind == "speech" and bits != a.get("bitDepth", 24):
                probs.append("%s is %d-bit, spec %d-bit" % (p, bits, a.get("bitDepth", 24)))
    return probs

# ------------------------------------------------------------- generation


class Log:
    """Generation log: request hash, model, sizes and API metadata per output
    file, so reruns skip up-to-date files and a spec change is detected."""

    def __init__(self, path):
        self.path, self.lock = path, threading.Lock()
        self.data = {"files": {}}
        if os.path.exists(path):
            with open(path, encoding="utf-8") as f:
                self.data = json.load(f)

    def entry(self, rel):
        return self.data["files"].get(rel)

    def record(self, rel, info):
        with self.lock:
            self.data["files"][rel] = info
            atomic_write(self.path, json.dumps(self.data, indent=2, sort_keys=True).encode() + b"\n")


class Runner:
    def __init__(self, spec, assets, root, provider, args):
        self.spec, self.assets, self.root, self.provider, self.args = spec, assets, root, provider, args
        self.by_id = {a.id: a for a in assets}
        self.log = Log(os.path.join(root, ".asset-gen", "log.json"))
        self.calls, self.calls_lock = 0, threading.Lock()
        self.results = {}

    def rel(self, p):
        return os.path.relpath(p, self.root)

    def raw_path(self, a, index=None):
        name = a.id if index is None else "%s_%04d" % (a.id, index)
        return os.path.join(self.root, ".asset-gen", "raw", name + ".png")

    def ref_files(self, a):
        files = []
        for r in a.get("references", []):
            ra = self.by_id[r]
            raw = self.raw_path(ra, 0 if ra.kind == "imageSequence" else None)
            files.append(raw if os.path.exists(raw) else ra.outputs()[0])
        return files

    def take_call(self):
        with self.calls_lock:
            if self.args.max_calls is not None and self.calls >= self.args.max_calls:
                raise StopIteration("reached --max-calls %d" % self.args.max_calls)
            self.calls += 1

    def request_hash(self, **kw):
        return hashlib.sha256(json.dumps(kw, sort_keys=True).encode()).hexdigest()

    def up_to_date(self, path, req):
        e = self.log.entry(self.rel(path))
        return (os.path.exists(path) and e is not None and e.get("request") == req
                and e.get("sha256") == sha256_file(path))

    def keep_existing(self, path, req):
        """True when an existing file must be kept (not ours or spec changed, no --force)."""
        if not os.path.exists(path) or self.args.force:
            return False
        e = self.log.entry(self.rel(path))
        return e is None or e.get("request") != req

    def run_asset(self, a):
        status = {"id": a.id, "warnings": [], "state": "pending", "files": []}
        try:
            if a.kind == "image":
                self.gen_image(a, status)
            elif a.kind == "imageSequence":
                self.gen_sequence(a, status)
            elif a.kind == "speech":
                self.gen_speech(a, status)
            else:
                status["state"] = "external" if all(os.path.exists(p) for p in a.outputs()) else "missing"
        except StopIteration as ex:
            status["state"], status["error"] = "stopped", str(ex)
        except Exception as ex:                             # report and continue with other assets
            status["state"], status["error"] = "failed", "%s: %s" % (type(ex).__name__, ex)
        if status["state"] == "pending":
            status["state"] = "current"
        problems = verify(a) if status["state"] in ("done", "current", "external", "kept") else []
        if problems:                                        # wrong files fail whoever made them
            status["problems"] = problems
            status["state"] = "failed"
        self.results[a.id] = status
        log("%-24s %-9s %s" % (a.id, status["state"], status.get("error", "") or "; ".join(status["warnings"])))
        return status

    def one_image(self, a, out_path, raw_path, prompt, size, refs, status, extra):
        p = a.params
        background = "transparent" if a.get("alpha") == "straight" else "opaque"
        req = self.request_hash(model=p["model"], prompt=prompt, size=size, quality=p["quality"],
                                background=background, fidelity=p.get("inputFidelity") if refs else None,
                                refs=[sha256_file(r) for r in refs if os.path.exists(r)],
                                post=[a.get("width"), a.get("height"), a.get("alpha"), a.get("fit", "cover"),
                                      a.get("focus"), a.get("grid")], provider=self.provider.name, **extra)
        if not self.args.force and self.up_to_date(out_path, req):
            return False
        if self.keep_existing(out_path, req):
            status["state"] = "kept"
            status["warnings"].append("%s exists and was not produced by this spec version (use --force)" % self.rel(out_path))
            return False
        self.take_call()
        img, meta = self.provider.image(prompt, size, p, background, refs)
        atomic_write(raw_path, png_bytes(img.convert("RGBA")))
        final, warn = to_target(img, a)
        status["warnings"] += warn
        atomic_write(out_path, png_bytes(final))
        self.log.record(self.rel(out_path), {
            "asset": a.id, "request": req, "sha256": sha256_file(out_path), "provider": self.provider.name,
            "model": p["model"], "quality": p["quality"], "api_size": "%dx%d" % size, "background": background,
            "references": [self.rel(r) for r in refs], "prompt": prompt, "raw": self.rel(raw_path),
            "created": dt.datetime.now(dt.timezone.utc).isoformat(timespec="seconds"), **meta})
        status["files"].append(self.rel(out_path))
        status["state"] = "done"
        return True

    def gen_image(self, a, status):
        W, H, _ = plan_size(a.get("width"), a.get("height"), a.params["maxPixels"])
        prompt = build_prompt(self.spec, a, W, H)
        refs = self.ref_files(a)[:MAX_REFS]
        self.one_image(a, a.outputs()[0], self.raw_path(a), prompt, (W, H), refs, status, {})

    def gen_sequence(self, a, status):
        W, H, _ = plan_size(a.get("width"), a.get("height"), a.params["maxPixels"])
        n, mode = a.frame_count, a.get("consistency", "chain")
        base_refs = self.ref_files(a)
        for i, out in enumerate(a.outputs()):
            refs = list(base_refs)
            if i > 0 and mode in ("chain", "key"):
                refs.append(self.raw_path(a, 0))
            if i > 1 and mode == "chain":
                refs.append(self.raw_path(a, i - 1))
            refs = [r for r in refs if os.path.exists(r)][:MAX_REFS]
            prompt = build_prompt(self.spec, a, W, H, frame=(i, n))
            self.one_image(a, out, self.raw_path(a, i), prompt, (W, H), refs, status, {"frame": i})

    def gen_speech(self, a, status):
        out = a.outputs()[0]
        p = a.params
        req = self.request_hash(model=p["model"], voice=p["voice"], text=a.get("text"),
                                instructions=p.get("instructions"), speed=p.get("speed"),
                                post=[a.get("sampleRate"), a.get("channels"), a.get("bitDepth", 24),
                                      a.get("duration"), a.get("fitDuration")], provider=self.provider.name)
        if not self.args.force and self.up_to_date(out, req):
            return
        if self.keep_existing(out, req):
            status["state"] = "kept"
            status["warnings"].append("%s exists and was not produced by this spec version (use --force)" % self.rel(out))
            return
        self.take_call()
        x, rate, meta = self.provider.speech(a.get("text"), p)
        data, warn = audio_to_target(x, rate, a)
        status["warnings"] += warn
        atomic_write(out, data)
        self.log.record(self.rel(out), {
            "asset": a.id, "request": req, "sha256": sha256_file(out), "provider": self.provider.name,
            "model": p["model"], "voice": p["voice"], "text": a.get("text"), "instructions": p.get("instructions"),
            "created": dt.datetime.now(dt.timezone.utc).isoformat(timespec="seconds"), **meta})
        status["files"].append(self.rel(out))
        status["state"] = "done"

    def run(self, jobs):
        pending = order(self.assets)
        done_ids = set()
        with cf.ThreadPoolExecutor(max_workers=max(1, jobs)) as pool:
            futures = {}
            while pending or futures:
                ready = [a for a in pending if all(r in done_ids or r not in {x.id for x in self.assets}
                                                   for r in a.get("references", []))]
                for a in ready:
                    pending.remove(a)
                    futures[pool.submit(self.run_asset, a)] = a
                if not futures:
                    break
                finished, _ = cf.wait(futures, return_when=cf.FIRST_COMPLETED)
                for f in finished:
                    done_ids.add(futures.pop(f).id)
        return self.results

# ------------------------------------------------------------- manifests


def write_manifest(root, assets, scene_path):
    files = []
    for a in assets:
        for p in a.outputs():
            if os.path.exists(p):
                files.append({"path": os.path.relpath(p, root), "bytes": os.path.getsize(p), "sha256": sha256_file(p)})
    scene_rel = None
    if scene_path and os.path.exists(scene_path):
        scene_rel = os.path.relpath(scene_path, root)
        files.append({"path": scene_rel, "bytes": os.path.getsize(scene_path), "sha256": sha256_file(scene_path)})
    files.sort(key=lambda f: f["path"])
    manifest = {"scene": scene_rel, "file_count": len(files), "files": files}
    path = os.path.join(root, "asset_manifest.json")
    atomic_write(path, json.dumps(manifest, indent=2).encode() + b"\n")
    return path


def pin_scene(scene_path, out_path, assets):
    """Copy the scene with sha256 on every single-file image/audio asset. Text
    is edited in place, so the document's formatting is kept."""
    text = open(scene_path, encoding="utf-8").read()
    pinned = 0
    for a in assets:
        if a.kind == "imageSequence":
            continue
        p = a.outputs()[0]
        if not os.path.exists(p):
            continue
        digest = sha256_file(p)
        m = re.search(r'<(image|audio|video)\b[^>]*?\bid="%s"[^>]*?/?>' % re.escape(a.id), text, re.S)
        if not m:
            continue
        tag = m.group(0)
        if re.search(r'\bsha256="', tag):
            new = re.sub(r'\bsha256="[0-9a-f]*"', 'sha256="%s"' % digest, tag)
        else:
            new = re.sub(r'(\bid="%s")' % re.escape(a.id), r'\1 sha256="%s"' % digest, tag, count=1)
        text = text[:m.start()] + new + text[m.end():]
        pinned += 1
    atomic_write(out_path, text.encode("utf-8"))
    return pinned

# ----------------------------------------------------------------- main


def describe_plan(spec, assets, only):
    total = 0
    print("%-24s %-14s %-11s %-11s %-6s %s" % ("asset", "kind", "target", "api size", "calls", "notes"))
    for a in assets:
        if only and a.id not in only:
            continue
        if a.is_image:
            W, H, note = plan_size(a.get("width"), a.get("height"), a.params["maxPixels"])
            frac, _ = band_fraction(W, H, a.get("width"), a.get("height"))
            notes = [a.params["model"], a.params["quality"]]
            if a.get("alpha") == "straight":
                notes.append("transparent")
            if frac < 0.97:
                notes.append("keeps %d%% after crop" % round(frac * 100))
            if a.get("references"):
                notes.append("refs " + ",".join(a.get("references")))
            if note:
                notes.append(note)
            calls = a.frame_count
            print("%-24s %-14s %-11s %-11s %-6d %s" % (a.id, a.kind, "%dx%d" % (a.get("width"), a.get("height")),
                                                       "%dx%d" % (W, H), calls, "; ".join(notes)))
        elif a.kind == "speech":
            calls = 1
            print("%-24s %-14s %-11s %-11s %-6d %s %s" % (a.id, a.kind, "%dHz/%dch" % (a.get("sampleRate"), a.get("channels")),
                                                          "tts", calls, a.params["model"], a.params["voice"]))
        else:
            calls = 0
            state = "present" if os.path.exists(a.outputs()[0]) else "MISSING (no OpenAI endpoint)"
            print("%-24s %-14s %-11s %-11s %-6d external: %s" % (a.id, a.kind, "%dHz/%dch" % (a.get("sampleRate"), a.get("channels")),
                                                                 "-", calls, state))
        total += calls
    print("API calls if nothing is cached: %d" % total)
    return total


def main(argv=None):
    ap = argparse.ArgumentParser(description="Generate scene assets with the OpenAI API from an asset spec.")
    ap.add_argument("spec", help="asset spec JSON (asset-spec.schema.json)")
    ap.add_argument("--only", help="comma-separated asset ids")
    ap.add_argument("--force", action="store_true", help="regenerate even if files exist")
    ap.add_argument("--dry-run", action="store_true", help="validate and print the plan; no API calls")
    ap.add_argument("--provider", choices=["openai", "mock"], default="openai")
    ap.add_argument("--out-root", help="override project.outputRoot")
    ap.add_argument("--scene", help="override project.scene for the cross-check")
    ap.add_argument("--no-scene-check", action="store_true")
    ap.add_argument("--jobs", type=int, default=2, help="assets generated in parallel (default 2)")
    ap.add_argument("--max-calls", type=int, help="stop after this many API calls")
    ap.add_argument("--timeout", type=float, default=300.0, help="seconds per API request")
    ap.add_argument("--retries", type=int, default=6, help="SDK retries on 429/5xx/connection errors")
    ap.add_argument("--pin-scene", metavar="OUT", help="write a copy of the scene with sha256 on each asset")
    ap.add_argument("--env-file", default=".env",
                    help="dotenv file for OPENAI_API_KEY and related variables (default ./.env when present)")
    args = ap.parse_args(argv)
    if args.env_file and os.path.isfile(args.env_file):
        names = load_env_file(args.env_file)
        if names:
            log("loaded %s from %s" % (", ".join(names), args.env_file))

    try:
        spec = load_spec(args.spec)
        spec_dir = os.path.dirname(os.path.abspath(args.spec))
        root = os.path.abspath(args.out_root or os.path.join(spec_dir, spec["project"].get("outputRoot", ".")))
        assets = [Asset(r, spec, root) for r in spec["assets"]]
        check_spec(spec, assets)
        only = set(args.only.split(",")) if args.only else None
        if only and only - {a.id for a in assets}:
            raise SpecError("unknown --only ids: %s" % ", ".join(sorted(only - {a.id for a in assets})))
        scene = args.scene or (os.path.join(spec_dir, spec["project"]["scene"]) if spec["project"].get("scene") else None)
        if scene and not args.no_scene_check:
            errs, warns = check_scene(scene, assets)
            for w in warns:
                log("warning: " + w)
            if errs:
                raise SpecError("spec does not match the scene %s:\n  %s" % (scene, "\n  ".join(errs)))
    except (SpecError, OSError, json.JSONDecodeError, ET.ParseError) as ex:
        log("error: %s" % ex)
        return 2

    total = describe_plan(spec, assets, only)
    if args.dry_run:
        return 0
    try:
        provider = OpenAIProvider(args.timeout, args.retries) if args.provider == "openai" else MockProvider()
    except SpecError as ex:
        log("error: %s" % ex)
        return 2
    selected = [a for a in assets if not only or a.id in only]
    by_id = {a.id: a for a in assets}
    missing_refs = [(a.id, r) for a in selected for r in a.get("references", [])
                    if only and r not in only and not os.path.exists(by_id[r].outputs()[0])]
    if missing_refs:
        log("error: references not generated yet: %s" % ", ".join("%s->%s" % m for m in missing_refs))
        return 2
    log("generating %d asset(s) with %s into %s (up to %d calls)" % (len(selected), provider.name, root, total))
    runner = Runner(spec, selected, root, provider, args)
    runner.by_id = by_id                                # references may point outside --only
    results = runner.run(args.jobs)
    manifest = write_manifest(root, assets, scene if scene and os.path.exists(scene) else None)
    log("manifest: %s (%d API calls made)" % (manifest, runner.calls))
    if args.pin_scene and scene:
        n = pin_scene(scene, os.path.abspath(args.pin_scene), assets)
        log("pinned sha256 on %d assets -> %s" % (n, args.pin_scene))
    states = [r["state"] for r in results.values()]
    for r in results.values():
        for p in r.get("problems", []):
            log("problem: %s: %s" % (r["id"], p))
    if "failed" in states:
        return 1
    if "missing" in states or "stopped" in states:
        return 3
    return 0


if __name__ == "__main__":
    sys.exit(main())
