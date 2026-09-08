#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
app_root="$repo_root/app"

cargo fmt --manifest-path "$app_root/src-tauri/Cargo.toml" --check
cargo check --manifest-path "$app_root/src-tauri/Cargo.toml" --lib

swift_runtime="/Library/Developer/CommandLineTools/usr/lib/swift-5.5/macosx"
if [[ -f "$swift_runtime/libswift_Concurrency.dylib" ]]; then
  export DYLD_LIBRARY_PATH="$swift_runtime${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}"
fi

cargo test --manifest-path "$app_root/src-tauri/Cargo.toml" 'dictation_' -- --test-threads=1
cargo test --manifest-path "$app_root/src-tauri/Cargo.toml" \
  private_upload_and_first_tick_share_one_persisted_install_id -- --test-threads=1

(
  cd "$app_root"
  npx tsc --noEmit
  npx vitest run \
    src/lib/dictationDiagnostics.test.ts \
    src/components/log-viewer/DictationDiagnosticsView.test.tsx \
    src/components/log-viewer/DiagnosticsWorkspace.test.tsx
  npm run build
)

(
  cd "$repo_root"
  python3 -m unittest tests.test_log_receiver.LogReceiverExportRouteTests
)
