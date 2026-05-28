#!/usr/bin/env bash
# Show ffmpeg avfoundation audio input indices for record-test-wav.sh

set -euo pipefail

if ! command -v ffmpeg >/dev/null 2>&1; then
  echo "ffmpeg not found. Install with: brew install ffmpeg"
  exit 1
fi

echo "Audio devices (use index with MURMUR_MIC=…):"
echo ""
ffmpeg -f avfoundation -list_devices true -i "" 2>&1 \
  | awk '/AVFoundation audio devices:/{a=1;next} a && /\[/{print}'
