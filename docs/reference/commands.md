# Tauri Commands Reference

The 176 commands registered in `lib.rs` and exposed to the frontend via `invoke()`, grouped by source module under `app/src-tauri/src/`.

Parameters are listed with their Rust names; the frontend passes them camelCased (`model_name` → `modelName`). `app_handle` / `state` / `window` injections are omitted — they are supplied by Tauri, not by the caller.

For Rust → frontend events see [events.md](events.md). For the hooks that call these commands see [hooks.md](hooks.md).

---

## Recording and dictation (`commands/recording.rs`)

| Command | Parameters | Returns | Description |
|---------|-----------|---------|-------------|
| `init_dictation` | — | `Result<JSON, String>` | Static `{"type":"initialized","state":"idle"}` marker. |
| `get_status` | — | `Result<JSON, String>` | Current status, model name, and language. |
| `configure_dictation` | `options: JSON` | `Result<JSON, String>` | Pushes settings into `DictationState`: model, language, auto-paste and delay (clamped 10–500), VAD sensitivity (0–100), punctuation, file output, vocabulary entries, voice commands, app profiles, cleanup/formatting/correction toggles, code-vocab folder, idle timeout. A model change reselects the runtime backend, deferring across an active recording generation. Rejects conflicting vocabulary/command configurations without mutating prior state. |
| `start_native_recording` | `device_name: Option<String>`, `origin: Option<String>` | `Result<JSON, String>` | Resolves the immutable context, claims the single audio owner, starts model preparation, emits `starting`, and returns without waiting for Core Audio. Readiness later emits `recording`. |
| `stop_native_recording` | — | `Result<JSON, String>` | Cancels a `starting` attempt, or stops a ready capture and runs VAD → inference → transcript transform → delivery. Recordings under 0.3s are discarded. |
| `cancel_native_recording` | — | `Result<(), String>` | Cancels initialization or capture through a brief `recovering` transition to `idle`, or discards a processing run without delivery. Detached audio cleanup continues asynchronously. |
| `cancel_audio_initialization` | `reason: "device_changed"` | `Result<(), String>` | Cancels only a `starting` attempt after the selected microphone changes; a ready recording is left alone. |
| `process_audio` | `audio_data: String` | `Result<JSON, String>` | Runs the full pipeline over base64-encoded 16kHz mono WAV. |
| `transcribe_file` | `file_path: String` | `Result<JSON, String>` | Decodes and transcribes an audio file through the same pipeline with live-only stages skipped. Emits `file-transcription-status-changed`. |
| `transform_status` | — | `TransformStatus` | Current selected-text transform state (used to arbitrate against dictation). |
| `count_vocab_tokens` | `text: String` | `Result<Option<usize>, String>` | Token count for the loaded model's tokenizer; `None` if no model is loaded. Drives the Whisper prompt budget UI. |
| `preview_vocabulary_aliases` | `entries`, `voice_commands`, `text`, `cli_formatting` | `Result<String, String>` | Runs alias + command resolution over sample text in memory. No persistence, no delivery. |
| `reformat_history_text` | `raw_text`, `mode_id` | `Result<HistoryReformatResult, String>` | Runs bounded retained raw recognition through one explicit Mode. No audio, injection, statistics, history write, or learning side effects. |
| `scan_code_vocab` | `folder: String`, `scan_id: String` | `Result<VocabScanSummary, String>` | Breadth-first identifier scan of a project folder with throttled progress events. Returns ranked terms, counts, cap state, and whether the result was adopted. |
| `cancel_code_vocab_scan` | `scan_id: String` | `bool` | Cancels only the matching scan. |
| `get_ide_context_status` | `bundle_id: String` | `IdeContextStatus` | Index state for one profile. |
| `refresh_ide_context` | `bundle_id: String` | `Result<IdeContextStatus, String>` | Rebuilds the memory-only index from that profile's opted-in roots. |
| `clear_ide_context` | `bundle_id: String` | `Result<IdeContextStatus, String>` | Drops the in-memory index for that profile. |

## Permissions (`commands/permissions.rs`)

