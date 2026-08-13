# Voice query

Issue [#538](https://github.com/georgenijo/murmur-app/issues/538). Voice Query is an opt-in bridge from Murmur's local speech recognition to one user-configured CLI executable. Double-tap its dedicated shortcut, ask a question, tap once to finish, and read the CLI's streaming stdout in a separate answer popover.

## Privacy and trust boundary

- Microphone capture and transcription stay in Murmur's existing local ASR runtime.
- Murmur starts the exact absolute executable path directly. It never invokes a shell, builds a command string, expands variables, or interprets the transcript.
- Fixed arguments remain separate argv elements. The recognized question is appended as exactly one final argv element.
- The child receives a cleared environment with only `HOME`, `PATH`, `TMPDIR`, `LANG`, `LC_ALL`, `LC_CTYPE`, `USER`, and `LOGNAME` forwarded when present. Arbitrary parent secrets are not inherited. `USER` is required on macOS: Claude Code derives its Keychain credential account name from it and reports "Not logged in" without it.
- The configured CLI is outside Murmur's local-only trust boundary. It may send the question or answer to cloud services according to its own configuration; Settings states this before the user opts in. Murmur cannot verify or prevent that egress.
- No executable is selected by default and the shortcut is disabled by default.
- Question and answer content never enters structured telemetry, dictation history, usage statistics, transcript/audio file output, or broadcast state events. Answer chunks and listening partials are targeted only to the `query-review` webview; full answer retrieval is requester-gated to that window. Partials are display-only and are never stored on the session or sent to the CLI.

## Lifecycle

The shared `rdev` listener owns a third detector for the query shortcut (`alt_r`, `ctrl_l`, or `shift_r`). It uses the same double-tap timing as toggle dictation but has no spoken-keyword path. A query and selected-text transform cannot use the same physical key.

Each accepted press allocates a monotonic `query_pass_id`. Capture, ASR, process ownership, streamed chunks, listening partials, completion, Copy, Escape, and cancellation carry that exact ID. Stale continuations no-op.

Exclusivity is scoped to what a query actually holds, and the two predicates in `query_flow.rs` are deliberately different. While the query owns the microphone or the shared ASR backend (`connecting`, `listening`, `transcribing`), `blocks_capture` refuses dictation, file transcription, and the Settings microphone test. Once the query reaches `running` it holds only its CLI child — capture stopped and joined before `transcribing`, ASR finished before `running` — so dictation is free to start again during answer generation, which is the longest phase. The stricter `blocks_pipeline` additionally covers `running` and continues to exclude Performance Lab, corpus capture, and selected-text transform, because the child competes for CPU and may itself be a heavy inference runtime. Dictation refused during capture reports `busy_querying` rather than failing silently.

Dictation and a query answer both finish on the clipboard, and the answer yields. `injector` keeps a monotonic count of Murmur's successful clipboard writes; the query snapshots it before starting the CLI and re-checks it on completion. If anything else wrote the clipboard meanwhile — a dictation started during answer generation, or a transform — the answer skips its auto-copy, reports `clipboard_superseded`, and stays in the popover behind Copy. Text the user already produced and may have pasted is never replaced underneath them, and the rule is general rather than a dictation-specific special case.

The state sequence is `connecting → listening → transcribing → running → ready|failed`. A single tap stops capture after the double-tap start. Escape and Cancel are valid throughout. While listening, Core ML Parakeet re-decodes the growing capture buffer about every 700 ms (one inference in flight, skipped if a decode is already running) and shows the words in the popover. Those partials are display-only: they skip VAD and transcript transforms, never feed the CLI, and stop updating after 20 seconds of captured audio so cost stays bounded. Whisper and CPU Parakeet stay silent during listening so a slow in-flight decode cannot stall the finish path. Words on screen can revise as more context arrives; that is expected of re-decoding a growing buffer. Sending, cancelling, or failing clears the partial immediately. The completed answer is copied to the clipboard and remains selectable in the popover; Copy repeats that action. It is never pasted automatically. Answer text is rendered as sanitised Markdown (`react-markdown` + `rehype-sanitize`, the same pairing the updater modals use) so headings, lists, and code blocks read as formatted prose; the CLI's output is untrusted, so raw HTML is stripped rather than rendered. Listening partials stay plain text.

The popover never activates Murmur. It is created non-activating and stays that way through every state, so an arriving answer does not pull focus from the app being worked in and dismissing the popover does not leave the main window frontmost. Clicks and text selection work without key focus, and Escape reaches the pass through the global `rdev` listener rather than the webview's own key handler.

Like the notch overlay, the answer popover is configured `visibleOnAllWorkspaces` so it joins every Space instead of staying pinned to the one it was created on. Its position still derives from the main window's monitor, so on a multi-display setup it follows that display rather than the active one.

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
