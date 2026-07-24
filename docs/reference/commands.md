# Tauri Commands Reference

The 105 commands registered in `lib.rs` and exposed to the frontend via `invoke()`, grouped by source module under `app/src-tauri/src/`.

Parameters are listed with their Rust names; the frontend passes them camelCased (`model_name` → `modelName`). `app_handle` / `state` / `window` injections are omitted — they are supplied by Tauri, not by the caller.

For Rust → frontend events see [events.md](events.md). For the hooks that call these commands see [hooks.md](hooks.md).

---

## Recording and dictation (`commands/recording.rs`)

| Command | Parameters | Returns | Description |
|---------|-----------|---------|-------------|
| `init_dictation` | — | `Result<JSON, String>` | Static `{"type":"initialized","state":"idle"}` marker. |
| `get_status` | — | `Result<JSON, String>` | Current status, model name, and language. |
| `configure_dictation` | `options: JSON` | `Result<JSON, String>` | Pushes settings into `DictationState`: model, language, auto-paste and delay (clamped 10–500), VAD sensitivity (0–100), punctuation, file output, vocabulary entries, voice commands, app profiles, cleanup/formatting/correction toggles, code-vocab folder, idle timeout. A model change reselects the runtime backend, deferring across an active recording generation. Rejects conflicting vocabulary/command configurations without mutating prior state. |
| `start_native_recording` | `device_name: Option<String>` | `Result<JSON, String>` | Starts cpal capture, resolves the immutable per-recording context, and warms the model concurrently. Idle → Recording. |
| `stop_native_recording` | — | `Result<JSON, String>` | Stops capture and runs VAD → inference → transcript transform → delivery. Recordings under 0.3s are discarded. |
| `cancel_native_recording` | — | `Result<(), String>` | Discards an in-progress recording without transcribing (used for speculative Both-mode holds). |
| `process_audio` | `audio_data: String` | `Result<JSON, String>` | Runs the full pipeline over base64-encoded 16kHz mono WAV. |
| `transcribe_file` | `file_path: String` | `Result<JSON, String>` | Decodes and transcribes an audio file through the same pipeline with live-only stages skipped. Emits `file-transcription-status-changed`. |
| `transform_status` | — | `TransformStatus` | Current selected-text transform state (used to arbitrate against dictation). |
| `count_vocab_tokens` | `text: String` | `Result<Option<usize>, String>` | Token count for the loaded model's tokenizer; `None` if no model is loaded. Drives the Whisper prompt budget UI. |
| `preview_vocabulary_aliases` | `entries`, `voice_commands`, `text`, `cli_formatting` | `Result<String, String>` | Runs alias + command resolution over sample text in memory. No persistence, no delivery. |
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
| `list_audio_devices` | — | `Result<Vec<String>, String>` | Input device names via cpal. |

## Keyboard (`commands/keyboard.rs`)

| Command | Parameters | Returns | Description |
|---------|-----------|---------|-------------|
| `start_keyboard_listener` | `hotkey: String`, `mode: String` | `Result<(), String>` | Starts the rdev listener in `hold_down`, `double_tap`, or `both`. Validates mode; requires Accessibility. |
| `stop_keyboard_listener` | — | `()` | Stops processing key events; the thread stays alive. |
| `update_keyboard_key` | `hotkey: String` | `()` | Changes the trigger key at runtime. Emits `hold-down-stop` if the old key was held, so no recording is stranded. |
| `set_keyboard_recording` | `recording: bool` | `()` | Syncs recording state into the double-tap detector. |
| `set_app_disabled` | `disabled: bool` | `Result<(), String>` | Global disable/enable. Mirrors state to the tray check item and emits `app-disabled-changed`. |
| `get_app_disabled` | — | `bool` | Current global-disable state. |
| `start_transform_listener` | `hotkey: String` | `Result<(), String>` | Arms the independent transform hold key. Rejects the active dictation key. |
| `stop_transform_listener` | — | `()` | Disarms the transform key. |
| `set_transform_key` | `hotkey: String` | `Result<(), String>` | Changes the transform key at runtime. |

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

## Performance diagnostics (`commands/performance.rs`)

| Command | Parameters | Returns | Description |
|---------|-----------|---------|-------------|
| `list_performance_runs` | `limit: Option<u32>` | `Result<PerformanceRunListV1, String>` | Completed runs, newest first (cap 200). |
| `get_performance_run` | `run_id: String` | `Result<Option<PerformanceRunV1>, String>` | One run with stage timings, warm state, RSS deltas, and transform follow-ups. |
| `get_performance_resource_window` | — | `Result<Vec<ResourceSampleV1>, String>` | The rolling CPU/memory sample window (cap 600). |
| `clear_performance_diagnostics` | — | `Result<(), String>` | Deletes local run history and samples; emits `performance-diagnostics-cleared`. |

## Logging (`commands/logging.rs`)

| Command | Parameters | Returns | Description |
|---------|-----------|---------|-------------|
| `get_log_contents` | `lines: usize` | `String` | Last N lines of the pretty log file. |
| `clear_logs` | — | `Result<(), String>` | Removes log files (including rotated and JSONL variants) and clears the ring buffer. |
| `log_frontend` | `level`, `message`, `transform_pass_id: Option<u64>` | `()` | Routes a frontend message through Rust tracing with `source="frontend"`, optionally correlated to a transform pass. |
| `open_log_viewer` | — | `Result<(), String>` | Shows and focuses the log-viewer window. |

## Overlay (`commands/overlay.rs`)

| Command | Parameters | Returns | Description |
|---------|-----------|---------|-------------|
| `get_overlay_geometry` | — | `OverlayGeometry` | The geometry contract (window/pill/dropdown boxes) derived from the cached notch by `geometry_for()`. Never null — a synthetic fallback notch substitutes when none is detected. |
| `show_overlay` | — | `Result<(), String>` | Positions and shows the overlay; re-enables mouse events (disabled by `focusable:false`). |
| `hide_overlay` | — | `Result<(), String>` | Hides the overlay. |
| `set_overlay_expanded` | `expanded: bool` | `Result<AppliedSurface, String>` | Resizes between collapsed and expanded frames (top-anchored) and returns the applied frame, so CSS never animates into a window that hasn't grown yet. |
| `show_main_window` | — | `Result<(), String>` | Shows and focuses the main window — used by the overlay's gear button instead of granting the overlay broad window permissions. |

## Frontmost apps (`frontmost.rs`)

| Command | Parameters | Returns | Description |
|---------|-----------|---------|-------------|
| `list_running_applications` | — | `Vec<RunningApplication>` | Bounded memory-only list of running apps (bundle ID + name) for the per-app profile picker. Empty on non-macOS. |

## Tray (`commands/tray.rs`)

| Command | Parameters | Returns | Description |
|---------|-----------|---------|-------------|
| `update_tray_icon` | `_icon_state: String` | `Result<(), String>` | No-op; the tray icon is always the static white waveform. Retained for API compatibility. |

## Telemetry (`telemetry.rs`)

| Command | Parameters | Returns | Description |
|---------|-----------|---------|-------------|
| `get_event_history` | — | `Vec<AppEvent>` | The in-memory ring buffer (up to 500 events). |
| `clear_event_history` | — | `()` | Clears the ring buffer. Does not delete the JSONL file. |

## Resource monitor (`resource_monitor.rs`)

| Command | Parameters | Returns | Description |
|---------|-----------|---------|-------------|
| `get_resource_usage` | — | `ResourceSampleV1` | Current CPU percentage, process RSS, and separated Rust-heap / FFI-heap figures (the custom malloc zone keeps whisper.cpp's allocations out of the Rust total). |