| Command | Parameters | Returns | Description |
|---------|-----------|---------|-------------|
| `check_accessibility_permission` | — | `bool` | `AXIsProcessTrusted()`. |
| `request_accessibility_permission` | — | `Result<(), String>` | Triggers the system prompt and opens the Accessibility pane. |
| `reset_accessibility_permission` | — | `Result<(), String>` | Clears a stale TCC entry so the grant can be re-made. |
| `check_microphone_permission` | — | `bool` | Whether microphone access is granted. |
| `check_microphone_permission_status` | — | `String` | Fine-grained state (granted / denied / undetermined / restricted) for the onboarding step. |
| `request_microphone_access` | — | `Result<(), String>` | Fires the native in-app prompt via `AVCaptureDevice.requestAccess`. |
| `request_microphone_permission` | — | `Result<(), String>` | Opens the Microphone privacy pane. |
| `reset_microphone_permission` | — | `Result<(), String>` | Clears a stale microphone TCC entry. |
| `open_system_preferences` | — | `Result<(), String>` | Opens System Settings to the Microphone pane. |
| `open_system_audio_preferences` | — | `Result<(), String>` | Opens Privacy & Security → Screen & System Audio Recording. |
| `get_audio_input_inventory` | — | `Result<AudioInputInventorySnapshot, String>` | Exact-main-window-gated shared schema-v1 snapshot: monotonic `revision`, `available` / `stale` / `unavailable` status, stable-ID/display-name descriptors, actual `defaultInputId`, and a bounded error code. The window gate runs before any inventory read or refresh request. A cold read may wait for the one coalesced startup refresh; it never bypasses active capture ownership. |
| `list_audio_devices` | — | `Result<Vec<AudioDeviceDescriptor>, String>` | Exact-main-window-gated compatibility view over the shared inventory. The gate runs before reading the inventory. Returns descriptors only from an authoritative `available` snapshot; stale/unavailable state fails closed and never enumerates per caller. |

## Meeting capture (`commands/meeting.rs`)

| Command | Parameters | Returns | Description |
|---------|-----------|---------|-------------|
| `start_meeting` | `request: {deviceName?, retainAudio, retentionDays?, maxSessions}` | `Result<MeetingSession, String>` | Freezes model/language/punctuation and retention policy, prunes configured history, creates the SQLite session, and starts separate microphone/System Audio capture. Refuses every competing audio/model owner. |
| `stop_meeting` | — | `Result<(), String>` | Requests capture teardown; the worker must destroy the IOProc, aggregate device, and tap before acknowledging. Pending durable chunks continue through serialized inference. |
| `get_meeting_status` | — | `MeetingRuntimeStatus` | Current generation, session, phase, elapsed time, per-channel activity, and stable failure code. |
| `get_system_audio_permission_status` | — | `SystemAudioPermissionState` | Returns the cached `unknown` / `granted` / `denied` / `unsupported` state without creating a tap. |
| `request_system_audio_permission` | — | `Result<SystemAudioPermissionState, String>` | Explicitly creates one short-lived tap probe, then tears it down and emits the resulting permission state. |
| `get_meeting_store_status` | — | `MeetingStoreStatus` | Store availability, schema version, session count, and pending-segment count. |
| `list_meetings` | `query?`, `offset?`, `limit?` | `Result<MeetingPage, String>` | Bounded newest-first list; a non-empty query searches finalized transcript text through FTS5. |
| `get_meeting` | `id` | `Result<MeetingDetail, String>` | One session plus its ordered Me/Them segments and optional validated derived artifact. |
| `get_meeting_export_text` | `id` | `Result<String, String>` | Renders `[MM:SS] Me/Them` plain text for clipboard or validated file export. |
| `delete_meeting` | `id` | `Result<(), String>` | Deletes one inactive session, its segments/FTS rows, and owned chunk audio. |
| `delete_all_meetings` | — | `Result<(), String>` | Deletes all sessions and owned chunk audio; refused while a meeting is active. |
| `prune_meetings` | `retentionDays?`, `maxSessions` | `Result<u64, String>` | Deletes completed/interrupted sessions beyond the bounded age/count policy. |

## Meeting summaries (`commands/meeting_summary.rs`)

| Command | Parameters | Returns | Description |
|---------|-----------|---------|-------------|
| `start_meeting_summary` | `sessionId` | `Result<MeetingSummaryStatus, String>` | Starts explicit, cancellable schema-v1 summary/action-item derivation for a completed meeting through the signed local sidecar; refuses competing owners. |
| `get_meeting_summary_status` | — | `MeetingSummaryStatus` | Current generation, session, phase, bounded chunk progress, elapsed runtime, helper peak RSS, and stable content-free error code. |
| `cancel_meeting_summary` | — | `bool` | Cancels the exact active generation and in-flight sidecar request; a previously stored artifact remains intact. |

## Microphone input test (`commands/microphone_preview.rs`)

These commands are gated to the main window. Preview owns the production audio
supervisor but retains no recording buffer and never enters transcription or
delivery. Live VAD uses only a bounded rolling in-memory window.

