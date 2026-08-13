# Voice query

Issue [#538](https://github.com/georgenijo/murmur-app/issues/538). Voice Query is an opt-in bridge from Murmur's local speech recognition to one user-configured CLI executable. Double-tap its dedicated shortcut, ask a question, tap once to finish, and read the CLI's streaming stdout in a separate answer popover.

Issue [#550](https://github.com/georgenijo/murmur-app/issues/550) added provider presets, sign-in preflight, actionable failures, and declared environment variables around that generic bridge. The bridge itself did not change: Murmur still spawns one absolute executable with fixed argv and no shell.

## Privacy and trust boundary

- Microphone capture and transcription stay in Murmur's existing local ASR runtime.
- Murmur starts the exact absolute executable path directly. It never invokes a shell, builds a command string, expands variables, or interprets the transcript.
- Fixed arguments remain separate argv elements. The recognized question plus any frozen context is appended as exactly one final argv element.
- The child receives a cleared environment with only `HOME`, `PATH`, `TMPDIR`, `LANG`, `LC_ALL`, `LC_CTYPE`, `USER`, and `LOGNAME` forwarded when present, plus any variables the user declared explicitly (below). Arbitrary parent secrets are not inherited. `USER` is required on macOS: Claude Code derives its Keychain credential account name from it and reports "Not logged in" without it.
- The configured CLI is outside Murmur's local-only trust boundary. It may send the question or answer to cloud services according to its own configuration; Settings states this before the user opts in. Murmur cannot verify or prevent that egress.
- No executable is selected by default and the shortcut is disabled by default.
- Desktop context is separately opt-in: level 0 (default) attaches nothing;
  level 1 freezes the frontmost app name and focused-window title; level 2 also
  attempts the current AX selection through the transform capture path. Secure
  fields and ambiguous secure checks fail closed, selection is capped at 8 KiB,
  and a focus change prevents a later selection from being paired with the
  original app. A per-app profile can exclude all query context for that app.
- Question, context, and answer content never enters structured telemetry, dictation history, usage statistics, transcript/audio file output, or broadcast state events. Answer chunks are targeted only to the `query-review` webview; full answer and context-summary retrieval is requester-gated to that window. The summary contains app/window labels and selection byte count, never the selection itself.

## Provider presets

A preset is static data in `query_presets.rs`, not a code path: binary name, home-relative install fallbacks, the arguments that put the provider in one-shot print mode, an auth-probe argv, the argv for its own interactive login, its "you are not signed in" output signatures, and the environment names it actually reads. Claude Code, Codex, Grok, and Cursor Agent ship as presets; Custom leaves every field to the user. Choosing a preset only *fills in* the executable and arguments — Murmur always spawns exactly what those fields say afterwards, and a preset whose binary was not found leaves a manually chosen path alone.

Discovery probes the standard absolute prefixes (`/opt/homebrew/bin`, `/usr/local/bin`, …), per-user install prefixes under `$HOME`, each preset's own private locations, and finally the host process `PATH`. The fixed prefixes are what make discovery work for a Finder or Dock launch, where `PATH` is the bare system default.

## Preflight and sign-in

`validate_query_command` runs the identical validator the query uses when Voice Query is enabled, so a missing or non-executable path is refused at configuration time rather than mid-question, after the user has already spoken.

Settings' "Test sign-in" runs the preset's auth probe through the same `spawn_user_cli` path — same cleared environment, same declared pairs, same process-group ownership — so a green check proves the real thing will work. The verdict is decided by signature first and exit code second, because `claude auth status` reports `"loggedIn": false` and still exits 0. A clean exit alone is not enough either: a preset that declares what its success looks like must actually say it, so a renamed subcommand, a shim, or a stubbed binary reads as `unknown` rather than earning a green check. Anything inconclusive shows the raw output instead of guessing.

The probe spawns the same user CLI a query does, so it takes the same exclusivity — pressing Test while a query, dictation, transform, or benchmark is running is refused rather than starting a second provider process alongside it. That output routinely names an account and organisation, so it is returned only to the requesting main window and never written to telemetry, history, or the event log.

"Sign in…" launches the vendor CLI's own login in Terminal (`claude auth login`, `codex login`, …). Murmur never sees, prompts for, or proxies the credential; it opens the flow and then re-probes every few seconds for up to two minutes until the provider reports signed in. The only string ever built for a shell is that Terminal command line, and both the validated executable path and the preset's static argv are single-quoted rather than trusted.

## Declared environment variables

The fail-closed allowlist is right for secrets and wrong for the handful of pairs a provider CLI needs to find its own configuration, so Settings accepts explicit name/value pairs (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`, …). They are applied *underneath* the inherited allowlist, so a declared pair can never shadow `HOME` or any other allowlist key; the validator refuses those names outright, along with malformed names, duplicates, and anything past 16 pairs or a 4 KiB value.

Two families are refused because they are not configuration. Loader variables (`DYLD_*`, `LD_*`, `NODE_OPTIONS`, `PYTHONPATH`, `RUBYOPT`, `BASH_ENV`, …) change *which code the CLI runs* — every preset is a Node, Python, or Ruby program behind a shim, so these are as good as `DYLD_INSERT_LIBRARIES` for executing attacker code inside the child. Proxy and CA variables (`HTTPS_PROXY`, `SSLKEYLOGFILE`, `NODE_EXTRA_CA_CERTS`, `SSL_CERT_FILE`, …) change *where its traffic goes* and can silently redirect, intercept, or key-log the provider's TLS, which is the privacy half of the same rule. Matching is case-insensitive: environment lookup is case-sensitive on Unix, so a deny list that only knows `HTTPS_PROXY` is bypassed by `https_proxy`.

The pairs live in a Rust-owned `query-env.json` (0600) in the app data directory rather than the frontend settings blob, so they are never mirrored into localStorage. Both commands are main-window gated, and a hand-edited file is re-validated on read: a tampered one is refused rather than trusted, and the query still runs with the inherited allowlist. Values are stored in plain text — this surface is for configuration, not credentials, and Settings says so. Accepting secrets needs a Keychain-backed design first.

The declared set is resolved once at the start of a pass, like every other piece of per-recording context: editing it mid-question applies to the next query.

## Lifecycle

The shared `rdev` listener owns a third detector for the query shortcut (`alt_r`, `ctrl_l`, or `shift_r`). It uses the same double-tap timing as toggle dictation but has no spoken-keyword path. A query and selected-text transform cannot use the same physical key.

Each accepted press allocates a monotonic `query_pass_id`. Capture, immutable desktop context, ASR, process ownership, streamed chunks, completion, Copy, Escape, and cancellation carry that exact ID. Stale continuations no-op. Microphone startup begins before the optional AX selection lookup, so Chromium warm-up or clipboard fallback cannot clip the start of the spoken question; CLI launch waits for that pass's context result.

Exclusivity is scoped to what a query actually holds, and the two predicates in `query_flow.rs` are deliberately different. While the query owns the microphone or the shared ASR backend (`connecting`, `listening`, `transcribing`), `blocks_capture` refuses dictation, file transcription, and the Settings microphone test. Once the query reaches `running` it holds only its CLI child — capture stopped and joined before `transcribing`, ASR finished before `running` — so dictation is free to start again during answer generation, which is the longest phase. The stricter `blocks_pipeline` additionally covers `running` and continues to exclude Performance Lab, corpus capture, and selected-text transform, because the child competes for CPU and may itself be a heavy inference runtime. Dictation refused during capture reports `busy_querying` rather than failing silently.

Dictation and a query answer both finish on the clipboard, and the answer yields. `injector` keeps a monotonic count of Murmur's successful clipboard writes; the query snapshots it before starting the CLI and re-checks it on completion. If anything else wrote the clipboard meanwhile — a dictation started during answer generation, or a transform — the answer skips its auto-copy, reports `clipboard_superseded`, and stays in the popover behind Copy. Text the user already produced and may have pasted is never replaced underneath them, and the rule is general rather than a dictation-specific special case.

The state sequence is `connecting → listening → transcribing → running → ready|failed`. A single tap stops capture after the double-tap start. Escape and Cancel are valid throughout. When context is enabled, the popover always says what happened (`Context: Safari — … — 1.2 KB selection`, `excluded for this app`, or `unavailable`) so attachment is never invisible. The completed answer is copied to the clipboard and remains selectable in the popover; Copy repeats that action. It is never pasted automatically. Answer text is rendered as sanitised Markdown (`react-markdown` + `rehype-sanitize`, the same pairing the updater modals use) so headings, lists, and code blocks read as formatted prose; the CLI's output is untrusted, so raw HTML is stripped rather than rendered.

The popover never activates Murmur. It is created non-activating and stays that way through every state, so an arriving answer does not pull focus from the app being worked in and dismissing the popover does not leave the main window frontmost. Clicks and text selection work without key focus, and Escape reaches the pass through the global `rdev` listener rather than the webview's own key handler.

Like the notch overlay, the answer popover is configured `visibleOnAllWorkspaces` so it joins every Space instead of staying pinned to the one it was created on. Its position still derives from the main window's monitor, so on a multi-display setup it follows that display rather than the active one.

## Process and output bounds

`managed_child` creates a dedicated process group for the exact direct child. Normal completion, timeout, cancellation, Escape, and app exit wait for confirmed child exit and an empty owned process group. A process that cannot be confirmed stopped fails closed and prevents a new query from replacing its ownership record.

Configuration limits are 32 fixed arguments, 4 KiB per argument, 32 KiB total fixed arguments, a 32 KiB spoken question, 256-byte app name, 1 KiB window title, 8 KiB selected-text context, a 256 KiB answer, 16 declared environment variables of 4 KiB each, and a 5–300 second timeout. Question and context are framed as untrusted reference data inside exactly one final literal argv element; no shell is involved. Stdout is decoded incrementally across split UTF-8 sequences. Missing/non-executable paths, non-zero exits, timeouts, oversized output, and empty output surface stable actionable errors without paths or content in telemetry.

stderr is piped rather than discarded and drained continuously — a chatty CLI would otherwise fill the pipe buffer and block — into a bounded 16 KiB *tail*, because the line that explains a failure is the last one. On a failure that tail is stored on the session and read back by the popover through the same requester-gated path as the answer; it is never broadcast, since it can quote a path, a prompt, or an account name. When the tail or the partial stdout carries a recognised auth-failure signature, `exit_nonzero`/`process_failed`/`empty_answer` become `provider_not_authenticated`, which names the exact fix and offers the vendor sign-in. Codes that describe Murmur's own bounds (`timed_out`, `termination_unconfirmed`, `output_too_large`) are never reinterpreted.

On a failure the popover shows the error, the fix, and the CLI's own words, with any partial stdout labelled as evidence below. It previously rendered `answer || errorMessage`, so a provider that printed "Not logged in" on stdout and exited non-zero looked as though it had answered — the failure was invisible.

## Related modules

| Area | Path |
|------|------|
| Orchestration and process streaming | `app/src-tauri/src/query_flow.rs` |
| Provider presets, discovery, auth probe, login launch | `app/src-tauri/src/query_presets.rs` |
| Declared environment variables | `app/src-tauri/src/query_env.rs` |
| Settings provider block | `app/src/components/settings/VoiceQueryProvider.tsx` |
| Shared error codes and fixes | `app/src/lib/queryErrors.ts`, `app/src/lib/queryProviders.ts` |
| Direct child/process-group ownership | `app/src-tauri/src/managed_child.rs` |
| Shared keyboard detector | `app/src-tauri/src/keyboard.rs` |
| Native popover geometry | `app/src-tauri/src/commands/query_popover.rs` |
| Main-window hotkey driver | `app/src/lib/hooks/useQueryFlow.ts` |
| Review window | `app/src/lib/hooks/useQueryReviewDriver.ts`, `app/src/components/query-review/` |
