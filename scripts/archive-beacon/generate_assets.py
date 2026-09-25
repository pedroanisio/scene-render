#!/usr/bin/env python3
"""Generate the procedural assets used by examples/archive-beacon.xml.

Everything is synthesized locally, so the demo carries no third-party media:

  assets/generated/starfield/starfield.png  seeded RGBA star panorama
  assets/generated/telemetry/telemetry.mp4  FFmpeg testsrc2 signal
  assets/generated/audio/*.wav              synthetic hum/impact/alarm/etc.

The starfield seed and the regions kept clear for the 3D planet and station
come from layout.py so the image and the scene cannot drift apart. Requires
`ffmpeg` on PATH. Output is deterministic for a given seed.
"""
import os
import struct
import subprocess
import sys
import zlib

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import layout as L  # noqa: E402

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
OUT = os.path.join(ROOT, "assets", "generated")


def splitmix64(state):
    state = (state + 0x9E3779B97F4A7C15) & 0xFFFFFFFFFFFFFFFF
    z = state
    z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & 0xFFFFFFFFFFFFFFFF
    z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & 0xFFFFFFFFFFFFFFFF
    return state, z ^ (z >> 31)


class Rng:
    def __init__(self, seed):
        self.state = seed

    def unit(self):
        self.state, value = splitmix64(self.state)
        return (value >> 11) / float(1 << 53)


def write_png(path, width, height, rgba):
    def chunk(tag, data):
        body = tag + data
        return (struct.pack(">I", len(data)) + body +
                struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF))
    stride = width * 4
    raw = b"".join(b"\x00" + bytes(rgba[y * stride:(y + 1) * stride])
                   for y in range(height))
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n")
        f.write(chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6,
                                           0, 0, 0)))
        f.write(chunk(b"IDAT", zlib.compress(raw, 9)))
        f.write(chunk(b"IEND", b""))


def clear_zone(x, y):
    """True where 3D primitives are drawn; 2D stars would cover them."""
    px, py, rx, ry = L.PLANET_X, L.PLANET_Y, L.PLANET_RX + 40, L.PLANET_RY + 40
    if ((x - px) / rx) ** 2 + ((y - py) / ry) ** 2 <= 1.0:
        return True
    x0, y0, x1, y1 = L.STATION_CLEAR
    return x0 <= x <= x1 and y0 <= y <= y1


def starfield():
    import math
    w, h = L.PANO_W, L.PANO_H
    rgba = bytearray(w * h * 4)
    rng = Rng(L.SEED)
    palette = [(255, 255, 255), (200, 220, 255), (255, 236, 200),
               (170, 200, 255), (255, 210, 170)]
    placed = 0
    while placed < 5200:
        x = rng.unit() * w
        # Uniform on the sphere: sample latitude by asin so the poles are
        # not overcrowded after equirectangular stretching.
        lat = math.asin(2.0 * rng.unit() - 1.0)
        y = (0.5 - lat / math.pi) * h
        mag = rng.unit()
        color = palette[int(rng.unit() * len(palette))]
        if clear_zone(x, y):
            continue
        placed += 1
        radius = 0.6 + 1.9 * mag ** 6
        bright = 0.35 + 0.65 * mag
        stretch = 1.0 / max(0.15, math.cos(lat))
        rxs, rys = radius * stretch, radius
        for yy in range(int(y - rys - 1), int(y + rys + 2)):
            if yy < 0 or yy >= h:
                continue
            for xx in range(int(x - rxs - 1), int(x + rxs + 2)):
                d = math.hypot((xx + .5 - x) / rxs, (yy + .5 - y) / rys)
                a = max(0.0, 1.0 - d) * bright
                if a <= 0.0:
                    continue
                at = (yy * w + (xx % w)) * 4
                old = rgba[at + 3] / 255.0
                na = a + old * (1 - a)
                for c in range(3):
                    rgba[at + c] = color[c]
                rgba[at + 3] = int(min(1.0, na) * 255)
    os.makedirs(os.path.join(OUT, "starfield"), exist_ok=True)
    write_png(os.path.join(OUT, "starfield", "starfield.png"), w, h, rgba)


def ffmpeg(*args):
    subprocess.run(["ffmpeg", "-nostdin", "-v", "error", "-y", *args],
                   check=True)


def telemetry():
    os.makedirs(os.path.join(OUT, "telemetry"), exist_ok=True)
    ffmpeg("-f", "lavfi", "-i", "testsrc2=size=640x360:rate=30:duration=10",
           "-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "20",
           "-bitexact", os.path.join(OUT, "telemetry", "telemetry.mp4"))


AUDIO = {
    # Low station hum: detuned harmonics with a slow tremolo.
    "hum": ("30", "0.22*(sin(2*PI*55*t)+0.5*sin(2*PI*110.4*t)"
                  "+0.25*sin(2*PI*165.2*t))*(0.8+0.2*sin(2*PI*0.25*t))"),
    # Light burst: rising filtered-noise whoosh approximated by a chirp.
    "burst": ("1.6", "0.5*sin(2*PI*(80*t+260*t*t))*sin(PI*t/1.6)"
                     "+0.15*(random(0)-0.5)*sin(PI*t/1.6)"),
    # Impact: thud with a noisy crack, exponential decay.
    "impact": ("0.7", "(0.8*sin(2*PI*62*t)+0.4*(random(0)-0.5))*exp(-7*t)"),
    # Alarm: two-tone pulses.
    "alarm": ("1.6", "0.3*sin(2*PI*if(lt(mod(t,0.4),0.2),880,660)*t)"
                     "*lt(mod(t,0.2),0.16)"),
    # Shield hit: resonant ring sweeping down.
    "shield": ("1.8", "0.45*sin(2*PI*(420*t-60*t*t))*exp(-2.4*t)"
                      "+0.3*sin(2*PI*48*t)*exp(-4*t)"),
    # Transmission: pulsed rising carrier.
    "transmit": ("4", "0.3*sin(2*PI*(300*t+45*t*t))"
                      "*(0.6+0.4*sin(2*PI*8*t))*min(1,t*4)*min(1,(4-t)*2)"),
}


def audio():
    os.makedirs(os.path.join(OUT, "audio"), exist_ok=True)
    for name, (duration, expr) in AUDIO.items():
        ffmpeg("-f", "lavfi", "-i",
               f"aevalsrc=exprs='{expr}':s=48000:d={duration}:c=mono",
               "-ac", "2", "-c:a", "pcm_s16le", "-bitexact",
               os.path.join(OUT, "audio", f"{name}.wav"))


if __name__ == "__main__":
    starfield()
    telemetry()
    audio()
    print(f"assets written to {OUT}")