| Command | Parameters | Returns | Description |
|---------|-----------|---------|-------------|
| `get_microphone_preview_status` | — | `MicrophonePreviewStatus` | Returns the current generation-aware lifecycle snapshot or retained terminal error. |
| `start_microphone_preview` | `device_id: String`, `vad_sensitivity: u32` | `Result<MicrophonePreviewStatus, String>` | Claims a monotonic Preview owner with the current live-VAD sensitivity, returns its Connecting status immediately, and starts the selected stable device ID asynchronously so startup can be cancelled. `system_default` is the only value normalized to the live default. Refuses competing capture and benchmark owners. |
| `update_microphone_preview_vad_sensitivity` | `preview_id: u64`, `vad_sensitivity: u32` | `Result<bool, String>` | Updates only the exact active preview generation, clamps sensitivity to 0–100, and invalidates an in-flight decision from the previous slider value. |
| `stop_microphone_preview` | `preview_id: u64` | `Result<MicrophonePreviewStatus, String>` | Stops only the exact generation and waits for joined-worker `Idle`; a timeout blocks device reopening. |
| `cancel_microphone_preview` | `preview_id?: u64` | `Result<bool, String>` | Best-effort exact-owner cleanup for page/window teardown. An omitted ID resolves the active preview before cancellation and is a no-op when none exists. |

## Optional integrations (`commands/integrations.rs`)

| Command | Parameters | Returns | Description |
|---------|-----------|---------|-------------|
| `is_notchpill_installed` | — | `bool` | Uses macOS Launch Services to find NotchPill by bundle identifier; always `false` on other platforms. |

## Keyboard (`commands/keyboard.rs`)

| Command | Parameters | Returns | Description |
|---------|-----------|---------|-------------|
| `start_keyboard_listener` | `hotkey: String`, `mode: String` | `Result<(), String>` | Starts the rdev listener in `hold_down`, `double_tap`, or `both`. Validates mode; requires Accessibility. |
| `stop_keyboard_listener` | — | `()` | Stops processing key events; the thread stays alive. |
| `update_keyboard_key` | `hotkey: String` | `()` | Changes the trigger key at runtime. Emits `hold-down-stop` if the old key was held, so no recording is stranded. |
| `set_keyboard_recording` | `recording: bool` | `()` | Syncs recording state into the double-tap detector. |
| `set_app_disabled` | `disabled: bool` | `Result<(), String>` | Global disable/enable. Mirrors state to the tray check item and emits `app-disabled-changed`. |
| `get_app_disabled` | — | `bool` | Current global-disable state. |
| `set_paste_last_shortcut` | `enabled: bool` | `Result<(), String>` | Arms or disarms global ⌘⇧V on the shared rdev listener. Enabling requires Accessibility permission. |
| `start_transform_listener` | `hotkey: String` | `Result<(), String>` | Arms the independent transform hold key. Rejects the active dictation key. |
| `stop_transform_listener` | — | `()` | Disarms the transform key. |
| `set_transform_key` | `hotkey: String` | `Result<(), String>` | Changes the transform key at runtime. |
| `start_query_listener` | `hotkey: String` | `Result<(), String>` | Arms the independent Voice Query double-tap key on the shared rdev thread. Rejects dictation and transform conflicts. |
| `stop_query_listener` | — | `()` | Disarms the query key without stopping the shared listener thread. |

## Delivery recovery (`delivery_recovery.rs`)

| Command | Parameters | Returns | Description |
|---------|-----------|---------|-------------|
| `retry_last_delivery` | — | `Result<RetryResult, String>` | Re-delivers the latest process-memory final text through the normal secure injector. Returns explicit `auto_pasted`, `clipboard_only`, `empty`, `busy`, or `failed` feedback without recording, retranscription, History, statistics, or learning changes. |

## Voice Query (`query_flow.rs`)

