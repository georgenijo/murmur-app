#!/bin/bash
# murmur-audio-triage.sh — READ-ONLY diagnostics for a Core Audio input hang.
#
# For the machine where Murmur gets stuck on "Connecting microphone":
#   1. Open Murmur and start a recording so it is stuck on "Connecting" RIGHT NOW
#      (this lets the script photograph the stuck thread — the key evidence).
#   2. Run this script in Terminal:  bash murmur-audio-triage.sh
#   3. It prints a path to a .zip at the end — send that file back.
#
# This script only LOOKS at things. It does not change settings, kill programs,
# restart audio, or ask for a password.
set -u
TS=$(date +%Y%m%d-%H%M%S)
OUTDIR="$HOME/Desktop/murmur-audio-triage-$TS"
mkdir -p "$OUTDIR"
exec > >(tee "$OUTDIR/report.txt") 2>&1

section() { printf '\n===== %s =====\n' "$1"; }

# macOS has no `timeout`; a hang inside any CoreAudio-touching probe is itself
# a finding, so every probe gets a watchdog.
with_timeout() {
  local secs=$1; shift
  "$@" &
  local pid=$!
  ( sleep "$secs" && kill -9 "$pid" 2>/dev/null ) &
  local watchdog=$!
  wait "$pid" 2>/dev/null
  local rc=$?
  kill "$watchdog" 2>/dev/null
  wait "$watchdog" 2>/dev/null
  if [ "$rc" -ge 128 ]; then
    echo "[TIMED OUT after ${secs}s — if this probe touches CoreAudio, the hang is system-wide, not Murmur-specific]"
  fi
  return "$rc"
}

echo "Murmur audio triage — read-only. Started $(date)."

section "SYSTEM"
sw_vers
sysctl -n hw.model machdep.cpu.brand_string 2>/dev/null
echo "uptime: $(uptime)"

section "COREAUDIOD PROCESS (macOS audio daemon)"
ps -axo pid,ppid,user,%cpu,%mem,state,lstart,etime,comm | head -1
ps -axo pid,ppid,user,%cpu,%mem,state,lstart,etime,comm | grep -i '[c]oreaudiod' || echo "coreaudiod NOT RUNNING (major finding)"
CA_PID=$(pgrep -x coreaudiod | head -1 || true)
if [ -n "${CA_PID:-}" ]; then
  echo "coreaudiod thread count: $(ps -M -p "$CA_PID" 2>/dev/null | tail -n +2 | wc -l | tr -d ' ')"
fi

section "MURMUR PROCESS + STUCK-THREAD SAMPLE"
MURMUR_PID=$(pgrep -x Murmur || pgrep -x localdictation || pgrep -f 'Murmur\.app/Contents/MacOS' || true)
MURMUR_PID=$(echo "$MURMUR_PID" | head -1)
if [ -n "${MURMUR_PID:-}" ]; then
  ps -o pid,state,%cpu,etime,comm -p "$MURMUR_PID"
  echo "Sampling Murmur for 3s (passive; shows where the audio thread is blocked)..."
  with_timeout 30 sample "$MURMUR_PID" 3 -file "$OUTDIR/murmur-sample.txt" >/dev/null
  if [ -s "$OUTDIR/murmur-sample.txt" ]; then
    echo "sample saved to murmur-sample.txt; audio-relevant excerpt:"
    grep -n -i -m 5 -A 25 'cpal\|coreaudio\|AudioObject\|AudioUnit\|AudioOutputUnit\|HALC\|HALB\|CAHAL' "$OUTDIR/murmur-sample.txt" | head -120 \
      || echo "(no CoreAudio frames found — Murmur may not be stuck in Connecting right now; re-run while it is)"
  fi
else
  echo "Murmur is not running — start it, get it stuck on Connecting, and re-run for the key evidence."
fi

section "THIRD-PARTY AUDIO (HAL) PLUGINS — classic causes of this exact hang"
for d in /Library/Audio/Plug-Ins/HAL "$HOME/Library/Audio/Plug-Ins/HAL"; do
  echo "-- $d"
  ls -la "$d" 2>/dev/null || echo "(empty or absent)"
done

section "KNOWN AUDIO-GRABBING SOFTWARE CURRENTLY RUNNING"
pgrep -ifl 'zoom|teams|webex|discord|krisp|loopback|audio hijack|soundsource|blackhole|obs|elgato|wave link|rogue' \
  || echo "(none of the usual suspects running)"

section "COREAUDIO LIVENESS PROBES (a timeout here = audio wedged for the whole system)"
echo "-- volume query (touches CoreAudio):"
with_timeout 10 osascript -e 'get volume settings'
echo "-- audio device list:"
with_timeout 30 system_profiler SPAudioDataType

section "COREAUDIOD ERRORS/FAULTS, LAST 3 HOURS (system log)"
with_timeout 180 log show --last 3h --style compact \
  --predicate 'process == "coreaudiod" AND messageType IN {16, 17}' 2>/dev/null | tail -80

section "MICROPHONE PERMISSION (TCC) EVENTS, LAST 3 HOURS"
with_timeout 180 log show --last 3h --style compact \
  --predicate 'subsystem == "com.apple.TCC" AND eventMessage CONTAINS[c] "microphone"' 2>/dev/null | tail -40

section "EXCLUSIVE-ACCESS (HOG MODE) MENTIONS, LAST 3 HOURS"
with_timeout 180 log show --last 3h --style compact \
  --predicate 'process == "coreaudiod" AND eventMessage CONTAINS[c] "hog"' 2>/dev/null | tail -20

section "MURMUR'S OWN RECENT LOG"
MLOG=$(find "$HOME/Library/Logs" "$HOME/Library/Application Support" -maxdepth 4 \
  \( -iname '*murmur*' -o -iname '*localdictation*' \) -type f 2>/dev/null \
  | xargs ls -t 2>/dev/null | head -1)
if [ -n "${MLOG:-}" ]; then
  echo "newest Murmur log file: $MLOG"
  tail -200 "$MLOG" > "$OUTDIR/murmur-app-log-tail.txt" 2>/dev/null && echo "last 200 lines saved to murmur-app-log-tail.txt"
else
  echo "(no Murmur log files found)"
fi

section "DONE"
ZIP="$HOME/Desktop/murmur-audio-triage-$TS.zip"
ditto -c -k --sequesterRsrc "$OUTDIR" "$ZIP" 2>/dev/null || zip -rq "$ZIP" "$OUTDIR"
echo ""
echo "All done. Nothing on this Mac was modified, killed, or restarted."
echo ">>> Please send back this file (it's on the Desktop): $ZIP"
