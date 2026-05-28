#!/usr/bin/env bash
# Record a Murmur-compatible test WAV (16 kHz mono 16-bit PCM).
# Re-run anytime; overwrites the same file.
#
# Usage:
#   ./scripts/record-test-wav.sh
#
# Optional env:
#   MURMUR_WAV=~/Desktop/murmur-test.wav   output path
#   MURMUR_MIC=2                           avfoundation audio device index
#   MURMUR_DURATION=5                      seconds to record
#
# List mics: ./scripts/list-mics.sh

set -euo pipefail

OUT="${MURMUR_WAV:-$HOME/Desktop/murmur-test.wav}"
MIC="${MURMUR_MIC:-2}"
DUR="${MURMUR_DURATION:-5}"

if ! command -v ffmpeg >/dev/null 2>&1; then
  echo "ffmpeg not found. Install with: brew install ffmpeg"
  exit 1
fi

mkdir -p "$(dirname "$OUT")"

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  Murmur test recording"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  File:     $OUT"
echo "  Mic:      avfoundation audio device :$MIC"
echo "  Duration: ${DUR}s"
echo ""
echo "  Wrong mic? Run: ./scripts/list-mics.sh"
echo "  Then: MURMUR_MIC=<index> ./scripts/record-test-wav.sh"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""
read -r -p "Press Enter when ready to record (Ctrl+C to quit)… " _

for i in 3 2 1; do
  printf '\rStarting in %s… ' "$i"
  sleep 1
done
printf '\r\033[K'
echo "🔴  SPEAK NOW (${DUR}s)"
echo ""

ffmpeg -loglevel warning -y \
  -f avfoundation -i ":$MIC" \
  -t "$DUR" \
  -ar 16000 -ac 1 \
  -c:a pcm_s16le \
  "$OUT"

echo ""
echo "✅  Saved: $OUT"
ffprobe -hide_banner "$OUT" 2>&1 | grep -E "Duration|Stream #0" || true
echo ""
echo "Run the same command again to re-record (overwrites)."