| Command | Parameters | Returns | Description |
|---------|-----------|---------|-------------|
| `list_query_provider_presets` | — | `Result<Vec<QueryProviderPreset>, String>` | Main-window-only provider metadata and local executable discovery for Claude, Codex, Grok, Cursor, and Custom. Presets are data; they do not create alternate spawn paths. |
| `load_query_environment` | `provider: QueryProviderId` | `Result<Vec<String>, String>` | Main-window-only list of configured environment **names**. Saved values remain Rust-side and are never returned to a webview. |
| `save_query_environment` | `provider: QueryProviderId`, `variables: Vec<QueryEnvironmentVariable>` | `Result<(), String>` | Main-window-only owner-permission storage for permitted absolute config-directory values. Non-empty entries merge by name; an empty list clears the provider and recovers a malformed/future store by replacing it with an empty current-version store. Base allowlist overrides and secret variable names are rejected. |
| `validate_query_command` | `command: QueryCommandConfig` | `Result<QueryCommandValidation, String>` | Main-window preflight used before enabling the shortcut. Resolves the exact executable and validates argv, timeout, provider, and Rust-owned declared environment. |
| `test_query_provider` | `command: QueryCommandConfig` | `Result<QueryProviderTestResult, String>` | Main-window-only bounded auth probe through `spawn_user_cli`; returns sanitized 16 KiB stdout/stderr tails only to Settings and emits no content telemetry. |
| `launch_query_provider_sign_in` | `command: QueryCommandConfig` | `Result<(), String>` | Main-window-only explicit repair action that opens the preset's interactive login in Terminal. Normal query and probe execution remain direct, shell-free spawns. |
| `launch_query_sign_in_for_pass` | `query_pass_id: u64` | `Result<(), String>` | Query-review-only equivalent using the exact failed pass's immutable provider, executable, and Rust-owned environment. |
| `probe_query_sign_in_for_pass` | `query_pass_id: u64` | `Result<bool, String>` | Query-review-only bounded re-probe used after interactive sign-in; no stdout/stderr content crosses this IPC boundary. |
| `start_query_capture` | `device_name: Option<String>`, `query_pass_id: u64`, `command: QueryCommandConfig` | `Result<(), String>` | Revalidates the configured provider/executable/fixed argv; freezes local ASR, Rust-owned environment, requested `none` / `application` / `selection` context, and `retainQueryHistory` consent for the exact pass; then starts query capture. |
| `finish_query_capture` | `query_pass_id: u64` | `Result<(), String>` | Stops capture, transcribes locally, appends the transcript and frozen context together as one final argv element, and streams bounded stdout to the query popover. |
| `cancel_query` | `query_pass_id: u64` | `Result<(), String>` | Cancels the exact pass, confirms capture/owned process-group teardown, and hides the popover. Stale IDs no-op. |
| `copy_query_answer` | `query_pass_id: u64` | `Result<(), String>` | Copies a completed answer. It never pastes into another app. |
| `get_query_review_content` | — | `QueryReviewContent` | Returns `{queryPassId, answer, errorDetail, provider, usage, signInFix, contextSummary}` only to the `query-review` webview. `usage` contains provider-reported numbers only. `errorDetail` is the bounded stderr tail and remains distinct from answer content; the summary names the app/context kind but never contains window-title or selection text. Every other window receives empty content. |
| `list_query_history` | `offset: Option<u32>`, `limit: Option<u32>`, `provider: Option<QueryProviderId>` | `Result<QueryHistoryPageV1, String>` | Main-window-only, newest-first page from the separate opt-in query store. Defaults to 50 entries and caps requests at 100; it never has a separate context, stderr/detail, executable/argv/environment, or secrets field, though a retained answer may quote context sent to its CLI. |
| `clear_query_history` | — | `Result<(), String>` | Main-window-only direct purge of every retained Voice Query record and recovery artifact. Advances the store clear epoch so an older in-flight pass cannot reinsert content after deletion. |

## Selected-text transform (`transform_flow.rs`, `transform_apply.rs`)

| Command | Parameters | Returns | Description |
|---------|-----------|---------|-------------|
| `start_transform_capture` | `device_name: Option<String>`, `transform_pass_id: u64` | `Result<(), String>` | Begins a pass: arms the mic, freezes the AX selection snapshot, shows the popover in `listening`. Refuses (with a stable error code) when dictation, a benchmark, a file transcription, or another transform owns the pipeline. |
| `finish_transform_instruction` | `transform_pass_id: u64` | `Result<(), String>` | Stops the instruction mic, transcribes it (cleanup-only), expands preset/saved-transform names, and runs the sidecar. `listening` → `thinking` → `ready`/`failed`. |
| `retry_transform_instruction` | `device_name: Option<String>` | `Result<(), String>` | Re-arms listening for a new instruction against the **same** frozen selection, keeping the pass ID and advancing the attempt counter. |
| `approve_transform` | — | `Result<(), String>` | Applies the proposal through `transform_apply` (AX set-value, else paste fallback with clipboard restore) and schedules the linger-hide. |
| `cancel_transform` | `transform_pass_id: Option<u64>` | `Result<(), String>` | Scoped cancellation. A no-op if that pass no longer owns the flow, so a delayed Escape cannot cancel the next pass. Idempotent. |
| `undo_transform_and_close` | — | `Result<(), String>` | Restores the frozen original and closes the popover. On failure the Applied session is kept and `applied` is re-emitted with an error code so Undo stays available. |
| `apply_transform_result` | — | `Result<String, String>` | Lower-level write-back entry point. |
| `undo_transform` | — | `Result<(), String>` | Lower-level undo entry point. |

## Transform review popover (`commands/transform_popover.rs`)

| Command | Parameters | Returns | Description |
|---------|-----------|---------|-------------|
| `get_transform_popover_geometry` | `anchor: Option<Rect>` | `TransformPopoverGeometry` | `{compact, expanded}` boxes resolved against the selection anchor and the active screen — 8px below, flipped above when it would clip, clamped horizontally, or centered at 38% screen height with no anchor. Pure `popover_geometry_for()`, asserted by a shared fixture. |
| `show_transform_popover` | `anchor: Option<Rect>` | `Result<(), String>` | Sizes/positions to the compact box, applies the non-activating window treatment, shows it, and caches the anchor. |
| `hide_transform_popover` | — | `Result<(), String>` | Hides the popover. |
| `set_transform_popover_expanded` | `expanded: bool` | `Result<PopoverBox, String>` | Resizes between compact (listening/thinking) and expanded (ready/failed) against the cached anchor; returns the applied box as an acknowledgment. |
| `set_transform_popover_focusable` | `focusable: bool` | `Result<(), String>` | `false` during listening/thinking so focus is never stolen; `true` at ready/failed so Enter/Esc/Cmd+R reach the webview. |
| `get_transform_review_content` | — | `TransformReviewContent` | `{instruction, original, proposed}`. Fetched on each state change rather than broadcast, so sensitive text never rides an event payload. |

