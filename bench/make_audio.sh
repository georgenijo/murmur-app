#!/bin/bash
# Generate a reproducible local benchmark corpus with macOS system voices.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
AUDIO_DIR="$SCRIPT_DIR/audio"
VOICE="${MURMUR_BENCH_VOICE:-Samantha}"

# "fast" is synthesized at a high speaking rate to stress rapid delivery; the
# rest use the voice's default rate. The stress fixtures (jargon/numbers/
# disfluent/xxlong/fast) were added for issue #273 to de-saturate model ranking.
# NOTE: `say` output is unnaturally fluent — these clips stress vocabulary/ITN
# and word content, NOT real human mumble/hesitation acoustics.
synth() {
  local name="$1"
  shift
  local text="$AUDIO_DIR/$name.txt"
  local aiff="$AUDIO_DIR/$name.aiff"
  local wav="$AUDIO_DIR/$name.wav"
  say -v "$VOICE" "$@" -o "$aiff" -f "$text"
  afconvert -f WAVE -d LEI16@16000 -c 1 "$aiff" "$wav"
  rm -f "$aiff"
  afinfo "$wav" | awk -v name="$name" -F': ' '/estimated duration/ {print name ": " $2 "s"}'
}

for name in short medium long xlong jargon numbers disfluent xxlong; do
  synth "$name"
done
synth fast -r 280
