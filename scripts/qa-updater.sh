#!/usr/bin/env bash
# qa-updater.sh — End-to-end QA test for the OTA auto-updater.
#
# Spins up a local update server claiming version 99.0.0 so the running
# app always sees a pending update and shows the native dialog.
#
# Usage: bash scripts/qa-updater.sh
#
# What it does:
#   1. Generates a signing keypair (once, at ~/.tauri/murmur-test.key)
#   2. Patches tauri.conf.json with the real pubkey + localhost endpoint
#   3. Builds a signed release app
#   4. Serves a fake latest.json on http://localhost:8080
#   5. Opens the app — update dialog should appear on launch
#   6. On Ctrl-C: kills the server and restores tauri.conf.json

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
UI_DIR="$REPO_ROOT/ui"
TAURI_DIR="$UI_DIR/src-tauri"
CONFIG="$TAURI_DIR/tauri.conf.json"
CONFIG_BACKUP="${CONFIG}.qa-backup"
KEY_FILE="$HOME/.tauri/murmur-test.key"
SERVER_DIR="/tmp/murmur-update-server"
PORT=8080

# ── colours ──────────────────────────────────────────────────────────────────
BOLD='\033[1m'; GREEN='\033[0;32m'; YELLOW='\033[0;33m'; RESET='\033[0m'
step()  { echo; echo -e "${BOLD}▶ $*${RESET}"; }
ok()    { echo -e "  ${GREEN}✓${RESET} $*"; }
warn()  { echo -e "  ${YELLOW}!${RESET} $*"; }

# ── cleanup (runs on exit / Ctrl-C) ──────────────────────────────────────────
cleanup() {
  echo
  step "Cleaning up"
  if [ -n "${SERVER_PID:-}" ] && kill -0 "$SERVER_PID" 2>/dev/null; then
    kill "$SERVER_PID"
    ok "HTTP server stopped"
  fi
  if [ -f "$CONFIG_BACKUP" ]; then
    mv "$CONFIG_BACKUP" "$CONFIG"
    ok "tauri.conf.json restored"
  fi
}
trap cleanup EXIT INT TERM

# ── step 1: keypair ───────────────────────────────────────────────────────────
step "Signing keypair"
if [ -f "$KEY_FILE" ]; then
  ok "Reusing existing keypair at $KEY_FILE"
else
  mkdir -p "$(dirname "$KEY_FILE")"
  warn "Generating new keypair at $KEY_FILE (one-time)"
  (cd "$UI_DIR" && npm run tauri -- signer generate -w "$KEY_FILE")
  ok "Keypair generated"
fi

PUBKEY="$(cat "${KEY_FILE}.pub")"
PRIVATE_KEY="$(cat "$KEY_FILE")"

# ── step 2: patch tauri.conf.json ────────────────────────────────────────────
step "Patching tauri.conf.json"
cp "$CONFIG" "$CONFIG_BACKUP"

python3 - "$CONFIG" "$PORT" <<PYEOF
import sys, json
path, port = sys.argv[1], int(sys.argv[2])
pubkey = open("${KEY_FILE}.pub").read().strip()
with open(path) as f:
    cfg = json.load(f)
cfg["plugins"]["updater"]["pubkey"] = pubkey
cfg["plugins"]["updater"]["endpoints"] = [f"http://localhost:{port}/latest.json"]
with open(path, "w") as f:
    json.dump(cfg, f, indent=2)
    f.write("\n")
PYEOF

ok "pubkey + localhost endpoint written"

# ── step 3: build ─────────────────────────────────────────────────────────────
step "Building signed release app (takes a few minutes)"
(cd "$UI_DIR" && TAURI_SIGNING_PRIVATE_KEY="$PRIVATE_KEY" npm run tauri build)
ok "Build complete"

# ── step 4: locate artifacts ──────────────────────────────────────────────────
step "Locating signed artifacts"
TARBALL="$(find "$TAURI_DIR/target/release/bundle/macos" -name "*.app.tar.gz" | head -1)"
SIG_FILE="${TARBALL}.sig"
APP_BUNDLE="$(find "$TAURI_DIR/target/release/bundle/macos" -maxdepth 1 -name "*.app" | head -1)"
GEN_BIN="$TAURI_DIR/target/release/gen_latest_json"

if [ -z "$TARBALL" ] || [ ! -f "$SIG_FILE" ]; then
  echo "ERROR: .app.tar.gz or .sig not found — was the build signed?" >&2
  echo "  (TAURI_SIGNING_PRIVATE_KEY must be set and TAURI_SIGNING_PRIVATE_KEY content valid)" >&2
  exit 1
fi

TARBALL_NAME="$(basename "$TARBALL")"
# URL-encode spaces in the filename
TARBALL_URL_NAME="$(python3 -c "import urllib.parse, sys; print(urllib.parse.quote(sys.argv[1]))" "$TARBALL_NAME")"
SIG="$(cat "$SIG_FILE")"
ok "Tarball: $TARBALL_NAME"
ok "Signature: ${SIG:0:30}…"

# ── step 5: generate latest.json and start server ────────────────────────────
step "Starting update server on http://localhost:$PORT"
mkdir -p "$SERVER_DIR"
cp "$TARBALL" "$SERVER_DIR/"

"$GEN_BIN" \
  "99.0.0" \
  "$(date -u +"%Y-%m-%dT%H:%M:%SZ")" \
  "$SIG" \
  "http://localhost:$PORT/$TARBALL_URL_NAME" \
  "QA test — intentional fake 99.0.0 to trigger the update dialog" \
  > "$SERVER_DIR/latest.json"

ok "latest.json written:"
python3 -m json.tool "$SERVER_DIR/latest.json" | sed 's/^/    /'

python3 -m http.server "$PORT" --directory "$SERVER_DIR" \
  > /tmp/murmur-update-server.log 2>&1 &
SERVER_PID=$!
sleep 1

if ! kill -0 "$SERVER_PID" 2>/dev/null; then
  echo "ERROR: HTTP server failed to start — is port $PORT already in use?" >&2
  exit 1
fi
ok "Server running (PID $SERVER_PID, log: /tmp/murmur-update-server.log)"

# ── step 6: open the app ──────────────────────────────────────────────────────
step "Opening app"
open "$APP_BUNDLE"
ok "App launched: $(basename "$APP_BUNDLE")"

echo
echo -e "${BOLD}────────────────────────────────────────────────${RESET}"
echo    "  The update dialog should appear within a few"
echo    "  seconds of the app launching."
echo
echo    "  Expected flow:"
echo    "    1. Native dialog: 'Update to 99.0.0 available'"
echo    "    2. Click Install → app downloads, relaunches"
echo    "    3. After relaunch: no dialog (already at 99.0.0)"
echo
echo    "  To test offline failure:"
echo    "    Ctrl-C now (stops server), then reopen the app —"
echo    "    should launch normally with no crash."
echo -e "${BOLD}────────────────────────────────────────────────${RESET}"
echo    "  Press Ctrl-C to stop the server and restore config."
echo

# keep alive so trap fires on Ctrl-C
wait "$SERVER_PID" 2>/dev/null || true
