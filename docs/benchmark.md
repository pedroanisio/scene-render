# Final 10-second 4K benchmark

## Workload

`examples/ten-layer-composition.xml` renders without resolution, duration, or
FPS overrides:

- 3840×2160 RGBA working frame;
- 10.000 seconds at 30/1 FPS, 300 frames;
- ten simultaneous instances of one shared decoded source;
- nested transform and group clipping;
- normal, add, multiply, screen, overlay, and difference modes;
- linear-light compositing;
- completed v1.0.0 CPU pipeline and one FFmpeg encoder thread;
- H.264/libx264, yuv420p, `low`/veryfast quality.

Command:

```sh
sh benchmarks/run-phase1.sh "$(pwd)" "$(pwd)/build/scene-render" \
  "$(pwd)/artifacts/phase1-10s-4k.mp4" \
  "$(pwd)/build/benchmark-10s-4k.log"
```

The script name is retained for compatibility with the original baseline; the
reported run used the completed 1.0.0 binary.

## Host

| Item | Value |
|---|---|
| Kernel / architecture | Linux 6.18.44, x86-64 |
| CPU | Intel Xeon Platinum 8573C, managed VM |
| Available logical CPUs | 9 |
| Compiler / optimization | GCC 13.3.0, `-O2` |
| FFmpeg | 6.1.1-3ubuntu5 |

## Result

| Metric | Measured |
|---|---:|
| Frames | 300 |
| Render section | 118.912167 s |
| Encoder write + final wait | 16.223750 s |
| End-to-end wall time | 135.138277 s |
| End-to-end throughput | 2.220 FPS |
| Render-only throughput | 2.523 FPS |
| Peak self/child RSS | 708,468 KiB (691.86 MiB) |
| Output size | 310,125 bytes |
| SHA-256 | `03a27100ee9833a40132e6a9b569dfd1dfae045b13fe01a3c6f40c1f763f0399` |

`ffprobe`:

```text
codec_name=h264
width=3840
height=2160
pix_fmt=yuv420p
r_frame_rate=30/1
nb_frames=300
duration=10.000000
size=310125
```

The content is deliberately low entropy, so encoded size is not a quality or
bitrate benchmark. Peak RSS includes the FFmpeg child; the engine's own
one-frame streaming design is substantially smaller than that combined peak.

## Interpretation

The acceptance workload completes correctly with bounded memory but is not
real-time on the scalar CPU compositor. Deterministic multithreading currently
accelerates viewport projection, not general multilayer traversal. The obvious
future optimization path is tiled per-layer traversal with pre-resolved media
frames, transfer-function lookup/SIMD, and an optional GPU backend. Such a
backend must preserve the exact CPU reference or publish a separate perceptual
tolerance contract.