## Transform model (`commands/transform_model.rs`)

| Command | Parameters | Returns | Description |
|---------|-----------|---------|-------------|
| `transform_model_status` | — | `TransformModelStatus` | Install state, size, and platform support for the pinned local LLM. |
| `download_transform_model` | — | `Result<(), String>` | Streams the pinned GGUF to a `.partial`, hashes while streaming, fsyncs, then atomically publishes under its SHA-256 directory. Exact size and hash are enforced. |
| `remove_transform_model` | — | `Result<(), String>` | Shuts the helper down first, then deletes the hash directory and any partial. |
| `reset_transform_runtime` | — | `()` | Clears the circuit breaker after repeated helper faults. |

## Transform diagnostics (`commands/transform_diagnostics.rs`)

| Command | Parameters | Returns | Description |
|---------|-----------|---------|-------------|
| `list_transform_attempts` | `limit: Option<usize>` | `Result<TransformAttemptListV1, String>` | Bounded, content-free per-pass records including refused, cancelled, and superseded passes. |
| `arm_next_transform_diagnostic_capture` | — | `Result<CaptureArmStatusV1, String>` | Arms one consented content capture. In-memory only, single pass, 10-minute expiry. |
| `get_transform_diagnostic_capture_status` | — | `Result<CaptureArmStatusV1, String>` | Whether an arm is live and when it expires. |
| `list_transform_diagnostic_captures` | — | `Result<Vec<DiagnosticCaptureSummaryV1>, String>` | Stored captures (max 3, 7-day expiry). |
| `get_transform_diagnostic_capture` | `capture_id: String` | `Result<Option<DiagnosticCaptureV1>, String>` | Full capture for in-app review. There is no export path. |
| `delete_transform_diagnostic_capture` | `capture_id: String` | `Result<(), String>` | Deletes one capture. |

## Models (`commands/models.rs`)

| Command | Parameters | Returns | Description |
|---------|-----------|---------|-------------|
| `check_model_exists` | — | `bool` | Whether any transcription model is installed (drives first-launch routing). |
| `check_specific_model_exists` | `model_name: String` | `bool` | Whether a named model is on disk. Path-traversal protected. |
| `get_model_runtime_catalog` | — | `Vec<ModelRuntimeSnapshot>` | Every catalog entry with backend, accelerator, capabilities, platform support, install state, and lifecycle state. |
| `get_model_runtime_status` | `model_name: String` | `Result<ModelRuntimeSnapshot, String>` | Snapshot for one model. Unknown identifiers error. |
| `download_model` | `model_name: String` | `Result<(), String>` | Streaming download with `download-progress` events, atomic publication, and Silero VAD co-download. Core ML installs show an indeterminate Installing state during extraction/compilation. |

## Personal knowledge (`commands/knowledge.rs`)

| Command | Parameters | Returns | Description |
|---------|-----------|---------|-------------|
| `get_knowledge_store_status` | — | `KnowledgeStoreStatus` | Ready / recovered / reinitialized / unavailable, schema version, record count, store revision, privacy-safe recovery info. |
| `retry_knowledge_store` | — | `KnowledgeStoreStatus` | Re-runs local initialization after an unavailable state. |
| `list_knowledge` | `request: KnowledgeListRequest` | `Result<KnowledgeListResponse, String>` | Bounded search/filter page (default 50, cap 100), with an optional Voice Command filter. |
| `get_knowledge` | `id: String` | `Result<KnowledgeEntry, String>` | One record by stable ID. |
| `upsert_knowledge` | `draft: KnowledgeDraft` | `Result<KnowledgeEntry, String>` | Creates or edits with an expected revision. Voice Commands additionally validate payload/type, scope, built-ins, duplicate phrases, variables, clipboard permission, and vocabulary conflicts. |
| `set_knowledge_enabled` | `id`, `enabled`, `expected_revision` | `Result<KnowledgeEntry, String>` | Enable/disable with optimistic concurrency. |
| `delete_knowledge` | `id`, `expected_revision` | `Result<u64, String>` | Deletes one record, returns the new store revision. |
| `resolve_knowledge` | `request: KnowledgeResolveRequest` | `Result<Option<KnowledgeEntry>, String>` | Deterministic exact-trigger resolution using the same scope/provenance precedence that feeds the Smart Correction matcher. |
| `preview_voice_command` | `request: VoiceCommandPreviewRequest` | `Result<VoiceCommandPreviewResponse, String>` | Runs the real matcher and variable expansion with no clipboard output and no paste. Clipboard input requires both saved permission and an explicit preview request. |
| `export_knowledge_to_file` | `path: String` | `Result<u64, String>` | Atomic export to versioned JSON. |
| `inspect_knowledge_import` | `path: String` | `Result<KnowledgeImportSummary, String>` | Validates an import and reports new / duplicate / conflicting records without writing. |
| `import_knowledge_from_file` | `path: String` | `Result<KnowledgeImportResult, String>` | Atomically imports validated new records; never overwrites local records. |
| `delete_all_knowledge` | `expected_revision: u64` | `Result<u64, String>` | Deletes all records and in-store recovery artifacts after typed confirmation. |

