#!/bin/sh
# Profile a scene slice with gprof.
#
#   scripts/profile.sh [SCENE] [FIRST:END] [THREADS]
#
# Defaults: benchmarks/perf-scene.xml, all 30 frames, 1 thread. gprof samples
# only the main thread, so THREADS=1 attributes every stage to it; use the
# per-stage timers (--metrics / --metrics-trace) for multi-threaded runs.
# Time inside libm, libc and the kernel is not attributed by gprof; compare
# the report total with user_s in metrics.txt to see how much is missing.
#
# Outputs in build/profile/: scene-render (instrumented), flat.txt, graph.txt,
# metrics.txt, trace.jsonl. Extra make variables (CPPFLAGS, LDFLAGS, CC) are
# passed through from the environment or `make profile VAR=...`.
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
scene=${1:-benchmarks/perf-scene.xml}
range=${2:-0:30}
threads=${3:-1}
out="$root/build/profile"

case "$scene" in /*) ;; *) scene="$root/$scene" ;; esac

make -C "$root" SR_PROFILE=1 BUILD=build/profile build/profile/scene-render >/dev/null
rm -f "$out/gmon.out"
# gprof writes gmon.out into the working directory at exit.
(cd "$out" && ./scene-render --scene "$scene" --frame-range "$range" \
    --threads "$threads" --output "$out/profile.mkv" \
    --metrics --metrics-trace "$out/trace.jsonl" 2> "$out/metrics.txt")
gprof -b -p "$out/scene-render" "$out/gmon.out" > "$out/flat.txt"
gprof -b -q "$out/scene-render" "$out/gmon.out" > "$out/graph.txt"

cat "$out/metrics.txt"
echo "--- top functions (self time) ---"
sed -n '1,20p' "$out/flat.txt"
echo "reports: $out/flat.txt $out/graph.txt $out/trace.jsonl"
