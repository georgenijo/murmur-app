#!/usr/bin/env bash
set -euo pipefail

log_dir="${HOME}/Library/Application Support/local-dictation/logs"
mkdir -p "${log_dir}"

stamp="$(date +%Y%m%d-%H%M%S)"
out="${log_dir}/macos-audio-${stamp}.log"

cat <<EOF
Capturing macOS audio diagnostics to:
  ${out}

Reproduce the ducking issue now. Press Ctrl-C to stop.
EOF

predicate='process == "coreaudiod" OR process == "Murmur" OR process == "Local Dictation Dev" OR subsystem CONTAINS[c] "audio" OR category CONTAINS[c] "audio" OR eventMessage CONTAINS[c] "duck" OR eventMessage CONTAINS[c] "volume" OR eventMessage CONTAINS[c] "mute" OR eventMessage CONTAINS[c] "HAL" OR eventMessage CONTAINS[c] "AudioDevice" OR eventMessage CONTAINS[c] "AudioSession" OR eventMessage CONTAINS[c] "route"'

log stream --style compact --info --predicate "${predicate}" | tee "${out}"
