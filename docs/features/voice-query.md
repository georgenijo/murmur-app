# Voice query

Issues [#538](https://github.com/georgenijo/murmur-app/issues/538), [#550](https://github.com/georgenijo/murmur-app/issues/550), [#551](https://github.com/georgenijo/murmur-app/issues/551), [#552](https://github.com/georgenijo/murmur-app/issues/552), [#553](https://github.com/georgenijo/murmur-app/issues/553), [#554](https://github.com/georgenijo/murmur-app/issues/554), and [#572](https://github.com/georgenijo/murmur-app/issues/572). Voice Query is an opt-in bridge from Murmur's local speech recognition to one user-configured CLI executable. Double-tap its dedicated shortcut, ask a question, tap once to finish, and read the CLI's streaming answer in a separate popover.

## Privacy and trust boundary

- Microphone capture and transcription stay in Murmur's existing local ASR runtime.
- Murmur starts the exact absolute executable path directly. It never invokes a shell, builds a command string, expands variables, or interprets the transcript.
- Queries and authentication probes run from an owner-only isolated workspace under Murmur's app-data directory. They never use `/`, the source checkout, or the user's frontmost working directory as ambient provider context.
- Fixed arguments remain separate argv elements. The recognized question and any opted-in context are appended together as exactly one final argv element.
- The child receives a cleared environment with only `HOME`, `PATH`, `TMPDIR`, `LANG`, `LC_ALL`, `LC_CTYPE`, `USER`, and `LOGNAME` forwarded when present. Arbitrary parent secrets are not inherited. `USER` is required on macOS: Claude Code derives its Keychain credential account name from it and reports "Not logged in" without it.
- A provider may add only its declared config-directory selector: `CLAUDE_CONFIG_DIR` for Claude and `CODEX_HOME` for Codex (Custom may use either). Base allowlist keys, undeclared names, API keys, and tokens are rejected. Values are owner-only Rust app data, never localStorage; Settings receives only the configured variable names after saving and never reads saved values back. Explicit **Clear saved values** also repairs a malformed or future-version store by replacing the untrusted file with an empty current-version store.
- The configured CLI is outside Murmur's local-only trust boundary. It may send the question, enabled context, or answer to cloud services according to its own configuration; Settings states this before the user opts in. Murmur cannot verify or prevent that egress.
- No executable is selected by default and the shortcut is disabled by default.
- Question, answer, and context content never enters structured telemetry, logs, dictation history, usage statistics, performance diagnostics, transcript/audio file output, or broadcast state events. The separate Voice Query history store is the one exception for question and answer content, and only when **Keep Voice Query history on this Mac** was enabled at the start of that pass. Context is never stored as a separate field, but a retained answer may quote context that was sent to the CLI. Usage statistics accept only content-free counters. Answer chunks and listening partials are targeted only to the `query-review` webview; full answer and context-summary retrieval is requester-gated to that window, while numeric usage may also reach the main window for local aggregation. Partials are display-only and are never stored on the session or sent to the CLI.

## Opt-in app context

Context is off by default. Each pass freezes one level at query start; focus or Settings changes after that point apply only to the next pass:

- **None:** preserves the original question argument byte-for-byte.
- **App & window:** adds the native frontmost application name and Accessibility focused-window title.
- **App, window & selection:** also adds selected text through the same secure-field-aware AX capture policy as Selected-text Transform. The included selection is truncated on a valid UTF-8 boundary at 8 KiB.

The app identity is sampled first and later metadata/selection reads are accepted only for that exact PID and bundle ID, so a focus change cannot combine two applications. Secure fields, ambiguous secure-field checks, unavailable Accessibility data, and selection failures fail closed: no selection is included, while the query can continue with whatever lower-level app metadata was safely captured. There is no screenshot, OCR, Screen Recording permission, or other phase-2 visual capture path.

The final literal prompt argument contains the question followed by a labeled, explicitly untrusted context block. The popover shows a requester-gated summary such as `Context: Safari — window title · 1.2 KB selection`; it never silently attaches context. App/window mode explicitly says `selection off` (and `app only` when no window title was readable), while selection mode says `no readable selection` when its secure capture fails closed. Settings → App Overrides can deny Voice Query context for a specific bundle ID. That deny rule overrides every global or preset context level and never enables context on its own.

## Lifecycle

The shared `rdev` listener owns a third detector for the query shortcut (`alt_r`, `ctrl_l`, or `shift_r`). It uses the same double-tap timing as toggle dictation but has no spoken-keyword path. A query and selected-text transform cannot use the same physical key.

Each accepted press allocates a monotonic `query_pass_id`. Capture, ASR, process ownership, streamed chunks, listening partials, completion, Copy, Escape, and cancellation carry that exact ID. Stale continuations no-op.

Exclusivity is scoped to what a query actually holds, and the two predicates in `query_flow.rs` are deliberately different. While the query owns the microphone or the shared ASR backend (`connecting`, `listening`, `transcribing`), `blocks_capture` refuses dictation, file transcription, and the Settings microphone test. Once the query reaches `running` it holds only its CLI child — capture stopped and joined before `transcribing`, ASR finished before `running` — so dictation is free to start again during answer generation, which is the longest phase. The stricter `blocks_pipeline` additionally covers `running` and continues to exclude Performance Lab, corpus capture, and selected-text transform, because the child competes for CPU and may itself be a heavy inference runtime. Dictation refused during capture reports `busy_querying` rather than failing silently.

**Automatically copy answers** is on by default to preserve the shipped behavior for existing settings. Each query snapshots that preference when the pass starts, alongside its other immutable pass configuration; changing Settings while a query is running applies only to the next pass. When the snapshot is off, completion does not read or write the clipboard. The answer remains in the requester-gated review popover, where its exact-pass **Copy** action is still available.

When automatic copy is enabled, dictation and a query answer can both finish on the clipboard, and the answer yields. `injector` keeps a monotonic count of Murmur's successful clipboard writes; the query snapshots it before starting the CLI and re-checks it on completion. If anything else wrote the clipboard meanwhile — a dictation started during answer generation, or a transform — the answer skips its auto-copy, reports `clipboard_superseded`, and stays in the popover behind Copy. Text the user already produced and may have pasted is never replaced underneath them, and the rule is general rather than a dictation-specific special case.

The state sequence is `connecting → listening → transcribing → running → ready|failed`. A single tap stops capture after the double-tap start. Escape and Cancel are valid throughout. While listening, Core ML Parakeet re-decodes the growing capture buffer about every 700 ms (one inference in flight, skipped if a decode is already running) and shows the words in the popover. Those partials are display-only: they skip VAD and transcript transforms, never feed the CLI. Beyond 20 seconds of captured audio each tick re-decodes only the trailing 20-second window, so per-tick cost stays bounded and the words keep updating until the user stops, cancels, or the pass fails. The live peek is resampled to 16 kHz, the same rate the stop path uses, before decode. Each tick writes a content-free `query.partial_tick` event (`outcome` + `sample_count`, never text) so listening behavior is diagnosable in the same Events stream as other transcription. Whisper and CPU Parakeet stay silent during listening so a slow in-flight decode cannot stall the finish path. Words on screen can revise as more context arrives; that is expected of re-decoding a growing buffer. Sending, cancelling, or failing clears the partial immediately.

Only one current, valid terminal success may claim automatic copy, so duplicate completion cannot write twice. Cancellation, timeout, provider failure, empty output, and stale passes never change the clipboard. A malformed structured response may still become a Ready raw-fallback answer under the fail-safe adapter contract below, but it reports content-free `auto_copy_unavailable` and requires the requester-gated manual Copy action instead of overwriting the clipboard automatically. A clipboard write failure similarly reports `clipboard_unavailable` on an otherwise successful Ready pass. Neither condition changes the provider outcome or exposes the answer in telemetry or logs. Whether copied automatically or left for manual Copy, an answer is never pasted automatically. Answer text is rendered as sanitised Markdown (`react-markdown` + `rehype-sanitize`, the same pairing the updater modals use) so headings, lists, and code blocks read as formatted prose; the CLI's output is untrusted, so raw HTML is stripped rather than rendered. Listening partials stay plain text.

The popover never activates Murmur. It is created non-activating and stays that way through every state, so an arriving answer does not pull focus from the app being worked in and dismissing the popover does not leave the main window frontmost. Clicks and text selection work without key focus, and Escape reaches the pass through the global `rdev` listener rather than the webview's own key handler.

Like the notch overlay, the answer popover is configured `visibleOnAllWorkspaces` so it joins every Space instead of staying pinned to the one it was created on. Its position still derives from the main window's monitor, so on a multi-display setup it follows that display rather than the active one.

## Provider presets and preflight

Claude, Codex, Grok, Cursor, and Custom are data presets rather than alternate process-launch paths. A preset declares discovery candidates, recommended literal argv, an authentication probe, known authentication-failure signatures, an interactive sign-in command, and permitted config-directory environment names. Claude's recommended argv is `--print --verbose --output-format stream-json --include-partial-messages --safe-mode --tools "" --no-session-persistence`; `--verbose` is required by structured stream mode, while the remaining bounds prevent project instructions, plugins, hooks, MCP servers, built-in tools, and persisted sessions from becoming query overhead. Codex's starts with `exec --json`. Cursor uses non-mutating Ask mode and explicitly trusts only Murmur's private isolated workspace so its non-interactive request cannot stall at a workspace-trust prompt. Selecting a preset copies its discovered absolute executable and recommended argv into the editable configuration. If Voice Query is enabled, the switch temporarily disarms its shortcut, validates and preflights that exact immutable preset command, then restores the same shortcut only after success. A failed or superseded check leaves it off, and rapid switches cannot let an older provider re-enable itself. Switching while already disabled never auto-enables it. Editing the executable or arguments still disables the shortcut until that custom command is validated. Custom preserves the generic bridge and requires an explicit absolute executable plus fixed arguments.

Enabling Voice Query validates the exact executable, argv limits, timeout, provider, and Rust-owned environment before the global listener is armed. Settings' **Test** action additionally runs the preset's authentication probe (for example, `claude auth status` or `codex login status`) through the identical direct `spawn_user_cli` path. Probe stdout and stderr are bounded to 16 KiB tails and shown only in Settings; they never enter telemetry or logs. Custom has no built-in authentication probe, so Test validates its executable and configuration only.

The known Codex npm-wrapper failure where its nested macOS platform binary is
missing is normalized at the Rust boundary. Settings and the review popover
show a concise reinstall/update action while retaining the existing stable
`probe_failed` or `exit_nonzero` code; the Node stack, install path, and raw
stderr are not sent to either webview.

Known authentication output maps to `provider_not_authenticated` and an exact repair such as “Run claude /login in Terminal.” **Sign in…** is an explicit exception that opens the provider-owned interactive command in Terminal, then re-runs the direct bounded probe until it succeeds or the polling window ends. Both the AppleScript launcher and Terminal command start from the same exact environment allowlist; the provider command uses `/usr/bin/env -i` before adding declared values. Query execution and probes never use Terminal or a shell.

For Claude, Murmur also appends a fixed Rust-owned `--append-system-prompt` argument pair at validation time, separate from the user-editable fixed arguments above. It tells the model it has no tools or file access and that its working directory is the dictation app's empty private scratch folder rather than the user's project — because with all tools disabled the CLI otherwise fabricates file listings of that folder instead of answering. The instruction is a constant compiled into Murmur; it never contains the question, context, or answer, and it applies to every Claude pass regardless of saved Settings, so a fix here reaches existing saved configurations without the user re-saving. It is not part of `recommended_arguments` and does not affect the authentication probe.

## Process and output bounds

`managed_child` creates a dedicated process group for the exact direct child. Normal completion, timeout, cancellation, Escape, and app exit wait for confirmed child exit and an empty owned process group. A process that cannot be confirmed stopped fails closed and prevents a new query from replacing its ownership record.

Configuration limits are 32 fixed arguments, 4 KiB per argument, 32 KiB total fixed arguments, a 32 KiB question, a 512-byte app name, a 2 KiB window title, an 8 KiB selection, a final composite prompt cap, a 256 KiB answer, a 16 KiB stderr tail, and a 5–300 second timeout. Stdout is decoded incrementally across split UTF-8 sequences. On terminal failure the error always takes precedence over partial stdout; sanitized stderr appears separately as provider detail only in the requester-gated review window. It never becomes answer content, telemetry, or a log field. Missing/non-executable paths, non-zero exits, timeouts, oversized output, and empty output surface stable actionable errors without paths or content in telemetry.

## Structured provider output

One `VoiceQueryAdapter` seam sits after the shared bounded stdout reader. Claude stream-json text deltas map to the existing sequence-numbered answer events; its terminal result supplies input/output/cache token counts and provider-reported cost for the pass. Current Claude hook lifecycle, status, and thinking-token frames are structurally validated and ignored even though safe mode normally suppresses them. Codex JSONL emits completed `agent_message` items and terminal usage in the same typed form. Provider banners, reasoning, tool activity, session IDs, hooks, and other metadata are not answer text. Usage remains Rust-side and pass-scoped while the query is active.

Typed Claude assistant/result errors (including the SDK `errors[]` field) and Codex `turn.failed`/fatal `error` events fail the pass even if the process exits zero. Claude's typed authentication failures and known provider authentication details map to `provider_not_authenticated`; other typed failures map to content-free `provider_error`. Provider detail remains UTF-8-safe, bounded to the same requester-only 16 KiB field, and never enters telemetry.

Custom, Grok, and Cursor retain raw stdout behavior. If a Claude or Codex line is malformed, a recognized frame has the wrong shape, an event is outside the recognized JSONL contract, or EOF arrives before an authoritative terminal frame, parsing never fails the pass: the adapter atomically replaces any optimistic extracted chunks with the complete raw stdout received so far, then streams later bytes in raw mode. The replacement flag and sequence number prevent duplicated content during this fail-safe transition. The structured raw archive shares the existing 256 KiB output cap.

## Token and usage monitoring

After a pass reaches Ready, the query popover footer shows provider-reported input and output tokens and cost when the provider supplied one. Custom, Grok, Cursor, malformed-JSON raw fallback, and provider versions that omit usage remain valid passes and simply show no token summary. Cache and reasoning counts stay in the typed pass record but are not presented as answer content.

The main window folds each exact terminal pass into the existing durable local statistics blob once: completed queries, successes/failures, input/output tokens, provider split, provider-reported cost, and failures by stable error code. The Insights popover presents those all-time counters. Reset Stats clears query counters with dictation statistics. The stats schema accepts only known provider IDs, known error codes, and finite non-negative numbers; arbitrary strings and unknown fields are discarded, so questions, answers, stderr, paths, commands, and credentials cannot become usage-stat fields.

The broadcast terminal state may carry the same typed numeric usage object for local aggregation, but never answer or question content. Query telemetry keeps its exact allowlist: token counts and provider cost converted to integer micro-US-dollars may accompany `query.pass_state`; nested objects, provider names, floating costs, byte counts, and unknown numeric keys are rejected. Provider quota and OAuth usage endpoints remain out of scope—Murmur never reads another app's credentials.

## Opt-in local history and diagnostics

**Keep Voice Query history on this Mac** is off by default. Each pass freezes
that choice when it starts, so changing Settings never changes or cancels the
pass already in flight. When enabled, the terminal pass is written best-effort
to a separate Rust-owned SQLite store. A record contains only its timestamp,
provider preset, original transcribed question, answer (including a bounded
partial answer on failure), provider-reported token counts, total duration, and
stable error code. It never contains the composed prompt as a separate field,
provider stderr or typed error detail, executable path, argv, environment
values, or secrets. An enabled history pass is retained whether or not it
included app/window/selection context or a provider fell back to raw output.
The saved answer may therefore quote context that was sent to the CLI; enabling
history is explicit local consent to retain that question-and-answer result.
Context configured but unavailable or disabled for the current app is simply
not appended. Context and query content remain excluded from telemetry,
statistics, performance diagnostics, and logs.

The store retains the newest 200 records and prunes in the same transaction as
each insert. History → Queries reads it through main-window-only, paged IPC,
offers a provider filter, and exposes a direct **Delete all query history**
action. Turning retention off stops future inserts but does not silently delete
existing records. Query content is never mirrored into localStorage, dictation
history, Correct and Teach, exports, logs, telemetry, stats, or the performance
database. Insert and purge notifications contain only `inserted` or `cleared`.

Every pass still writes a content-free record to the existing Performance
diagnostics store, whether or not content retention is enabled. The Runs
workspace shows Capture, Transcription, Provider spawn, First answer, and Total
timings, plus the process exit code and whether stderr was present. It never
stores stderr bytes, provider detail, question, answer, or context. Clear
Performance Data and Delete all query history therefore remain independent
operations.

## Related modules

| Area | Path |
|------|------|
| Orchestration and process streaming | `app/src-tauri/src/query_flow.rs` |
| Structured provider adapters and pass-scoped usage | `app/src-tauri/src/query_adapter.rs` |
| Provider presets, auth probes, and declared environment store | `app/src-tauri/src/query_provider.rs` |
| Native frontmost app/window metadata | `app/src-tauri/src/frontmost.rs` |
| Secure selected-text capture | `app/src-tauri/src/selection.rs` |
| Direct child/process-group ownership | `app/src-tauri/src/managed_child.rs` |
| Shared keyboard detector | `app/src-tauri/src/keyboard.rs` |
| Native popover geometry | `app/src-tauri/src/commands/query_popover.rs` |
| Main-window hotkey driver | `app/src/lib/hooks/useQueryFlow.ts` |
| Review window | `app/src/lib/hooks/useQueryReviewDriver.ts`, `app/src/components/query-review/` |
| Content-free aggregate counters | `app/src/lib/stats.ts`, `app/src/components/UsageDashboard.tsx` |
| Opt-in local history store and IPC | `app/src-tauri/src/query_history/`, `app/src/lib/queryHistory.ts` |
| Queries workspace | `app/src/lib/hooks/useQueryHistory.ts`, `app/src/components/history/QueryHistoryPanel.tsx` |
| Content-free per-pass diagnostics | `app/src-tauri/src/performance_metrics/`, `app/src/components/log-viewer/` |
