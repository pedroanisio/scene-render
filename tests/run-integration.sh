#!/bin/sh
# usage: run-integration.sh ROOT BINARY WORKDIR [PROBE]
#   ROOT     repository root (read-only: nothing is written below it)
#   BINARY   scene-render executable under test
#   WORKDIR  directory that receives every output, cache and copied scene
#   PROBE    sr-probe (tests/tools/sr-probe.c) for stream inspection;
#            default: sr-probe next to BINARY. No FFmpeg CLI is needed.
set -eu

if [ $# -ne 3 ] && [ $# -ne 4 ]; then
    echo "usage: $0 ROOT BINARY WORKDIR [PROBE]" >&2
    exit 2
fi
root=$(cd "$1" && pwd)
binary=$2
work=$3
probe_tool=${4:-$(dirname "$binary")/sr-probe}
test -x "$probe_tool" || {
    echo "sr-probe not found at $probe_tool" >&2
    exit 2
}
mkdir -p "$work"
work=$(cd "$work" && pwd)
case $binary in
    /*) ;;
    *) binary=$(pwd)/$binary ;;
esac

# Scenes that write next to themselves (physics caches) are copied into the
# work directory with their in-tree ../build/test-artifacts/ paths rewritten,
# so the source tree is never modified.
stage_scene() {
    sed -e 's#\.\./build/test-artifacts/##g' "$root/tests/$1" > "$work/$1"
}

# field FILE TYPE KEY: value of KEY on the first TYPE (video|audio) stream
# line printed by sr-probe.
field() {
    "$probe_tool" "$1" | awk -v type="type=$2" -v key="$3" '
        $2 == type { for (i = 1; i <= NF; ++i) {
            split($i, kv, "="); if (kv[1] == key) { print kv[2]; exit } } }'
}

# golden NAME FILE: compare FILE's SHA-256 with the NAME entry of
# tests/golden.sha256. Hashes are taken from stdin so the file's location
# never matters, and a missing entry is reported rather than compared as "".
golden() {
    expected=$(awk -v name="$1" '$2 == name {print $1}' "$root/tests/golden.sha256")
    if [ -z "$expected" ]; then
        echo "no golden hash named '$1' in $root/tests/golden.sha256" >&2
        exit 1
    fi
    actual=$(sha256sum < "$2" | awk '{print $1}')
    test "$actual" = "$expected" || {
        echo "$1 golden mismatch: expected $expected, got $actual" >&2
        exit 1
    }
    echo "golden $1 ok"
}

for scene in "$root"/examples/*.xml; do
    "$binary" --scene "$scene" --validate
done
"$binary" --scene "$root/examples/basic-multilayer.xml" \
    --renderer gpu --validate

if command -v xmllint >/dev/null 2>&1; then
    xsd_files=$(
        for path in "$root"/examples/*.xml "$root"/tests/data-*.xml; do
            case ${path##*/} in
                data-doctype.xml|data-duplicate.xml|data-invalid.xml) ;;
                *) printf '%s\n' "$path" ;;
            esac
        done | LC_ALL=C sort)
    # Paths come from globs under $root; word splitting on newlines only.
    old_ifs=$IFS
    IFS='
'
    # shellcheck disable=SC2086
    xmllint --noout --schema "$root/schema/scene-v1.xsd" $xsd_files
    IFS=$old_ifs
    echo "XSD validation passed"
else
    echo "NOTICE: xmllint not found; skipping XSD validation" >&2
fi

first="$work/golden-a.ppm"
second="$work/golden-b.ppm"
"$binary" --scene "$root/examples/keyframe-curves.xml" \
    --resolution 320x180 --preview-frame 30 --preview-out "$first"
"$binary" --scene "$root/examples/keyframe-curves.xml" \
    --resolution 320x180 --preview-frame 30 --preview-out "$second"
cmp "$first" "$second"
golden keyframe "$first"

ten_layer="$work/ten-layer.ppm"
"$binary" --scene "$root/examples/ten-layer-composition.xml" \
    --preview-frame 0 --preview-out "$ten_layer" --threads 4
golden ten-layer "$ten_layer"

viewport_a="$work/viewport-a.ppm"
viewport_b="$work/viewport-b.ppm"
"$binary" --scene "$root/tests/data-viewport.xml" --preview-frame 3 \
    --preview-out "$viewport_a" --threads 1
"$binary" --scene "$root/tests/data-viewport.xml" --preview-frame 3 \
    --preview-out "$viewport_b" --threads 4
cmp "$viewport_a" "$viewport_b"
golden viewport "$viewport_a"

advanced_a="$work/advanced-a.ppm"
advanced_b="$work/advanced-b.ppm"
stage_scene data-advanced.xml
rm -f "$work/advanced.physics"
"$binary" --scene "$work/data-advanced.xml" --preview-frame 8 \
    --preview-out "$advanced_a"
test -s "$work/advanced.physics"
"$binary" --scene "$work/data-advanced.xml" --preview-frame 8 \
    --preview-out "$advanced_b"
cmp "$advanced_a" "$advanced_b"
golden advanced "$advanced_a"

production_a="$work/production-a.ppm"
production_b="$work/production-b.ppm"
"$binary" --scene "$root/examples/production-features.xml" --preview-frame 30 \
    --preview-out "$production_a" --threads 1
