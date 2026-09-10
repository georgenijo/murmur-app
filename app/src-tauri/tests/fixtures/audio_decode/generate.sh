#!/bin/sh
set -eu

fixture_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
ffmpeg_bin=${FFMPEG_BIN:-/opt/homebrew/bin/ffmpeg}

if [ ! -x "$ffmpeg_bin" ]; then
  echo "ffmpeg not found at $ffmpeg_bin; set FFMPEG_BIN to regenerate fixtures" >&2
  exit 1
fi

video_source="testsrc2=size=16x16:rate=5:duration=0.25"
audio_source="sine=frequency=440:sample_rate=16000:duration=0.25"

for extension in mp4 mov; do
  "$ffmpeg_bin" -hide_banner -loglevel error -y \
    -f lavfi -i "$video_source" \
    -f lavfi -i "$audio_source" \
    -map 0:v -map 1:a \
    -c:v libx264 -preset ultrafast -crf 35 -pix_fmt yuv420p \
    -c:a aac -b:a 24k -metadata encoder= -movflags +faststart \
    "$fixture_dir/video-first-aac.$extension"
done

"$ffmpeg_bin" -hide_banner -loglevel error -y \
  -f lavfi -i "$video_source" \
  -map 0:v -c:v libx264 -preset ultrafast -crf 35 -pix_fmt yuv420p \
  -metadata encoder= -movflags +faststart \
  "$fixture_dir/video-only.mp4"

"$ffmpeg_bin" -hide_banner -loglevel error -y \
  -f lavfi -i "$audio_source" \
  -map 0:a -c:a libopus -b:a 24k -strict experimental \
  -metadata encoder= -movflags +faststart \
  "$fixture_dir/unsupported-opus.mp4"

"$ffmpeg_bin" -hide_banner -loglevel error -y \
  -f lavfi -i "sine=frequency=440:sample_rate=48000:duration=0.25" \
  -map 0:a -c:a ac3 -b:a 32k \
  -metadata encoder= -movflags +faststart \
  "$fixture_dir/unsupported-ac3.mp4"

"$ffmpeg_bin" -hide_banner -loglevel error -y \
  -f lavfi -i "sine=frequency=330:sample_rate=16000:duration=0.25" \
  -f lavfi -i "sine=frequency=660:sample_rate=16000:duration=0.25" \
  -map 0:a -map 1:a \
  -c:a:0 libopus -b:a:0 24k -strict experimental \
  -c:a:1 aac -b:a:1 24k \
  -metadata encoder= -movflags +faststart \
  "$fixture_dir/unsupported-first-aac-second.mp4"
