# Voice query

Issues [#538](https://github.com/georgenijo/murmur-app/issues/538), [#550](https://github.com/georgenijo/murmur-app/issues/550), and [#551](https://github.com/georgenijo/murmur-app/issues/551). Voice Query is an opt-in bridge from Murmur's local speech recognition to one user-configured CLI executable. Double-tap its dedicated shortcut, ask a question, tap once to finish, and read the CLI's streaming answer in a separate popover.

## Privacy and trust boundary

- Microphone capture and transcription stay in Murmur's existing local ASR runtime.
- Murmur starts the exact absolute executable path directly. It never invokes a shell, builds a command string, expands variables, or interprets the transcript.
- Fixed arguments remain separate argv elements. The recognized question is appended as exactly one final argv element.
- The child receives a cleared environment with only `HOME`, `PATH`, `TMPDIR`, `LANG`, `LC_ALL`, `LC_CTYPE`, `USER`, and `LOGNAME` forwarded when present. Arbitrary parent secrets are not inherited. `USER` is required on macOS: Claude Code derives its Keychain credential account name from it and reports "Not logged in" without it.
- A provider may add only its declared config-directory selector: `CLAUDE_CONFIG_DIR` for Claude and `CODEX_HOME` for Codex (Custom may use either). Base allowlist keys, undeclared names, API keys, and tokens are rejected. Values are owner-only Rust app data, never localStorage; Settings receives only the configured variable names after saving and never reads saved values back. Explicit **Clear saved values** also repairs a malformed or future-version store by replacing the untrusted file with an empty current-version store.
- The configured CLI is outside Murmur's local-only trust boundary. It may send the question or answer to cloud services according to its own configuration; Settings states this before the user opts in. Murmur cannot verify or prevent that egress.
- No executable is selected by default and the shortcut is disabled by default.
- Question and answer content never enters structured telemetry, dictation history, usage statistics, transcript/audio file output, or broadcast state events. Answer chunks are targeted only to the `query-review` webview; full answer retrieval is requester-gated to that window.

## Lifecycle

The shared `rdev` listener owns a third detector for the query shortcut (`alt_r`, `ctrl_l`, or `shift_r`). It uses the same double-tap timing as toggle dictation but has no spoken-keyword path. A query and selected-text transform cannot use the same physical key.

Each accepted press allocates a monotonic `query_pass_id`. Capture, ASR, process ownership, streamed chunks, completion, Copy, Escape, and cancellation carry that exact ID. Stale continuations no-op.

Exclusivity is scoped to what a query actually holds, and the two predicates in `query_flow.rs` are deliberately different. While the query owns the microphone or the shared ASR backend (`connecting`, `listening`, `transcribing`), `blocks_capture` refuses dictation, file transcription, and the Settings microphone test. Once the query reaches `running` it holds only its CLI child — capture stopped and joined before `transcribing`, ASR finished before `running` — so dictation is free to start again during answer generation, which is the longest phase. The stricter `blocks_pipeline` additionally covers `running` and continues to exclude Performance Lab, corpus capture, and selected-text transform, because the child competes for CPU and may itself be a heavy inference runtime. Dictation refused during capture reports `busy_querying` rather than failing silently.

Dictation and a query answer both finish on the clipboard, and the answer yields. `injector` keeps a monotonic count of Murmur's successful clipboard writes; the query snapshots it before starting the CLI and re-checks it on completion. If anything else wrote the clipboard meanwhile — a dictation started during answer generation, or a transform — the answer skips its auto-copy, reports `clipboard_superseded`, and stays in the popover behind Copy. Text the user already produced and may have pasted is never replaced underneath them, and the rule is general rather than a dictation-specific special case.

The state sequence is `connecting → listening → transcribing → running → ready|failed`. A single tap stops capture after the double-tap start. Escape and Cancel are valid throughout. The completed answer is copied to the clipboard and remains selectable in the popover; Copy repeats that action. It is never pasted automatically. Answer text is rendered as sanitised Markdown (`react-markdown` + `rehype-sanitize`, the same pairing the updater modals use) so headings, lists, and code blocks read as formatted prose; the CLI's output is untrusted, so raw HTML is stripped rather than rendered.

The popover never activates Murmur. It is created non-activating and stays that way through every state, so an arriving answer does not pull focus from the app being worked in and dismissing the popover does not leave the main window frontmost. Clicks and text selection work without key focus, and Escape reaches the pass through the global `rdev` listener rather than the webview's own key handler.

Like the notch overlay, the answer popover is configured `visibleOnAllWorkspaces` so it joins every Space instead of staying pinned to the one it was created on. Its position still derives from the main window's monitor, so on a multi-display setup it follows that display rather than the active one.

## Provider presets and preflight

Claude, Codex, Grok, Cursor, and Custom are data presets rather than alternate process-launch paths. A preset declares discovery candidates, recommended literal argv, an authentication probe, known authentication-failure signatures, an interactive sign-in command, and permitted config-directory environment names. Claude's recommended argv is `--print --verbose --output-format stream-json --include-partial-messages`; `--verbose` is a single literal argument required by Claude's structured stream mode. Codex's starts with `exec --json`. Selecting a preset copies its discovered absolute executable and recommended argv into the editable configuration; Custom preserves the generic bridge.

Enabling Voice Query validates the exact executable, argv limits, timeout, provider, and Rust-owned environment before the global listener is armed. Settings' **Test** action additionally runs the preset's authentication probe (for example, `claude auth status` or `codex login status`) through the identical direct `spawn_user_cli` path. Probe stdout and stderr are bounded to 16 KiB tails and shown only in Settings; they never enter telemetry or logs. Custom has no built-in authentication probe, so Test validates its executable and configuration only.

Known authentication output maps to `provider_not_authenticated` and an exact repair such as “Run claude /login in Terminal.” **Sign in…** is an explicit exception that opens the provider-owned interactive command in Terminal, then re-runs the direct bounded probe until it succeeds or the polling window ends. Both the AppleScript launcher and Terminal command start from the same exact environment allowlist; the provider command uses `/usr/bin/env -i` before adding declared values. Query execution and probes never use Terminal or a shell.

## Process and output bounds

`managed_child` creates a dedicated process group for the exact direct child. Normal completion, timeout, cancellation, Escape, and app exit wait for confirmed child exit and an empty owned process group. A process that cannot be confirmed stopped fails closed and prevents a new query from replacing its ownership record.

Configuration limits are 32 fixed arguments, 4 KiB per argument, 32 KiB total fixed arguments, a 32 KiB question, a 256 KiB answer, a 16 KiB stderr tail, and a 5–300 second timeout. Stdout is decoded incrementally across split UTF-8 sequences. On terminal failure the error always takes precedence over partial stdout; sanitized stderr appears separately as provider detail only in the requester-gated review window. It never becomes answer content, telemetry, or a log field. Missing/non-executable paths, non-zero exits, timeouts, oversized output, and empty output surface stable actionable errors without paths or content in telemetry.

## Structured provider output

One `VoiceQueryAdapter` seam sits after the shared bounded stdout reader. Claude stream-json text deltas map to the existing sequence-numbered answer events; its terminal result supplies input/output/cache token counts and provider-reported cost for the pass. Codex JSONL emits completed `agent_message` items and terminal usage in the same typed form. Provider banners, reasoning, tool activity, session IDs, and other metadata are not answer text. Usage remains Rust-side and pass-scoped until the separate opt-in presentation and aggregate work in #552.

Typed Claude assistant/result errors (including the SDK `errors[]` field) and Codex `turn.failed`/fatal `error` events fail the pass even if the process exits zero. Claude's typed authentication failures and known provider authentication details map to `provider_not_authenticated`; other typed failures map to content-free `provider_error`. Provider detail remains UTF-8-safe, bounded to the same requester-only 16 KiB field, and never enters telemetry.

Custom, Grok, and Cursor retain raw stdout behavior. If a Claude or Codex line is malformed, a recognized frame has the wrong shape, an event is outside the recognized JSONL contract, or EOF arrives before an authoritative terminal frame, parsing never fails the pass: the adapter atomically replaces any optimistic extracted chunks with the complete raw stdout received so far, then streams later bytes in raw mode. The replacement flag and sequence number prevent duplicated content during this fail-safe transition. The structured raw archive shares the existing 256 KiB output cap.

## Related modules

| Area | Path |
|------|------|
| Orchestration and process streaming | `app/src-tauri/src/query_flow.rs` |
| Structured provider adapters and pass-scoped usage | `app/src-tauri/src/query_adapter.rs` |
| Provider presets, auth probes, and declared environment store | `app/src-tauri/src/query_provider.rs` |
| Direct child/process-group ownership | `app/src-tauri/src/managed_child.rs` |
| Shared keyboard detector | `app/src-tauri/src/keyboard.rs` |
| Native popover geometry | `app/src-tauri/src/commands/query_popover.rs` |
| Main-window hotkey driver | `app/src/lib/hooks/useQueryFlow.ts` |
| Review window | `app/src/lib/hooks/useQueryReviewDriver.ts`, `app/src/components/query-review/` |
