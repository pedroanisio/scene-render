#!/bin/sh
# usage: run-integration.sh ROOT BINARY WORKDIR
#   ROOT     repository root (read-only: nothing is written below it)
#   BINARY   scene-render executable under test
#   WORKDIR  directory that receives every output, cache and copied scene
set -eu

if [ $# -ne 3 ]; then
    echo "usage: $0 ROOT BINARY WORKDIR" >&2
    exit 2
fi
root=$(cd "$1" && pwd)
binary=$2
work=$3
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
probe=$(ffprobe -v error -select_streams v:0 \
    -show_entries stream=width,height,nb_frames -of csv=p=0 "$video")
test "$probe" = "320,180,12" || {
    echo "unexpected ffprobe result: $probe" >&2
    exit 1
}

media="$work/media.mp4"
"$binary" --scene "$root/tests/data-media.xml" --output "$media" --threads 1
video_codec=$(ffprobe -v error -select_streams v:0 \
    -show_entries stream=codec_name -of csv=p=0 "$media")
audio_codec=$(ffprobe -v error -select_streams a:0 \
    -show_entries stream=codec_name -of csv=p=0 "$media")
test "$video_codec" = "h264" && test "$audio_codec" = "aac" || {
    echo "unexpected media codecs: video=$video_codec audio=$audio_codec" >&2
    exit 1
}
python3 - "$media" <<'PY'
import json
import subprocess
import sys

probe = subprocess.run([
    "ffprobe", "-v", "error", "-show_entries", "stream=codec_type,duration",
    "-of", "json", sys.argv[1]], check=True, capture_output=True, text=True)
streams = {item["codec_type"]: float(item["duration"])
           for item in json.loads(probe.stdout)["streams"]}
if set(streams) != {"video", "audio"} or abs(streams["video"]-streams["audio"]) > 1/48000:
    raise SystemExit(f"A/V duration mismatch: {streams}")
PY

viewport_video="$work/viewport.mp4"
"$binary" --scene "$root/tests/data-viewport.xml" --output "$viewport_video" \
    --threads 4 --resume
"$binary" --scene "$root/tests/data-viewport.xml" --output "$viewport_video" \
    --threads 4 --resume
version=$("$binary" --version | awk '{print $2}')
manifest=$(find "$viewport_video.resume" -name manifest.txt -type f \
    -exec grep -l "scene-render=$version" {} \; | head -n 1)
test -n "$manifest"
cache_dir=$(dirname "$manifest")
test "$(find "$cache_dir" -name '*.rgba' | wc -l)" -eq 6

if ffmpeg -hide_banner -encoders 2>/dev/null | grep -q 'ffv1'; then
    "$binary" --scene "$root/tests/data-equirect.xml" \
        --output "$work/equirect.mkv" --threads 1
    dimensions=$(ffprobe -v error -select_streams v:0 \
        -show_entries stream=width,height -of csv=p=0 \
        "$work/equirect.mkv")
    test "$dimensions" = "256,128"
fi

if ffmpeg -hide_banner -encoders 2>/dev/null | grep -q 'libx265'; then
    "$binary" --scene "$root/tests/data-h265.xml" \
        --output "$work/h265.mp4" --threads 1
    codec=$(ffprobe -v error -select_streams v:0 \
        -show_entries stream=codec_name -of csv=p=0 \
        "$work/h265.mp4")
    test "$codec" = "hevc"
fi

spatial="$work/spatial.mp4"
"$binary" --scene "$root/tests/data-spatial.xml" --output "$spatial" --threads 1
python3 - "$spatial" <<'PY'
import pathlib
import sys

data = pathlib.Path(sys.argv[1]).read_bytes()
uuid = bytes.fromhex("ffcc8263f8554a938814587a02521fdd")
if uuid not in data or b"GSpherical:ProjectionType=\"equirectangular\"" not in data:
    raise SystemExit("MP4 spherical UUID metadata missing")
PY
color=$(ffprobe -v error -select_streams v:0 \
    -show_entries stream=color_range,color_space,color_transfer,color_primaries \
    -of csv=p=0 "$spatial")
test "$color" = "tv,bt709,iec61966-2-1,bt709" || {
    echo "unexpected spatial color tags: $color" >&2
    exit 1
}

rec709="$work/rec709.mp4"
"$binary" --scene "$root/tests/data-rec709.xml" --output "$rec709" --threads 1
color=$(ffprobe -v error -select_streams v:0 \
    -show_entries stream=color_range,color_space,color_transfer,color_primaries \
    -of csv=p=0 "$rec709")
test "$color" = "tv,bt709,bt709,bt709" || {
    echo "unexpected Rec.709 color tags: $color" >&2
    exit 1
}

if ffmpeg -hide_banner -encoders 2>/dev/null | grep -q 'ffv1'; then
    "$binary" --scene "$root/tests/data-ffv1.xml" \
        --output "$work/ffv1.mkv" --threads 1
    codec=$(ffprobe -v error -select_streams v:0 \
        -show_entries stream=codec_name -of csv=p=0 \
        "$work/ffv1.mkv")
    test "$codec" = "ffv1"
fi
echo "integration tests passed"
