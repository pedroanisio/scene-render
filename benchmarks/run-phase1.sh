#!/bin/sh
set -eu

root=${1:-.}
binary=${2:-$root/build/scene-render}
output=${3:-$root/build/benchmark-10s-4k.mp4}
log=${4:-$root/build/benchmark-10s-4k.log}

mkdir -p "$(dirname "$output")" "$(dirname "$log")"
"$binary" \
    --scene "$root/examples/ten-layer-composition.xml" \
    --output "$output" --threads 1 --quality low --metrics \
    2>"$log"

ffprobe -v error -select_streams v:0 \
    -show_entries stream=codec_name,width,height,pix_fmt,r_frame_rate,nb_frames \
    -show_entries format=duration,size \
    -of default=noprint_wrappers=1 "$output"
sha256sum "$output"
echo "detailed timing: $log"
