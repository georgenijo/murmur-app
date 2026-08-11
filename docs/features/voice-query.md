# Voice query

Issue [#538](https://github.com/georgenijo/murmur-app/issues/538). Voice Query is an opt-in bridge from Murmur's local speech recognition to one user-configured CLI executable. Double-tap its dedicated shortcut, ask a question, tap once to finish, and read the CLI's streaming stdout in a separate answer popover.

## Privacy and trust boundary

- Microphone capture and transcription stay in Murmur's existing local ASR runtime.
- Murmur starts the exact absolute executable path directly. It never invokes a shell, builds a command string, expands variables, or interprets the transcript.
- Fixed arguments remain separate argv elements. The recognized question is appended as exactly one final argv element.
- The child receives a cleared environment with only `HOME`, `PATH`, `TMPDIR`, `LANG`, `LC_ALL`, and `LC_CTYPE` forwarded when present. Arbitrary parent secrets are not inherited.
- The configured CLI is outside Murmur's local-only trust boundary. It may send the question or answer to cloud services according to its own configuration; Settings states this before the user opts in. Murmur cannot verify or prevent that egress.
- No executable is selected by default and the shortcut is disabled by default.
- Question and answer content never enters structured telemetry, dictation history, usage statistics, transcript/audio file output, or broadcast state events. Answer chunks are targeted only to the `query-review` webview; full answer retrieval is requester-gated to that window.

## Lifecycle

The shared `rdev` listener owns a third detector for the query shortcut (`alt_r`, `ctrl_l`, or `shift_r`). It uses the same double-tap timing as toggle dictation but has no spoken-keyword path. A query and selected-text transform cannot use the same physical key.

Each accepted press allocates a monotonic `query_pass_id`. Capture, ASR, process ownership, streamed chunks, completion, Copy, Escape, and cancellation carry that exact ID. Stale continuations no-op. Query capture is mutually exclusive with dictation, file transcription, Performance Lab, corpus capture, and selected-text transform.

The state sequence is `connecting → listening → transcribing → running → ready|failed`. A single tap stops capture after the double-tap start. Escape and Cancel are valid throughout. The completed answer is copied to the clipboard and remains selectable in the popover; Copy repeats that action. It is never pasted automatically.

## Process and output bounds

`managed_child` creates a dedicated process group for the exact direct child. Normal completion, timeout, cancellation, Escape, and app exit wait for confirmed child exit and an empty owned process group. A process that cannot be confirmed stopped fails closed and prevents a new query from replacing its ownership record.

Configuration limits are 32 fixed arguments, 4 KiB per argument, 32 KiB total fixed arguments, a 32 KiB question, a 256 KiB answer, and a 5–300 second timeout. Stdout is decoded incrementally across split UTF-8 sequences. Missing/non-executable paths, non-zero exits, timeouts, oversized output, and empty output surface stable actionable errors without paths or content in telemetry.

## Related modules

| Area | Path |
|------|------|
| Orchestration and process streaming | `app/src-tauri/src/query_flow.rs` |
| Direct child/process-group ownership | `app/src-tauri/src/managed_child.rs` |
| Shared keyboard detector | `app/src-tauri/src/keyboard.rs` |
| Native popover geometry | `app/src-tauri/src/commands/query_popover.rs` |
| Main-window hotkey driver | `app/src/lib/hooks/useQueryFlow.ts` |
| Review window | `app/src/lib/hooks/useQueryReviewDriver.ts`, `app/src/components/query-review/` |
