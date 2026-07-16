#!/bin/bash
# Generate a reproducible local benchmark corpus with macOS system voices.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
AUDIO_DIR="$SCRIPT_DIR/audio"
VOICE="${MURMUR_BENCH_VOICE:-Samantha}"

for name in short medium long xlong; do
  text="$AUDIO_DIR/$name.txt"
  aiff="$AUDIO_DIR/$name.aiff"
  wav="$AUDIO_DIR/$name.wav"
  say -v "$VOICE" -o "$aiff" -f "$text"
  afconvert -f WAVE -d LEI16@16000 -c 1 "$aiff" "$wav"
  rm -f "$aiff"
  afinfo "$wav" | awk -v name="$name" -F': ' '/estimated duration/ {print name ": " $2 "s"}'
done