## Appearance file transport (`commands/theme.rs`)

These commands are strictly gated to the main webview. The native dialog
plugin selects a path; these commands perform only bounded transport. Theme
schema validation, preview, resolution, and cache generation stay in the
frontend.

| Command | Parameters | Returns | Description |
|---------|-----------|---------|-------------|
| `read_theme_file` | `path: String` | `Result<String, String>` | Reads at most 256 KiB of UTF-8 from a regular, non-symlink file, enough for bounded VS Code JSON/JSONC themes. Returns clear errors for invalid paths, oversized files, invalid UTF-8, and I/O failure. |
| `write_theme_file` | `path: String`, `contents: String` | `Result<(), String>` | Rejects content over 64 KiB and symlink/non-regular destinations, writes and fsyncs a unique sibling temporary file, atomically renames it over the destination, then syncs the parent directory. New Unix files use mode `0600`; replacements preserve the existing regular target's permissions. A failed write/publish cleans the temporary file and preserves an existing target. |

## Correct and Teach (`commands/correct_and_teach.rs`)

| Command | Parameters | Returns | Description |
|---------|-----------|---------|-------------|
| `propose_learned_correction` | `request: CorrectionProposalRequest` | `CorrectionProposalOutcome` | Computes one bounded local diff and stores an ephemeral proposal. Never writes knowledge. Ambiguous alignments fail closed. |
| `propose_specific_learned_correction` | `request: SpecificCorrectionProposalRequest` | `CorrectionProposalOutcome` | Validates one user-selected whole-term replacement, counts and previews exact matches locally, stores an ephemeral proposal. |
| `confirm_learned_correction` | `proposal_id: u64`, `scope: KnowledgeScope` | `Result<KnowledgeEntry, String>` | Persists the reviewed replacement with `learned_correction` provenance and refreshes the next matcher generation. |
| `discard_learned_correction_proposal` | `proposal_id: u64` | `()` | Discards the proposal without persistence. |

## Performance Lab (`commands/benchmark.rs`)

| Command | Parameters | Returns | Description |
|---------|-----------|---------|-------------|
| `get_benchmark_models` | — | `Vec<BenchmarkModel>` | Installed models eligible for benchmarking. |
| `get_benchmark_activity` | — | `BenchmarkActivity` | Whether a run is active, for busy-state isolation against dictation and transform. |
| `run_benchmark` | `request: BenchmarkRequest` | `Result<BenchmarkReport, String>` | Runs the selected models over the fixture corpus, emitting `benchmark-progress`. Reports raw, normalized, and delivered WER plus latency, realtime factor, and memory, with privacy-safe environment/corpus/execution metadata. |
| `cancel_benchmark` | — | `bool` | Cancels the active run. |
| `save_benchmark_report` | `report_json`, `output_dir`, `file_name` | `Result<String, String>` | Writes a report as `benchmark-<version>-<machine>-<createdAt>.json`. |
| `open_benchmark_output_folder` | `output_dir: String` | `Result<(), String>` | Reveals the report folder in Finder. |
| `run_microphone_startup_benchmark` | `request: MicrophoneStartupBenchmarkRequest {runId, deviceId, cycles}` | `Result<MicrophoneStartupBenchmarkReport, String>` | Main-window-only bounded production-capture startup diagnostic (1–10 cycles). Uses an exact `MicrophoneBenchmark` owner, preserves the immutable production backend order without training or persisting its per-device backend memo, emits correlated progress, retains no PCM, and resolves only after post-join Idle. |
| `cancel_microphone_startup_benchmark` | `run_id: String` | `Result<bool, String>` | Cancels only the matching microphone benchmark UUID. Returns false for no owner or a stale run ID and waits through the run command for confirmed teardown. |
| `save_microphone_startup_benchmark_report` | `report: MicrophoneStartupBenchmarkReport`, `output_dir: String` | `Result<String, String>` | Main-window-only typed export. Revalidates the exact schema/cross-fields and 128 KiB cap, then writes `murmur-microphone-startup-<UTC timestamp>-<run UUID>.json` without accepting a caller-controlled filename. |

