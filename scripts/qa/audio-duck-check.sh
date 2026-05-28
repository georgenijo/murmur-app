#!/usr/bin/env bash
#
# QA: audio-ducking regression check (issue #177).
#
# Detects whether bringing an app to the foreground ducks OTHER system audio.
# Taps the system output level (scripts/qa/rms-meter.swift), plays nothing of its
# own, and scripts a surface/leave cycle on a target app while measuring the level.
#
# REQUIRES (cannot be automated in CI — hence a manual QA check, not a unit test):
#   - macOS 14.4+ (CoreAudio process-tap API)
#   - Audio actively playing through the default output (e.g. a video/music in
#     another app) for the whole run. The check measures THAT audio.
#   - One-time "audio recording" permission for the terminal (first run prompts).
#
# Usage:
#   scripts/qa/audio-duck-check.sh                 # target the running Murmur dev app
#   scripts/qa/audio-duck-check.sh "Safari"        # target an app by name
#   scripts/qa/audio-duck-check.sh --pid 12345     # target a specific pid
#
# Exit 0 = PASS (no duck), 1 = FAIL (duck detected), 2 = setup error.

set -euo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
METER_SRC="$DIR/rms-meter.swift"
METER_BIN="$DIR/rms-meter"
DUCK_DB=15          # surfaced level must not drop more than this below baseline
RMS_LOG="$(mktemp -t rmsqa).log"
ACT_LOG="$(mktemp -t rmsqa_act).log"

# --- resolve target ---
APP_NAME=""; APP_PID=""
if [[ "${1:-}" == "--pid" ]]; then
  APP_PID="${2:-}"
elif [[ -n "${1:-}" ]]; then
  APP_NAME="$1"
else
  APP_PID="$(pgrep -f 'target/debug/ui' | head -1 || true)"
  [[ -z "$APP_PID" ]] && { echo "No running Murmur dev app (target/debug/ui). Start it, or pass an app name."; exit 2; }
fi

surface() {  # bring target to front
  if [[ -n "$APP_PID" ]]; then
    osascript -e "tell application \"System Events\" to set frontmost of (first process whose unix id is ${APP_PID}) to true" 2>/dev/null || true
  else
    osascript -e "tell application \"${APP_NAME}\" to activate" 2>/dev/null || true
  fi
}
leave() { osascript -e 'tell application "Finder" to activate' 2>/dev/null || true; }

# --- build meter if needed ---
if [[ ! -x "$METER_BIN" || "$METER_SRC" -nt "$METER_BIN" ]]; then
  echo "Building rms-meter..."
  swiftc -O "$METER_SRC" -o "$METER_BIN" \
    -framework CoreAudio -framework AVFoundation -framework Foundation \
    || { echo "swiftc build failed"; exit 2; }
fi

echo "Target: ${APP_NAME:-pid $APP_PID}. Ensure other audio is PLAYING. Measuring ~24s..."
: > "$ACT_LOG"
"$METER_BIN" > "$RMS_LOG" 2>/dev/null &
METER=$!
trap 'kill -INT $METER 2>/dev/null || true' EXIT

sleep 5
echo "$(date +%H:%M:%S) SURFACE_1" >> "$ACT_LOG"; surface
sleep 5
echo "$(date +%H:%M:%S) LEAVE_1"   >> "$ACT_LOG"; leave
sleep 5
echo "$(date +%H:%M:%S) SURFACE_2" >> "$ACT_LOG"; surface
sleep 5
echo "$(date +%H:%M:%S) LEAVE_2"   >> "$ACT_LOG"; leave
sleep 4
kill -INT $METER 2>/dev/null || true; wait $METER 2>/dev/null || true

# --- analyze: mean dB while surfaced vs while not (baseline) ---
# A surfaced window = [SURFACE_n, SURFACE_n+5s); baseline = the rest of the run.
awk -v actfile="$ACT_LOG" '
  BEGIN {
    while ((getline line < actfile) > 0) {
      n=split(line, a, " "); split(a[1], t, ":"); sec=t[1]*3600+t[2]*60+t[3];
      if (a[2] ~ /SURFACE/) { s[++ns]=sec }
    }
  }
  {
    split(substr($1,1,8), t, ":"); sec=t[1]*3600+t[2]*60+t[3];
    db=$3; sub("db=","",db);
    surfaced=0; for (i=1;i<=ns;i++) if (sec>=s[i] && sec<s[i]+5) surfaced=1;
    if (surfaced) { sf+=db; nf++ } else { bg+=db; nb++ }
  }
  END {
    if (nf==0 || nb==0) { print "INCONCLUSIVE (no samples)"; exit 2 }
    fa=sf/nf; ba=bg/nb; drop=ba-fa;
    printf "baseline=%.1f dB  surfaced=%.1f dB  drop=%.1f dB\n", ba, fa, drop;
    if (drop > '"$DUCK_DB"') { print "FAIL: audio ducked while target was foreground (issue #177 regression)"; exit 1 }
    print "PASS: no ducking on foreground"; exit 0
  }
' "$RMS_LOG"
