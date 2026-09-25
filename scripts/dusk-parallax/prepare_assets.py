#!/usr/bin/env python3
"""Upscale the CC0 "Mountain at Dusk" layers for examples/dusk-parallax.xml.

The originals (assets/third-party/dusk-parallax/original) are 160 px tall
pixel art. The engine samples images bilinearly, so the layers are enlarged
here with nearest-neighbour filtering to keep the pixels crisp: the sky by
8x (it must cover a 1920 px frame on its own) and every other plane by 7x
(1120 px tall, 40 px taller than the 1080p frame). The fog sprite is used
at its original size. Requires ffmpeg on PATH.
"""
import os
import subprocess

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
DIR = os.path.join(ROOT, "assets", "third-party", "dusk-parallax")

# output name -> (original file, integer scale)
LAYERS = {
    "sky.png": ("parallax-mountain-bg.png", 8),
    "far-peaks.png": ("parallax-mountain-montain-far.png", 7),
    "mountains.png": ("parallax-mountain-mountains.png", 7),
    "forest.png": ("parallax-mountain-trees.png", 7),
    "foreground.png": ("parallax-mountain-foreground-trees.png", 7),
}


def main():
    for name, (source, scale) in LAYERS.items():
        subprocess.run(
            ["ffmpeg", "-nostdin", "-v", "error", "-y",
             "-i", os.path.join(DIR, "original", source),
             "-vf", f"scale=iw*{scale}:ih*{scale}:flags=neighbor,format=rgba",
             "-frames:v", "1", os.path.join(DIR, name)],
            check=True)
        print("wrote", os.path.join("assets", "third-party", "dusk-parallax",
                                    name))


if __name__ == "__main__":
    main()