"$binary" --scene "$root/examples/production-features.xml" --preview-frame 30 \
    --preview-out "$production_b" --threads 4 --renderer gpu
cmp "$production_a" "$production_b"
golden production "$production_a"

video="$work/integration.mp4"
"$binary" --scene "$root/examples/keyframe-curves.xml" --resolution 320x180 \
    --fps 12 --frame-range 0:12 --output "$video" --threads 1 --quality low
probe="$(field "$video" video width),$(field "$video" video height),$(field "$video" video frames)"
test "$probe" = "320,180,12" || {
    echo "unexpected probe result: $probe" >&2
    exit 1
}

media="$work/media.mp4"
"$binary" --scene "$root/tests/data-media.xml" --output "$media" --threads 1
video_codec=$(field "$media" video codec)
audio_codec=$(field "$media" audio codec)
test "$video_codec" = "h264" && test "$audio_codec" = "aac" || {
    echo "unexpected media codecs: video=$video_codec audio=$audio_codec" >&2
    exit 1
}
# Packet-derived durations; AAC frames are 1024 samples, so the audio track
# may overhang the last video frame by less than one frame.
video_duration=$(field "$media" video duration)
audio_duration=$(field "$media" audio duration)
awk -v v="$video_duration" -v a="$audio_duration" 'BEGIN {
    d = a - v; if (d < 0) d = -d
    if (d > 1024 / 48000) { printf "A/V duration mismatch: video=%s audio=%s\n", v, a; exit 1 } }'

# Segmented resume: two segments (4 + 2 frames) kept, then a rerun that
# reuses both must reproduce the same bytes and remove the parts directory.
viewport_video="$work/viewport.mp4"
rm -rf "$viewport_video.parts"
"$binary" --scene "$root/tests/data-viewport.xml" --output "$viewport_video" \
    --threads 4 --resume --segment-frames 4 --keep-parts
version=$("$binary" --version | awk '{print $2}')
grep -q "^version=$version\$" "$viewport_video.parts/manifest"
test "$(find "$viewport_video.parts" -name 'seg-*.mp4' | wc -l)" -eq 2
test "$(field "$viewport_video" video frames)" = "6"
cp "$viewport_video" "$work/viewport-first.mp4"
"$binary" --scene "$root/tests/data-viewport.xml" --output "$viewport_video" \
    --threads 4 --resume --segment-frames 4
cmp "$viewport_video" "$work/viewport-first.mp4"
test ! -e "$viewport_video.parts"

"$binary" --scene "$root/tests/data-equirect.xml" \
    --output "$work/equirect.mkv" --threads 1
dimensions="$(field "$work/equirect.mkv" video width),$(field "$work/equirect.mkv" video height)"
test "$dimensions" = "256,128"
test "$(field "$work/equirect.mkv" video spherical)" = "equirectangular"

"$binary" --scene "$root/tests/data-h265.xml" \
    --output "$work/h265.mp4" --threads 1
test "$(field "$work/h265.mp4" video codec)" = "hevc"

spatial="$work/spatial.mp4"
"$binary" --scene "$root/tests/data-spatial.xml" --output "$spatial" --threads 1
python3 - "$spatial" <<'PY'
import pathlib
import sys

data = pathlib.Path(sys.argv[1]).read_bytes()
uuid = bytes.fromhex("ffcc8263f8554a938814587a02521fdd")
if uuid not in data or b"GSpherical:ProjectionType=\"equirectangular\"" not in data:
    raise SystemExit("MP4 spherical UUID metadata missing")
if b"sv3d" not in data:
    raise SystemExit("MP4 sv3d box missing")
PY
test "$(field "$spatial" video spherical)" = "equirectangular"
# A segmented resume of the same scene keeps the spherical metadata (the
# segments carry none; the final mux adds both kinds).
"$binary" --scene "$root/tests/data-spatial.xml" --output "$work/spatial-resume.mp4" \
    --threads 1 --resume --segment-frames 1
test "$(field "$work/spatial-resume.mp4" video spherical)" = "equirectangular"
grep -q 'GSpherical:ProjectionType="equirectangular"' "$work/spatial-resume.mp4"
color="$(field "$spatial" video range),$(field "$spatial" video space),$(field "$spatial" video transfer),$(field "$spatial" video primaries)"
test "$color" = "tv,bt709,iec61966-2-1,bt709" || {
    echo "unexpected spatial color tags: $color" >&2
    exit 1
}

rec709="$work/rec709.mp4"
"$binary" --scene "$root/tests/data-rec709.xml" --output "$rec709" --threads 1
color="$(field "$rec709" video range),$(field "$rec709" video space),$(field "$rec709" video transfer),$(field "$rec709" video primaries)"
test "$color" = "tv,bt709,bt709,bt709" || {
    echo "unexpected Rec.709 color tags: $color" >&2
    exit 1
}

"$binary" --scene "$root/tests/data-ffv1.xml" \
    --output "$work/ffv1.mkv" --threads 1
test "$(field "$work/ffv1.mkv" video codec)" = "ffv1"

# PNG previews through libavcodec.
"$binary" --scene "$root/examples/keyframe-curves.xml" \
    --resolution 320x180 --preview-frame 30 --preview-out "$work/preview.png"
test "$(field "$work/preview.png" video codec)" = "png"
echo "integration tests passed"
