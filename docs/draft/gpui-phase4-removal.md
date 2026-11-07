# GPUI Phase 4 removal (#1007)

Part of #1012. Draft stacked on `gpui/26-ui-parity`; never merge this layer alone.

## Ownership

- `app/src-tauri/src`, tests, examples and evaluation fixtures move with `git mv`
  to `app/crates/murmur-services`. The Tauri host/runtime, IPC attributes,
  generated handler/plugin layer, permissions/capabilities and tao patch are removed.
- Core, capture protocol and permanent standalone importer move to sibling
  crates in `app/crates`. Services have no Tauri, tao, wry, webview or importer
  dependency, including dev/build/optional/target edges.
- `app/gpui` joins `app/Cargo.toml` with one native lockfile/toolchain. Capture
  keeps a separate workspace/lockfile because its CPAL native-library graph
  conflicts with GPUI's optional audio version during resolution.
- Capture helper/worker and Swift agent move to `app/sidecars`; metadata,
  entitlements and icons move to `app/macos`. The bundle manifest owns version,
  identity and updater key; the independent frozen identity snapshot is retained.
- Rust-loaded overlay/system-audio fixtures move from React to `tests/fixtures`.
  Rust already owns settings, appearance and palette schemas/fixtures. GPUI's
  committed theme-resolution fixtures remain; its Node generator is retired.
- React `app/src`, public bootstrap, visual suites, Node/Vite/Vitest/Playwright
  configuration, generated Tauri schemas and runtime configuration are deleted.
  GPUI interaction/AX tests and repeated offscreen Metal renders cover native UI.

## Permanent migration

The standalone objc2 importer remains buildable. The GPUI host's exact-child
worker remains its production transport. The parent checks `retired` before any
worker spawn or WebKit creation. After import and independently receipted exact-key
removal, a durable prepare receipt authorizes broad store cleanup. The worker
releases the view, purges all types, fetches/verifies zero records and exits.
Only then does the parent remove the exclusive bundle directory, sync its parent,
and commit retirement. Failure before the final marker retries; committed
imports cannot replay cleared History. Symlink directory replacement is refused.

## Invariants and audit

The shared algorithms retain start snapshots, recording generation/continuation
checks, exactly-once completion, retention/Clear/Delete/export gates, ordered
transforms, secure-field refusal, queued main-thread windows, rdev setup and
shutdown drain owners. Content-safe diagnostics and private QA pasteboards stay.
The termination-token/Clippy guard and all-target dependency policy remain in CI.

The requested source grep includes harmless language vocabulary and synthetic
transcription fixtures naming the retired framework; removing those terms would
change dictation behavior. Other remaining literals describe legacy origin,
updater compatibility, path parity and pre-flip schema fixtures. They are not
runtime dependencies. Actual WebKit API references are confined to the permanent
importer/retirement code (and its synthetic tests); the UI uses dynamic loading
only inside its disposable migration worker. No UI framework dependency remains
in the service/core dependency graphs.

## Verification

Results will be recorded in the PR after the changed revision is exercised.
All native runs use synthetic identities/HOME and exact owned PIDs. VoiceOver
remains on; production application/data/WebKit/pasteboard are untouched.

## Deferred acceptance

No release is dispatched or published. Signed installed upgrade-chain rehearsal
(stepwise and direct 0.62 → removal release → next native), TCC carryover and the
final empty-record/directory proof on those installed builds remain manual.
The final memory-meets-frozen-budgets exit criterion is pending an explicitly
requested measurement run. No benchmarks run for this implementation.
Independent Sol review remains with the parent/reviewer: this implementation
worker is expressly prohibited from starting subagents.
