#!/bin/zsh
set -euo pipefail

SCRIPT_DIR="${0:A:h}"
REPO="${SCRIPT_DIR:h}"
APP="${1:-$REPO/app/crates/murmur-services/target/release/bundle/macos/Murmur Bench.app}"
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
  --main-entitlements app/macos/entitlements.plist \
  --capture-agent-entitlements app/macos/capture-agent.entitlements.plist \
  --capture-agent-info-plist app/macos/capture-agent-info.plist \
  --capture-helper-info-plist app/sidecars/capture/Info.plist \
  --capture-worker-info-plist app/sidecars/capture/WorkerInfo.plist \
  --capture-agent-launchd-plist app/macos/macos/com.localdictation.capture-agent.plist \
  --capture-helper-entitlements app/macos/capture-helper.entitlements.plist \
  --capture-worker-entitlements app/macos/capture-worker.entitlements.plist \
  --expected-team-id "$TEAM_ID"

print -- "[3/3] Verifying signed app"
codesign --verify --deep --strict --verbose=2 "$APP"
print -r -- "success:$APP" > "$STATUS_FILE"
print -- "SIGNED_APP=$APP"
