# Text Injection

## Overview

After a current recording's transcription, text is always copied to the clipboard. Optionally, the app simulates a native CoreGraphics `Cmd+V` keystroke into the focused application. A stale or cancelled recording performs no delivery write.

## Clipboard (`injector.rs`)

Uses `arboard` crate (maintained by 1Password). Text is set via `Clipboard::new()` + `clipboard.set_text()`.

This always happens, regardless of auto-paste setting. The user can always manually Cmd+V.

## Auto-Paste

When `auto_paste` is enabled in settings:

1. Use a read-only `AXIsProcessTrusted()` precheck only to decide whether the configurable focus-settle delay is useful. When permission is already available, wait asynchronously for that delay (default 0ms) before main-thread dispatch so macOS activation and Space notifications can advance. The definitive permission decision still occurs later under ownership.
2. Re-check that the exact `recording_id` still owns delivery. A stale completion stops here without writing or pasting, so it cannot overwrite a newer recording's delivery state.
3. Check `AXIsProcessTrusted()` under that ownership fence. If accessibility is not granted, write the transcript to the clipboard and stop.
4. Verify the anchored target against the current native frontmost identity. The anchor is the sample frozen at the accepted stop transition when that sample is a complete identity, and otherwise the recording-start sample; a self-owned, incomplete, mismatched, or absent stop sample never makes delivery more permissive than the start-anchored behavior it replaces. The anchored target must be complete and external to Murmur, and the current bundle, PID, and process-instance evidence must identify that same process instance. A temporarily unavailable native lookup may be retried only within the bounded verifier; it never changes the frozen target or tries a different process.
5. Fail closed to clipboard-only for a different application, different process, process relaunch, partial identity mismatch, unavailable/incomplete identity, or self-owned anchored target. Contradictory native PID or launch-instance evidence is terminal even when an unbundled process withholds its bundle identity; a later sample cannot erase it. Moving to a different window in the same exact process instance remains eligible. Window relation, application activation, and Space changes are diagnostic facts only and never override identity verification. Every current refusal writes the transcript before returning clipboard-only.
6. Query the verified process's focused element role with the native macOS Accessibility API. Native AX timeout (`-25204`) remains `Unknown` without launching the compatibility query. A no-value result (`-25212`) does the same for non-Finder apps; Finder retains the bounded System Events role query so its desktop/file-view guard remains effective. Other native failures may also use that compatibility query. Skip auto-paste only when the role is on the confirmed non-editable denylist; unknown roles still allow paste.
7. Verify the exact target again because the Accessibility query may have blocked while focus changed.
8. Write the transcript to `NSPasteboard` and capture only its numeric `changeCount` generation. Clipboard contents are never read back.
9. Verify the exact target a third time, then require the pasteboard generation to remain unchanged. A different generation means another app or the user changed the clipboard; return the unconfirmed `ClipboardChanged` outcome without posting a key or claiming clipboard-only success.
10. Post Command-modified `V` key-down and key-up events through the CoreGraphics HID event tap. There is no AppleScript paste fallback.
11. If native event construction fails, wait 100ms, re-check the focused role, exact target, and unchanged pasteboard generation, then make one native retry only when all three remain safe.
12. Report a typed delivery outcome: automatic paste posted, confirmed clipboard-only, clipboard write failed, or delivery unconfirmed.

### Delay Rationale

The clipboard write (`arboard::set_text()` → `NSPasteboard`) is synchronous, so no delay is needed for clipboard sync. The delay exists solely to let macOS window focus settle after the transcription pipeline returns. The zero-delay default is sufficient for the native path; users can increase up to 500ms via the settings slider for applications that move focus asynchronously. Delivery-target verification runs after this delay, immediately before the focused-field query.

### Configurable Delay

The paste delay is configurable via a range slider in the settings panel (0–500ms, step 10ms). The slider appears when auto-paste is enabled. The value is sent to the Rust backend via `configure_dictation` and clamped to the 0–500 range. This configured delay is one component of the broader **Clipboard / paste** performance stage, which also measures the clipboard write, target/focus safety queries, Cmd+V event, and small dispatch overhead.

### Retry Behavior

Target verification and paste dispatch have separate retry contracts. Each target verification performs at most two bounded retries only when the native frontmost lookup is unavailable. Every retry compares against the original immutable identity; a mismatch is terminal and is never retried into another process. CoreGraphics event posting has no delivery result, so a successful native post completes immediately. If native event construction fails, the injector waits 100ms, re-checks the focused role, target identity, and pasteboard generation, and retries the native post once. A target mismatch stops as confirmed clipboard-only only while Murmur's pasteboard generation is still current. A generation change returns `ClipboardChanged`, an unconfirmed delivery: Murmur does not read or restore the new contents, post a key, claim the transcript is still ready, or show the manual-paste cue. If both native construction attempts fail while the generation remains current, the result is confirmed clipboard-only `PasteFailed`. `Err` is reserved for failure to confirm the initial clipboard write. The caller also enforces a 2s timeout for the complete injection operation.

### Failure Notification

The Rust pipeline emits a content-free, generation-gated `dictation-delivery-outcome` after each current recording's delivery attempt. Confirmed clipboard-only outcomes cover disabled or file-output-suppressed auto-paste, missing Accessibility permission, safe target/focus refusal, and exhausted native paste attempts while Murmur still owns the pasteboard generation. The collapsed non-activating overlay shows those outcomes as a bounded `⌘V` cue with the accessible meaning "Text copied to clipboard. Paste manually." A successful automatic paste, failed clipboard write, `ClipboardChanged`, or unconfirmed timeout never shows the cue; a newer outcome clears any older cue, and duplicate/stale recording IDs are ignored.