## Performance diagnostics (`commands/performance.rs`)

| Command | Parameters | Returns | Description |
|---------|-----------|---------|-------------|
| `list_performance_runs` | `limit: Option<u32>` | `Result<PerformanceRunListV1, String>` | Completed runs, newest first (cap 200). |
| `get_performance_run` | `run_id: String` | `Result<Option<PerformanceRunV1>, String>` | One run with stage timings, warm state, RSS deltas, and transform follow-ups. |
| `get_performance_resource_window` | — | `Result<Vec<ResourceSampleV1>, String>` | The rolling CPU/memory sample window (cap 600). |
| `get_performance_store_health` | — | `Result<PerformanceStoreHealthV1, String>` | Main/Diagnostics-window-only content-free store availability, skipped-run count, last bounded failure/recovery evidence, and recommended local action. |
| `recover_performance_store` | `allow_reinitialize: bool` | `Result<PerformanceStoreHealthV1, String>` | Main/Diagnostics-window-only explicit reopen/check. Quarantine/reinitialize additionally requires confirmed caller intent plus fresh corruption or invalid-record evidence; a healthy database is never deleted. |
| `get_capture_health_history` | — | `CaptureHealthHistoryV1` | The 20 newest finalized dictation startup observations. Each contains only `startupMs`, `usedFallback`, and an optional allowlisted fallback backend enum. |
| `clear_performance_diagnostics` | — | `Result<(), String>` | Deletes local run history, samples, and capture-startup observations; emits `performance-diagnostics-cleared`. |
| `show_diagnostics_window` | `tab: String` | `Result<(), String>` | Shows and focuses the persistent Diagnostics window, selecting one of its exact allowlisted tabs. |

## Logging (`commands/logging.rs`)

| Command | Parameters | Returns | Description |
|---------|-----------|---------|-------------|
| `get_log_contents` | `lines: usize` | `String` | Last N lines of the pretty log file. |
| `clear_logs` | — | `Result<(), String>` | Removes log files (including rotated and JSONL variants) and clears the ring buffer. |
| `log_frontend` | `level`, `message`, `transform_pass_id: Option<u64>` | `()` | Routes a frontend message through Rust tracing with `source="frontend"`, optionally correlated to a transform pass. |

## Export (`commands/export.rs`)

| Command | Parameters | Returns | Description |
|---------|-----------|---------|-------------|
| `save_text_export` | `path: String`, `contents: String` | `Result<u64, String>` | Writes a user-authored text export (transcript history today) to a path chosen in the native save dialog, returning bytes written. Refuses relative paths, directories, dotfiles, missing parents, extensions outside `.json`/`.md`/`.txt`, and payloads over 8 MB. The write is atomic (temp sibling, then rename). |

## Durable frontend data store (`commands/settings_store.rs`)

The durable home for frontend-owned settings, bounded transcript history,
usage statistics, and the saved theme library under the per-bundle app data directory. Blobs are opaque to
Rust: only their size and top-level JSON container are checked, so schema and
migration rules stay in TypeScript. Writes are atomic, serialized, and created
with owner-only permissions on Unix; rejected files are quarantined locally
without logging their content.

| Command | Parameters | Returns | Description |
|---------|-----------|---------|-------------|
| `load_settings_blob` | — | `Result<Option<String>, String>` | Reads `settings.json`, creating the directory if needed. `None` when the file is absent, or when it was over 1 MiB, not UTF-8, not valid JSON, or not a JSON object — those are renamed to `settings.json.corrupt-<unix-seconds>` and never deleted, so the caller falls back to its localStorage cache. `Err` is reserved for filesystem failures. |
| `save_settings_blob` | `blob: String` | `Result<(), String>` | Refuses anything over 1 MiB or not a JSON object, then publishes atomically (temp sibling, then rename). Concurrent writers (main and overlay windows) are serialized; a failed write removes the temp file and preserves the previous settings. |
| `load_history_blob` | — | `Result<Option<String>, String>` | Reads `history.json`. `None` means absent or quarantined; the 8 MiB JSON-array bound is checked before returning content. |
| `save_history_blob` | `blob: String` | `Result<(), String>` | Atomically publishes a history JSON array up to 8 MiB. |
| `clear_history_blob` | — | `Result<(), String>` | Idempotently removes `history.json` without touching settings or stats. |
| `load_stats_blob` | — | `Result<Option<String>, String>` | Reads `stats.json`. `None` means absent or quarantined; the 1 MiB JSON-object bound is checked before returning content. |
| `save_stats_blob` | `blob: String` | `Result<(), String>` | Atomically publishes a statistics JSON object up to 1 MiB. |
| `clear_stats_blob` | — | `Result<(), String>` | Idempotently removes `stats.json` without touching settings or history. |
| `load_theme_library_blob` | — | `Result<Option<String>, String>` | Main-window-only read of `theme-library.json`. `None` means absent or quarantined; the 1 MiB JSON-object boundary is enforced before content reaches the renderer. |
| `save_theme_library_blob` | `blob: String` | `Result<(), String>` | Main-window-only atomic publish of a theme-library JSON object up to 1 MiB. Rust treats the schema as opaque; TypeScript validates every entry and revision. |
| `clear_theme_library_blob` | — | `Result<(), String>` | Main-window-only, idempotent removal of `theme-library.json`, independent of settings, history, stats, and the active appearance cache. |

