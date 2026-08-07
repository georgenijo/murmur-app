#!/bin/zsh
set -euo pipefail

SCRIPT_DIR="${0:A:h}"
REPO="${SCRIPT_DIR:h}"
APP="${1:-$REPO/app/src-tauri/target/release/bundle/macos/Murmur Bench.app}"
IDENTITY="${MURMUR_SIGNING_IDENTITY:-Developer ID Application: George Nijo (P2U3P8B923)}"
TEAM_ID="${MURMUR_SIGNING_TEAM_ID:-P2U3P8B923}"
STATUS_FILE="/tmp/murmur-bench-sign.status"
LOG_FILE="/tmp/murmur-bench-sign.log"

exec > >(tee "$LOG_FILE") 2>&1
trap 'rc=$?; if (( rc != 0 )); then print -r -- "failed:$rc" > "$STATUS_FILE"; fi' EXIT

rm -f "$STATUS_FILE"
cd "$REPO"

print -- "[1/3] Checking Developer ID identity"
security find-identity -v -p codesigning | grep -F "$IDENTITY" >/dev/null

print -- "[2/3] Signing helpers and app"
python3 scripts/finalize_macos_bundle.py \
  --app "$APP" \
  --identity "$IDENTITY" \
  --main-entitlements app/src-tauri/entitlements.plist \
  --llm-helper-entitlements app/src-tauri/local-llm-sidecar.entitlements.plist \
  --capture-agent-entitlements app/src-tauri/capture-agent.entitlements.plist \
  --capture-agent-info-plist app/src-tauri/capture-agent-info.plist \
  --capture-helper-info-plist app/src-tauri/sidecars/capture/Info.plist \
  --capture-worker-info-plist app/src-tauri/sidecars/capture/WorkerInfo.plist \
  --capture-agent-launchd-plist app/src-tauri/macos/com.localdictation.capture-agent.plist \
  --capture-helper-entitlements app/src-tauri/capture-helper.entitlements.plist \
  --capture-worker-entitlements app/src-tauri/capture-worker.entitlements.plist \
  --expected-team-id "$TEAM_ID"

print -- "[3/3] Verifying signed app"
codesign --verify --deep --strict --verbose=2 "$APP"
print -r -- "success:$APP" > "$STATUS_FILE"
print -- "SIGNED_APP=$APP"
