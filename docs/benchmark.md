# Representative 10-second 4K benchmark

## Workload

`examples/v1.1-feature-showcase.xml` renders 300 frames at 3840×2160 and
30/1 FPS. It exercises the current v1.1 pipeline rather than a synthetic empty
frame: Display-P3 working color converted to tagged sRGB output, shaped Arabic
and Latin text, a cubic vector path, nested rounded/inverted masks, an imported
OBJ mesh and sphere with four light types, depth/shadow approximations, two
fixed-step rigid bodies and a constraint, four deformation modifiers, soft-body
motion, two particle systems, bloom/grade/vignette/flare, and stereo AAC audio.

Command:

```sh
./build/scene-render \
  --scene examples/v1.1-feature-showcase.xml \
  --output artifacts/v1.1-feature-showcase-4k.mp4 \
  --threads 8 --metrics --verbose
```

The standalone `benchmarks/run-phase1.sh` remains available for the lighter
ten-layer compositing baseline.

## Host

| Item | Value |
|---|---|
| Kernel / architecture | Linux 6.18.44, x86-64 |
| CPU | Intel Xeon Platinum 8573C, managed VM |
| Available logical CPUs | 9 |
| Requested workers | 8 |
| Compiler / optimization | GCC 13.3.0, `-O2` |
| FFmpeg | 6.1.1-3ubuntu5 |
| Video encoder | libx264, medium preset, CRF 18, 8 threads |

## Result

| Metric | Measured |
|---|---:|
| Frames | 300 |
| Render section | 793.399677 s |
| Encoder writes + final wait | 12.554869 s |
| End-to-end wall time | 806.784928 s |
| End-to-end throughput | 0.372 FPS |
| Render-only throughput | 0.378 FPS |
| Engine peak RSS | 104,048 KiB (101.61 MiB) |
| FFmpeg/child peak RSS | 2,160,088 KiB (2,109.46 MiB) |
| Conservative summed peak | 2,264,136 KiB (2,211.07 MiB) |
| Output size | 8,485,987 bytes |
| SHA-256 | `f3723d3ca936cdfefe3e47d1bf99182d9a46c28a5dfbfe6c43560f6a714f7807` |

`ffprobe` and a full decode check reported:

```text
video: h264, 3840x2160, yuv420p, 30/1, 300 frames, 10.000000 s
color: tv range, bt709 matrix/primaries, IEC 61966-2-1 transfer
audio: aac, 48000 Hz, stereo, 10.000000 s
container: MP4, 10.000000 s
decode errors: none
```

The RSS sum is deliberately conservative: `getrusage` exposes independent
self and child maxima, which need not occur simultaneously. The engine itself
streams one frame at a time; most of this run's memory upper bound belongs to
libx264's 4K lookahead/reference buffers.

## Baseline comparison

The previous v1.0 full-feature showcase measured 1,106.377 s wall time and
0.271 FPS on the same host class. The v1.1 run is 27.1% shorter and 37.3%
higher-throughput while adding paths, shaped text, OBJ geometry, transformed
masks, named-space conversion, and more explicit metadata. Sliding-window blur
and deterministic parallel effect/color rows account for the main improvement.

The simple ten-layer v1.0 baseline remains 135.138 s (2.220 FPS) because it
does not execute the expensive effects, particles, deformation, mesh lighting,
or shaped-text feature set. Neither result is real-time; the CPU renderer is a
deterministic reference implementation. The optional OpenCL path currently
accelerates final color conversion only, so it should not be presented as a
full GPU raster benchmark.

## Measurement tools

**Per-stage metrics.** `--metrics` now also prints engine CPU (`user_s`,
`sys_s`), setup time, and a `stages:` line. With the in-process libav encoder
there are no FFmpeg child processes, so engine CPU is the whole cost. Each stage shows `wall/cpu` seconds, where cpu covers every
engine thread, so a cpu/wall ratio near 1 means the stage ran serially. The
stages are `clear`, `lighting`, `composite`, `viewport`, `effects`, `convert`,
`resume` and `encode`.

`--metrics-trace FILE` writes one JSON row per frame, plus a final summary
row with totals and the slowest frame per stage:

```sh
./build/scene-render --scene examples/archive-beacon.xml \
  --frame-range 390:420 --metrics --metrics-trace build/trace.jsonl
```

**Profiling.** `make profile` (or `scripts/profile.sh [SCENE] [A:B]
[THREADS]`) builds a `-pg` binary in `build/profile/`. It renders the slice
and writes gprof `flat.txt` and `graph.txt` next to the trace. CMake builds
the same binary with `-DSR_PROFILE=ON`. gprof samples only the main thread
and cannot see time spent in libm, libc or the kernel, so profile with
`THREADS=1` and compare the report total with `user_s`.

**Regression guard.** `make perf-check` renders
`benchmarks/perf-scene.xml`, a 30-frame 1280×720 viewport scene that uses
every stage and only committed assets. It does one warm-up and then 5 timed
runs, and compares the median engine CPU per stage with
`benchmarks/baseline.json`. The check fails in either case:

- **Timing:** any stage above the noise floor, or the total, is more than 15%
  slower (exit 1).
- **Output:** sample frames differ between `--threads 1` and `--threads 4`,
  or differ from the recorded hashes (exit 2).

It is not part of `make test`, because timings depend on the host. The
committed baseline comes from an Intel Core Ultra 7 155H with 22 threads.
On that machine, run-to-run spread was 4% or less. On any other machine,
record a local baseline first with
`python3 scripts/perf-check.py --update`. Commit a new baseline only on
purpose, and only when both the host and the change are known.
