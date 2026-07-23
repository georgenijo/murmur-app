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

## Instruction expansion and name precedence

`transform_flow::expand_instruction` resolves a spoken instruction in this
order, stopping at the first match:

1. **Built-in preset** (`transform_presets::resolve_preset`) — checked by
   canonical name and every alias.
2. **Saved transform** (`KnowledgeKind::Transform`, `transform_flow::resolve_saved_transform`)
   — checked by name against enabled saved transforms.
3. **Raw transcript** — used as a free-form rewrite instruction when nothing matched.

Both lookups normalize the spoken text and the stored name with the same
`transform_presets::normalize()`: split on whitespace, trim non-alphanumeric
characters from each word's edges (Unicode-aware), lowercase, and rejoin —
so ASR punctuation ("Shorten.", "Make shorter!", "  fix grammar?  ") does not
prevent a match.

**Presets always shadow saved transforms with the same normalized name.** If
you save a transform named "shorten" (or an alias like "make shorter"), the
built-in preset still runs when you speak that name — the saved transform is
stored but unreachable by voice until you rename it or the built-in changes.
The Transforms editor rejects saving two transforms with the same normalized
name outright, and warns (without blocking) when a name collides with a
preset name or alias.

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
- Secure/password fields: AXSubrole checked **before** any value read; AX errors during the check fail closed — including for the clipboard fallback: an errored secure-field check (`secure_check_failed`, #334) may be retried by the AX warm-up loop (a retry re-runs the full check) but never falls over to the synthetic Cmd+C. No popover content; optional overlay flash only.

## Supported vs best-effort apps

| Class | Examples | Expected behavior |
|-------|----------|-------------------|
| AX-native text | Notes, Mail compose, TextEdit, many Cocoa fields | Selection capture + AX set-value preferred; undo restores original |
| Best-effort (paste fallback) | Many browsers, Slack, Electron shells | Capture often works; apply may paste-replace; clipboard is saved/restored with an epoch guard |
| Documented limitation | Cursor chat / other webview editors | Best-effort only — webview accessibility is incomplete. Not a blocker for #312; file follow-ups rather than scope-creeping AX work |

### Clipboard capture fallback (#329)

Chromium/Electron webviews (Brave, Chrome, Slack, …) often expose no `AXSelectedText` even with a live visible selection. When the AX capture returns `NoSelection` (secure-field checks passed benignly) **or `AxUnavailable`** (AX failed somewhere *other than* the secure-field check — e.g. the focused-element query failed/timed out; Chromium routinely misses the 25ms messaging deadline), capture first retries the AX read (3 attempts, 250ms apart — Chromium builds its accessibility tree lazily, and the first queries are what trigger the build; retries ride the user's key-hold so the latency is invisible), then falls back to: snapshot the **full pasteboard** (every item, every type — images/files survive, #335) → overwrite with a unique text sentinel → record the pasteboard `changeCount` → synthetic Cmd+C → poll until the pasteboard shows **exactly one** ownership change past the sentinel write with non-sentinel text (300ms deadline; with nothing selected Cmd+C is a no-op and the fallback times out; a count that moves more than one step means a third-party writer — clipboard manager, Universal Clipboard — interleaved, and that text is rejected, #335) → restore the user's pasteboard in full, success or failure. The fallback never runs for a positively detected `SecureField`, for `AccessibilityDenied`, or for an **errored secure-field check** (`secure_check_failed`, #334 — if the AXSubrole/AXRole read failed or timed out we could not prove the focused element is not a password field, so no synthetic Cmd+C; the AX retry loop may retry it, since a retry re-runs the full check, but the bar is **sticky for the whole capture**: an errored secure check on *any* attempt bars the fallback even if a later attempt fails shallower). Falling back on the remaining `AxUnavailable` cases is safe because the fallback only reproduces the user's own Cmd+C gesture and reads nothing via AX: secure fields refuse Copy system-wide (NSSecureTextField disables it at the framework level; browsers block password-field copy), so against a secure field the sentinel never changes and the fallback times out — it can fail, never leak. If the fallback fails, the AX path's terminal error is preserved (#336): `ax_unavailable` shows "Couldn't read the selection", and `no_selection` ("Select some text first") is reserved for the case where AX itself reported an empty selection. Residual limitation of the `changeCount` guard: it counts pasteboard changes but cannot attribute a writer, so if the synthetic Cmd+C is swallowed (the Cmd+Opt+C browser case) and **exactly one** foreign write lands inside the 300ms window, that single write is indistinguishable from a real copy and is accepted; no NSPasteboard API closes this fully — the guard reduces the exposure from "any foreign write in the window" to that single-quiet-write case. Remaining limitation: the snapshot has no AX range/bounds, so the popover centers and apply uses the paste fallback. Logging stays content-free (`via="clipboard_fallback"` + length bucket only). Known limitation: the fallback's Cmd+C fires while the transform key is physically held, and apps that read hardware modifier state (observed: Brave) see Cmd+Opt+C — not Copy — so for browsers the AX retry is the effective path.

## Failure semantics

- Any sidecar crash, timeout, protocol violation, model mismatch, or malformed output → popover **failed**, original selection **untouched**, dictation unaffected once busy clears.
- Cancel mid-thinking sends a cooperative protocol Cancel (then kill if unresponsive); BusyGuard clears when blocking work settles — dictation can start cleanly afterward.
- Apply/undo failures surface stable `errorCode`s (`target_gone`, `selection_changed`, `paste_failed`, …) without content in logs.
- Instructions never enter transcription history or stats.

## Correlated diagnostics

Every physical transform-key hold receives a process-local monotonic
`transform_pass_id` in the Rust rdev callback. The ID is carried explicitly
through the Tauri key events and commands, the active transform session, AX
capture attempts, instruction audio/ASR, sidecar work, popover effects,
apply/undo, cancellation, and linger cleanup. It is not inferred from a tracing
span, so correlation survives frontend dispatch, worker threads, and spawned
blocking work.

Retrying an instruction keeps the same pass ID and advances a one-based
instruction-attempt counter. A new physical hold receives a new ID; a stale
release or stale command cannot mutate the active pass. Status transitions log
their actual `from`, requested `to`, and whether the atomic transition won.
Pass resolution uses stable outcomes (`ready`, `failed`, `cancelled`, `applied`,
`undone`) plus stable stage/error codes.

The persistent `PerformanceRunV1` uses that same pass ID. A successfully
claimed pass records selected-text capture, instruction capture/ASR,
sidecar spawn/load, generation, and review-ready or terminal failure. The base
run completes once; later Apply/Undo attempts append bounded, content-free
follow-up duration/outcome records. Sidecar CPU/RSS samples use the helper's
resident PID and are summarized only for the transform run's wall-clock
interval. No second transform tracing path is introduced.

Transform diagnostics are content-free. They may contain IDs, enum values,
numeric AX outcomes, booleans, sample/token counts, timings, apply/capture
routes, and length buckets. They never contain selected text, instruction or
preset text, proposals, clipboard contents, paths, bundle IDs, device names, or
model-setting values. The structured-event layer independently removes any
string whose key or value is outside the transform stream's explicit stable
vocabulary.

## Sidecar removal / lifecycle

- Packaged as Tauri `externalBin` (`murmur-llm-sidecar`), signed with hardened runtime + App Sandbox (split entitlements via the repository finalizer).
- Host spawns with empty env, fixed cwd, model as inherited read-only fd 3 — no path args, no network in the helper.
- Idle unload and RSS ceilings are enforced by the host supervisor (`llm_sidecar.rs`). Circuit breaker disables the runtime after repeated faults until explicit reset.
- The app crate must **never** link `llama-cpp-2` (ggml ABI clash with whisper).

## Settings UI

Settings → **Transform**:

- Enable + hold-key picker (rejects dictation keys)
- Model status / Download / Remove / Reset runtime
- Saved transforms CRUD (`KnowledgeKind::Transform`) plus built-in presets: Shorten, Bullets, Professional, Fix grammar, Casual. Instructions are capped at `MAX_INSTRUCTION_BYTES` (4096 bytes); exact duplicate saved names are rejected, and a name colliding with a preset (name or alias) is shadowed — see [Instruction expansion and name precedence](#instruction-expansion-and-name-precedence)

## Threading

- The popover's `NSWindow` treatment (level, `_setPreventsActivation:`, shadow) is raw AppKit and **must** run on the main thread — macOS 26 hard-traps (`EXC_BREAKPOINT`, "Must only be used from the main thread") on off-main `NSWindow` mutation. The flow's effects run in async command context (tokio worker), so `native_window::set_window_level_and_activation` dispatches the raw calls through `run_on_main_thread`, matching the AX write paths in `selection.rs` / `injector` / `transform_apply.rs`. Tauri's own window methods (`set_size`/`set_position`/`show`/`hide`/`set_focus`) already hop to main internally.
- Native smoke canary for this crash class: **press the transform hold key on a built `.app`** (not `?mock=1`, not `tauri dev`). It exercises the real off-main window path and reproduced #325 immediately.

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
