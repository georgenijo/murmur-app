#!/bin/bash
set -euo pipefail
binary="${1:?binary path required}"
output="${2:?output directory required}"
mkdir -p "$output"
output="$(cd "$output" && pwd)"
for n in 1 2; do
  P0E_LOG="$output/capture-$n.log" P0E_SCREENSHOT="$output/golden-$n.png" "$binary"
done
cmp "$output/golden-1.png" "$output/golden-2.png"
sips -g pixelWidth -g pixelHeight "$output/golden-1.png"
shasum -a 256 "$output"/golden-*.png