Focus refusals and delivery failures still emit `auto-paste-failed` for the main window's detailed five-second banner. Disabled auto-paste and missing Accessibility permission are successful clipboard-only outcomes, not errors, so they emit only the delivery outcome. A recording-target mismatch uses the specific message "App focus changed. Text is in your clipboard; paste it when ready." Messages for clipboard-write failure or an unconfirmed timeout do not claim that text reached the clipboard. Neither event retries the paste, changes focus, or expands the overlay.

### Delivery-target diagnostics and privacy

Each auto-paste delivery that reaches target verification emits its final structured decision as
`pipeline.delivery_target_verified`, correlated only by the positive monotonic
`recording_id`. Its exact all-build telemetry schema admits:

- `anchor_code`: `stop` when the accepted stop transition produced the complete
  identity that authorized verification, or `start` when the immutable
  recording-start sample was used instead;
- `outcome_code`: `verified`, `different_application`, `different_process`,
  `process_relaunched`, `partial_identity_mismatch`, `lookup_unavailable`,
  `start_identity_incomplete`, `start_target_is_self`, or `stale_owner`;
- `source_code`: `native` or `none`;
- `retry_count` from 0 through 2 and `elapsed_ms` capped at 1,000;
- the booleans `same_application`, `same_process`,
  `same_process_instance`, `activation_changed`, `space_changed`,
  `current_is_self`, and `ownership_current`; and
- `window_relation_code`: `unknown`, `same`, or `different`.

The equality booleans are positive proof: `false` does not invent an inequality
when native metadata was unavailable. `partial_identity_mismatch` records the
specific case where bundle identity was unavailable but native PID or
launch-instance evidence provably contradicted the frozen target. The equality and transition fields explain a
refusal without becoming a second authorization path. `anchor_code` records
which frozen sample was verified; it is evidence only and never relaxes any
outcome. The `start_identity_incomplete` and `start_target_is_self` codes keep
their names for schema stability and describe the anchored target, which may be
either sample. In particular, `start_target_is_self` means the anchored
target was Murmur and is refused directly; switching to Murmur later is a
`different_application` outcome with `current_is_self: true`. Malformed required
evidence collapses to the event code alone. The sanitizer runs identically in
debug and release builds and rejects application names, bundle identifiers,
PIDs, process-instance tokens, window titles, transcript or clipboard content,
paths, raw native errors, and unknown nested fields. The human-readable summary
is constant rather than derived from caller text.

### Native path

`NSWorkspace` and `AXUIElement` inspect the target and focused role in-process;
the focused-role check retains its bounded System Events compatibility query
for eligible native failures (including Finder no-value). `CGEvent` is the only dictation
paste path. It never uses an `osascript` paste fallback: an unprovable target,
changed clipboard generation, or failed native event construction remains
fail-closed.

### Threading

`inject_text()` runs on the main thread via `app_handle.run_on_main_thread()` so its AppKit focus lookup and macOS keyboard APIs execute in the expected context.

## Permissions

| Feature | Permission Needed |
|---------|------------------|
| Clipboard copy | None |
| Auto-paste | Accessibility |

Settings > Delivery shows accessibility permission status when effective
auto-paste is enabled, with a "Grant" button that opens System Settings.

## Settings

- `autoPaste: boolean` — enable/disable auto-paste. Persisted to localStorage.
- `autoPasteDelayMs: number` — delay in ms before simulating Cmd+V (default 0, range 0–500). Persisted to localStorage.

Both are sent to the Rust backend via `configure_dictation` command.

## Save to File

Live hotkey dictation can optionally persist its output to disk via two independent toggles in Settings > Delivery:

- `saveTranscript: boolean` — write each transcription to a sequentially numbered `.txt`.
- `saveAudio: boolean` — write each recording to a matching `.wav` (16kHz mono, 16-bit PCM).
- `outputDir: string` — destination folder; empty means the default `Documents/Murmur` (created on first write).

Writing happens in `file_output.rs`, called from `run_transcription_pipeline` after the cancellation checkpoints and before injection. The WAV is written from the original (pre-VAD) 16kHz samples; the `.txt` is only written when the transcript is non-empty. A short sequential base name (`murmur-0001`, `murmur-0002`, …) is shared by the pair. The next number is the highest existing `murmur-NNNN` in the folder plus one (older timestamped names are ignored when numbering).

**Interaction with auto-paste:** when either toggle is on, the recording is treated as a "capture to file" action — the clipboard write still happens (clipboard-first is unconditional), but auto-paste is suppressed (`effective_auto_paste = auto_paste && !(save_transcript || save_audio)`). With both toggles off, behavior is unchanged. Write failures are non-fatal: they are logged and surfaced to the UI via the `file-output-failed` event (text remains in the clipboard).

The UI mirrors this effective state without mutating the stored `autoPaste`
preference: the switch appears off and unavailable while file output is active.
When the stored preference is on, the copy identifies it as paused and says it
will resume when both file toggles are off; when it is already off, the copy
says it remains off.

**Known limitation:** recordings the VAD classifies as no-speech return early before the write step, so they save neither file.
