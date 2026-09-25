#!/bin/sh
set -eu

root=${1:-.}
binary=${2:-$root/build/scene-render}
mkdir -p "$root/build/test-artifacts"

for scene in "$root"/examples/*.xml; do
    "$binary" --scene "$scene" --validate
done
"$binary" --scene "$root/examples/basic-multilayer.xml" \
    --renderer gpu --validate

python3 - "$root" <<'PY'
import pathlib
import sys
from lxml import etree

root = pathlib.Path(sys.argv[1])
schema = etree.XMLSchema(etree.parse(str(root / "schema/scene-v1.xsd")))
for path in sorted((root / "examples").glob("*.xml")):
    document = etree.parse(str(path))
    if not schema.validate(document):
        raise SystemExit(f"XSD failure in {path}: {schema.error_log.last_error}")
print("XSD validation passed")
PY

first="$root/build/test-artifacts/golden-a.ppm"
second="$root/build/test-artifacts/golden-b.ppm"
"$binary" --scene "$root/examples/keyframe-curves.xml" \
    --resolution 320x180 --preview-frame 30 --preview-out "$first"
"$binary" --scene "$root/examples/keyframe-curves.xml" \
    --resolution 320x180 --preview-frame 30 --preview-out "$second"
cmp "$first" "$second"
actual=$(sha256sum "$first" | awk '{print $1}')
expected=$(awk '$2 == "keyframe" {print $1}' "$root/tests/golden.sha256")
test "$actual" = "$expected" || {
    echo "golden mismatch: expected $expected, got $actual" >&2
    exit 1
}

viewport_a="$root/build/test-artifacts/viewport-a.ppm"
viewport_b="$root/build/test-artifacts/viewport-b.ppm"
"$binary" --scene "$root/tests/data-viewport.xml" --preview-frame 3 \
    --preview-out "$viewport_a" --threads 1
"$binary" --scene "$root/tests/data-viewport.xml" --preview-frame 3 \
    --preview-out "$viewport_b" --threads 4
cmp "$viewport_a" "$viewport_b"
actual=$(sha256sum "$viewport_a" | awk '{print $1}')
expected=$(awk '$2 == "viewport" {print $1}' "$root/tests/golden.sha256")
test "$actual" = "$expected" || {
    echo "viewport golden mismatch: expected $expected, got $actual" >&2
    exit 1
}

advanced_a="$root/build/test-artifacts/advanced-a.ppm"
advanced_b="$root/build/test-artifacts/advanced-b.ppm"
rm -f "$root/build/test-artifacts/advanced.physics"
"$binary" --scene "$root/tests/data-advanced.xml" --preview-frame 8 \
    --preview-out "$advanced_a"
test -s "$root/build/test-artifacts/advanced.physics"
"$binary" --scene "$root/tests/data-advanced.xml" --preview-frame 8 \
    --preview-out "$advanced_b"
cmp "$advanced_a" "$advanced_b"
actual=$(sha256sum "$advanced_a" | awk '{print $1}')
expected=$(awk '$2 == "advanced" {print $1}' "$root/tests/golden.sha256")
test "$actual" = "$expected" || {
    echo "advanced golden mismatch: expected $expected, got $actual" >&2
    exit 1
}

video="$root/build/test-artifacts/integration.mp4"
"$binary" --scene "$root/examples/keyframe-curves.xml" --resolution 320x180 \
    --fps 12 --frame-range 0:12 --output "$video" --threads 1 --quality low
probe=$(ffprobe -v error -select_streams v:0 \
    -show_entries stream=width,height,nb_frames -of csv=p=0 "$video")
test "$probe" = "320,180,12" || {
    echo "unexpected ffprobe result: $probe" >&2
    exit 1
}

media="$root/build/test-artifacts/media.mp4"
"$binary" --scene "$root/tests/data-media.xml" --output "$media" --threads 1
video_codec=$(ffprobe -v error -select_streams v:0 \
    -show_entries stream=codec_name -of csv=p=0 "$media")
audio_codec=$(ffprobe -v error -select_streams a:0 \
    -show_entries stream=codec_name -of csv=p=0 "$media")
test "$video_codec" = "h264" && test "$audio_codec" = "aac" || {
    echo "unexpected media codecs: video=$video_codec audio=$audio_codec" >&2
    exit 1
}

viewport_video="$root/build/test-artifacts/viewport.mp4"
"$binary" --scene "$root/tests/data-viewport.xml" --output "$viewport_video" \
    --threads 4 --resume
"$binary" --scene "$root/tests/data-viewport.xml" --output "$viewport_video" \
    --threads 4 --resume
test "$(find "$viewport_video.resume" -name '*.rgba' | wc -l)" -eq 6

if ffmpeg -hide_banner -encoders 2>/dev/null | grep -q 'ffv1'; then
    "$binary" --scene "$root/tests/data-equirect.xml" \
        --output "$root/build/test-artifacts/equirect.mkv" --threads 1
    dimensions=$(ffprobe -v error -select_streams v:0 \
        -show_entries stream=width,height -of csv=p=0 \
        "$root/build/test-artifacts/equirect.mkv")
    test "$dimensions" = "256,128"
fi

if ffmpeg -hide_banner -encoders 2>/dev/null | grep -q 'libx265'; then
    "$binary" --scene "$root/tests/data-h265.xml" \
        --output "$root/build/test-artifacts/h265.mp4" --threads 1
    codec=$(ffprobe -v error -select_streams v:0 \
        -show_entries stream=codec_name -of csv=p=0 \
        "$root/build/test-artifacts/h265.mp4")
    test "$codec" = "hevc"
fi

if ffmpeg -hide_banner -encoders 2>/dev/null | grep -q 'ffv1'; then
    "$binary" --scene "$root/tests/data-ffv1.xml" \
        --output "$root/build/test-artifacts/ffv1.mkv" --threads 1
    codec=$(ffprobe -v error -select_streams v:0 \
        -show_entries stream=codec_name -of csv=p=0 \
        "$root/build/test-artifacts/ffv1.mkv")
    test "$codec" = "ffv1"
fi
echo "integration tests passed"