## Overlay (`commands/overlay.rs`)

| Command | Parameters | Returns | Description |
|---------|-----------|---------|-------------|
| `get_overlay_geometry` | — | `OverlayGeometry` | The geometry contract (window/pill/dropdown boxes) derived from the cached notch by `geometry_for()`. Never null — a synthetic fallback notch substitutes when none is detected. |
| `show_overlay` | — | `Result<(), String>` | Shows the overlay at its current position and re-enables mouse events (disabled by `focusable:false`); it deliberately does not reset an in-progress calibration preview. |
| `hide_overlay` | — | `Result<(), String>` | Hides the overlay. |
| `set_overlay_expanded` | `expanded: bool` | `Result<AppliedSurface, String>` | Resizes between collapsed and expanded frames (top-anchored) and returns the applied frame, so CSS never animates into a window that hasn't grown yet. |
| `set_overlay_vertical_offset` | `offset: f64` | `Result<(), String>` | Clamps calibration to ±12 logical points, moves the actual native overlay window relative to the active monitor's top-center default, and logs target plus applied physical coordinates. The command is non-persistent; the frontend persists only Save and inactive Reset actions. |
| `show_main_window` | — | `Result<(), String>` | Shows and focuses the main window — used by the overlay's gear button instead of granting the overlay broad window permissions. |

## Frontmost apps (`frontmost.rs`)

| Command | Parameters | Returns | Description |
|---------|-----------|---------|-------------|
| `list_running_applications` | — | `Vec<RunningApplication>` | Bounded memory-only list of running apps (bundle ID + name) for the per-app profile picker. Empty on non-macOS. |

## Tray (`commands/tray.rs`)

| Command | Parameters | Returns | Description |
|---------|-----------|---------|-------------|
| `update_tray_icon` | `_icon_state: String` | `Result<(), String>` | No-op; the tray icon is always the static white waveform. Retained for API compatibility. |
| `set_tray_update_available` | `version: Option<String>` | `Result<(), String>` | Changes the native menu item between `Check for Updates…` and a bounded, validated `Update Murmur to vX.Y.Z…` label. |

## Native Modes (`commands/mode_runtime.rs`)

| Command | Parameters | Returns | Description |
|---------|-----------|---------|-------------|
| `get_mode_runtime_status` | — | `ModeRuntimeStatus` | Resolves the current frontmost bundle to the temporary override, app binding, or last manual Mode. |
| `cycle_mode` | — | `ModeRuntimeStatus` | Cycles enabled Modes without focusing Murmur; persists a manual selection outside bindings or creates a memory-only bundle override inside one. |
| `clear_temporary_mode_override` | — | `ModeRuntimeStatus` | Clears the app-scoped override and returns to its binding or the last manual Mode. |

## Updater (`commands/updater.rs`)

| Command | Parameters | Returns | Description |
|---------|-----------|---------|-------------|
| `get_update_install_environment` | — | `Result<{ appTranslocated: bool }, String>` | Reports whether Gatekeeper launched the current executable through its read-only `AppTranslocation` mount. Does not expose the executable path and fails closed if the current executable cannot be resolved. |
| `updater_canary` | `request: { action: "read" | "write", result?: JSON }` | `Result<{ path: string | null, result: JSON | null }, String>` | Reads the opt-in `MURMUR_UPDATER_CANARY` result path or atomically writes the app's canary result. With no environment marker, returns an inert state. |

## Telemetry (`telemetry.rs`)

| Command | Parameters | Returns | Description |
|---------|-----------|---------|-------------|
| `get_event_history` | — | `Vec<AppEvent>` | The in-memory ring buffer (up to 500 events). |
| `clear_event_history` | — | `()` | Clears the ring buffer. Does not delete the JSONL file. |

## Resource monitor (`resource_monitor.rs`)

| Command | Parameters | Returns | Description |
|---------|-----------|---------|-------------|
| `get_resource_usage` | — | `ResourceSampleV1` | Current CPU percentage, process RSS, and separated Rust-heap / FFI-heap figures (the custom malloc zone keeps whisper.cpp's allocations out of the Rust total). |
