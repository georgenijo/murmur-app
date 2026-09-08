#!/bin/bash
set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo "usage: bash run_concurrent_probe.sh DIARIZATION.wav ASR.wav OUTPUT_DIR" >&2
  exit 2
fi

diarization_audio="$1"
asr_audio="$2"
output_dir="$3"
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
diarizer="$script_dir/target/release/murmur-issue-540-diarization-spike"
asr_probe="$script_dir/target/release/asr_probe"

mkdir -p "$output_dir"

/usr/bin/time -l "$asr_probe" "$asr_audio" 20 \
  > "$output_dir/asr-baseline.jsonl" \
  2> "$output_dir/asr-baseline.time.txt"

"$diarizer" "$diarization_audio" 0.7 1 \
  > "$output_dir/diarization-concurrent.jsonl" \
  2> "$output_dir/diarization-concurrent.stderr.txt" &
diarization_pid=$!

ready=false
for _ in {1..400}; do
  if grep -q '"kind":"initialization"' "$output_dir/diarization-concurrent.jsonl"; then
    ready=true
    break
  fi
  if ! kill -0 "$diarization_pid" 2>/dev/null; then
    break
  fi
  sleep 0.05
done
if [[ "$ready" != true ]]; then
  wait "$diarization_pid" || true
  echo "diarization did not reach its initialized state" >&2
  exit 1
fi

/usr/bin/time -l "$asr_probe" "$asr_audio" 20 \
  > "$output_dir/asr-concurrent.jsonl" \
  2> "$output_dir/asr-concurrent.time.txt"
wait "$diarization_pid"

find "$output_dir" -type f -print0 | sort -z | xargs -0 shasum -a 256
