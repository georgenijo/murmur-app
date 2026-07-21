# Selected-text transform

Issue [#312](https://github.com/georgenijo/murmur-app/issues/312). Hold a dedicated transform shortcut while text is selected in another app, speak an instruction (“make this shorter” or a preset name), review a local LLM proposal in a non-focusable popover, then Approve to replace the selection or Undo to restore. Entirely on-device, fail-closed, never auto-applies.

Binding runtime ADR: [docs/decisions/2026-07-20-signed-local-llm-sidecar.md](../decisions/2026-07-20-signed-local-llm-sidecar.md).

## Flow

1. User selects text in a third-party app and holds the transform key (`alt_r` / `ctrl_l` / `shift_r`).
2. Host freezes an AX selection snapshot (secure fields refused), shows the review popover in **listening**, arms mic for the instruction only.
3. On release (≥300ms hold): stop mic → cleanup-only ASR on the instruction → expand built-in preset or saved transform name if matched → signed local-LLM sidecar proposes a rewrite.
4. Popover shows **thinking** then **ready** (word diff). Events carry `{ state, errorCode }` only; instruction / original / proposed text are pulled via `get_transform_review_content`.
5. **Approve** writes through `transform_apply` (AX set-value or paste fallback with clipboard restore). **Undo** restores the frozen original. Esc / short-tap cancels at every stage.

Dictation and transform are mutually exclusive (status guards both ways + sidecar busy + helper shutdown before recording).

## Model storage

- Catalog pin: Qwen2.5-1.5B-Instruct Q4_K_M (~1.1 GB), exact size + SHA-256 enforced at download and before spawn.
- Path: `~/Library/Application Support/local-dictation/models/transform-llm/<sha256>/qwen2.5-1.5b-instruct-q4_k_m.gguf`
- Download streams to a `.partial` file, hashes while streaming, fsyncs, then atomically publishes under the hash directory.
- **Remove** deletes the hash directory (and any partial). The helper is shut down first so the file is not open.
- **Reset runtime** clears the circuit breaker after repeated helper faults (`reset_transform_runtime`).
- Apple Silicon + macOS only; other platforms report unsupported.

## Permissions

- **Accessibility** is required for: transform-key listening (rdev), AX selection capture, and AX write-back / paste-fallback frontmost checks.
- **Microphone** is required for instruction ASR (same device as dictation settings).
- Secure/password fields: AXSubrole checked **before** any value read; AX errors during the check fail closed. No popover content; optional overlay flash only.

## Supported vs best-effort apps

| Class | Examples | Expected behavior |
|-------|----------|-------------------|
| AX-native text | Notes, Mail compose, TextEdit, many Cocoa fields | Selection capture + AX set-value preferred; undo restores original |
| Best-effort (paste fallback) | Many browsers, Slack, Electron shells | Capture often works; apply may paste-replace; clipboard is saved/restored with an epoch guard |
| Documented limitation | Cursor chat / other webview editors | Best-effort only — webview accessibility is incomplete. Not a blocker for #312; file follow-ups rather than scope-creeping AX work |

## Failure semantics

- Any sidecar crash, timeout, protocol violation, model mismatch, or malformed output → popover **failed**, original selection **untouched**, dictation unaffected once busy clears.
- Cancel mid-thinking sends a cooperative protocol Cancel (then kill if unresponsive); BusyGuard clears when blocking work settles — dictation can start cleanly afterward.
- Apply/undo failures surface stable `errorCode`s (`target_gone`, `selection_changed`, `paste_failed`, …) without content in logs.
- Instructions never enter transcription history or stats.

## Sidecar removal / lifecycle

- Packaged as Tauri `externalBin` (`murmur-llm-sidecar`), signed with hardened runtime + App Sandbox (split entitlements via the repository finalizer).
- Host spawns with empty env, fixed cwd, model as inherited read-only fd 3 — no path args, no network in the helper.
- Idle unload and RSS ceilings are enforced by the host supervisor (`llm_sidecar.rs`). Circuit breaker disables the runtime after repeated faults until explicit reset.
- The app crate must **never** link `llama-cpp-2` (ggml ABI clash with whisper).

## Settings UI

Settings → **Transform**:

- Enable + hold-key picker (rejects dictation keys)
- Model status / Download / Remove / Reset runtime
- Saved transforms CRUD (`KnowledgeKind::Transform`) plus built-in presets: Shorten, Bullets, Professional, Fix grammar, Casual

## Related modules

| Area | Path |
|------|------|
| Flow orchestrator | `src/transform_flow.rs` |
| Apply / undo | `src/transform_apply.rs` |
| Selection capture | `src/selection.rs` |
| Sidecar supervisor | `src/llm_sidecar.rs` |
| Presets | `src/transform_presets.rs` |
| Model install | `commands/transform_model.rs` |
| Popover window | `commands/transform_popover.rs`, `components/transform-review/` |
| Frontend drivers | `useTransformFlow`, `useTransformReviewDriver` |
