# Tauri Commands Reference

This document lists the registered Tauri commands exposed from the Rust backend to the frontend via `invoke()`. Commands are grouped by their source module under `app/src-tauri/src/`.

For event-based communication (Rust to frontend), see [events.md](events.md). For frontend hooks that call these commands, see [hooks.md](hooks.md).

---

## Recording (`commands/recording.rs`)

| Command | Parameters | Return Type | Description |
|---------|-----------|-------------|-------------|
| `init_dictation` | _(none)_ | `Result<JSON, String>` | Returns a static `{"type":"initialized","state":"idle"}` response. No-op initialization marker. |
| `process_audio` | `audio_data: String` | `Result<JSON, String>` | Accepts base64-encoded WAV audio, decodes it, runs the full VAD + transcription + text injection pipeline, and returns `{"type":"transcription","text":"..."}`. |
| `get_status` | _(none)_ | `Result<JSON, String>` | Returns current dictation status, model name, and language as `{"type":"status","state":"...","model":"...","language":"..."}`. |
| `configure_dictation` | `options: JSON` | `Result<JSON, String>` | Updates dictation settings. Accepts optional fields: `model` (string), `language` (string), `autoPaste` (bool), `autoPasteDelayMs` (u64, clamped 10-500), `vadSensitivity` (u64, clamped 0-100). Resets the transcription backend if model changes. |
| `start_native_recording` | `device_name: Option<String>` | `Result<JSON, String>` | Begins native audio capture via cpal with an optional device name. Transitions status from Idle to Recording. Returns early if already recording or processing. |
| `stop_native_recording` | _(none)_ | `Result<JSON, String>` | Stops audio capture, runs the full pipeline (VAD, transcription, text injection), and returns the transcription result. Recordings shorter than 0.3s are silently discarded. |
| `cancel_native_recording` | _(none)_ | `Result<(), String>` | Cancels an in-progress recording without transcribing. Audio is discarded. Used by "both" mode for speculative recordings from short taps. |

## Permissions (`commands/permissions.rs`)

| Command | Parameters | Return Type | Description |
|---------|-----------|-------------|-------------|
| `open_system_preferences` | _(none)_ | `Result<(), String>` | Opens macOS System Settings to the Microphone privacy pane. |
| `check_accessibility_permission` | _(none)_ | `bool` | Returns `true` if macOS Accessibility permission is granted (via `AXIsProcessTrusted()`). |
| `request_accessibility_permission` | _(none)_ | `Result<(), String>` | Triggers the macOS Accessibility permission prompt and opens System Settings to the Accessibility pane. |
| `request_microphone_permission` | _(none)_ | `Result<(), String>` | Opens macOS System Settings to the Microphone privacy pane. |
| `list_audio_devices` | _(none)_ | `Result<Vec<String>, String>` | Returns a list of available audio input device names via cpal. |

## Keyboard (`commands/keyboard.rs`)

| Command | Parameters | Return Type | Description |
|---------|-----------|-------------|-------------|
| `start_keyboard_listener` | `hotkey: String`, `mode: String` | `Result<(), String>` | Starts the global rdev keyboard listener with the specified hotkey and mode (`"double_tap"`, `"hold_down"`, or `"both"`). Validates mode and requires Accessibility permission. |
| `stop_keyboard_listener` | _(none)_ | `()` | Stops processing keyboard events. The rdev listener thread remains alive but idle. |
| `update_keyboard_key` | `hotkey: String` | `()` | Changes the target hotkey at runtime without restarting the listener. If the key is changed while held down, emits `hold-down-stop` to prevent stuck recording state. |
| `set_keyboard_recording` | `recording: bool` | `()` | Synchronizes the keyboard module's internal recording state flag. Used by the frontend to keep the double-tap detector's state machine in sync. |

## Logging (`commands/logging.rs`)

| Command | Parameters | Return Type | Description |
|---------|-----------|-------------|-------------|
| `get_log_contents` | `lines: usize` | `String` | Returns the last N lines from the pretty-printed log file (`app.log` or `app.dev.log`). |
| `clear_logs` | _(none)_ | `Result<(), String>` | Removes all log files (including rotated variants, JSONL event files, frontend logs) and clears the in-memory event ring buffer. |
| `log_frontend` | `level: String`, `message: String` | `()` | Routes a frontend log message through the Rust tracing system. Accepts levels: `"INFO"`, `"WARN"`, `"ERROR"`. Messages appear in the structured event stream with `source="frontend"`. |
| `open_log_viewer` | _(none)_ | `Result<(), String>` | Shows and focuses the `log-viewer` window. |

## Personal Knowledge (`commands/knowledge.rs`)

| Command | Parameters | Return Type | Description |
|---------|-----------|-------------|-------------|
| `get_knowledge_store_status` | _(none)_ | `KnowledgeStoreStatus` | Returns ready/recovered/reinitialized/unavailable state, schema version, record count, store revision, and privacy-safe recovery information. |
| `retry_knowledge_store` | _(none)_ | `KnowledgeStoreStatus` | Re-runs local initialization after an unavailable state. |
| `list_knowledge` | `request: KnowledgeListRequest` | `Result<KnowledgeListResponse, String>` | Bounded search/filter page; defaults to 50 and caps at 100 records. |
| `get_knowledge` | `id: String` | `Result<KnowledgeEntry, String>` | Returns one local record by stable ID. |
| `upsert_knowledge` | `draft: KnowledgeDraft` | `Result<KnowledgeEntry, String>` | Creates a manual record or edits one using its expected revision. |
| `set_knowledge_enabled` | `id`, `enabled`, `expected_revision` | `Result<KnowledgeEntry, String>` | Enables/disables one record with optimistic concurrency. |
| `delete_knowledge` | `id`, `expected_revision` | `Result<u64, String>` | Deletes one record and returns the new store revision. |
| `resolve_knowledge` | `request: KnowledgeResolveRequest` | `Result<Option<KnowledgeEntry>, String>` | Deterministically resolves an exact trigger across applicable scopes. Enabled replacement records also feed the immutable Smart Correction matcher. |
| `export_knowledge_to_file` | `path: String` | `Result<u64, String>` | Atomically exports the local store to versioned JSON selected by the user. |
| `inspect_knowledge_import` | `path: String` | `Result<KnowledgeImportSummary, String>` | Validates an import and reports new, duplicate, and conflicting records without writing. |
| `import_knowledge_from_file` | `path: String` | `Result<KnowledgeImportResult, String>` | Atomically imports validated new records without overwriting local records. |
| `delete_all_knowledge` | `expected_revision: u64` | `Result<u64, String>` | Deletes all records and in-store recovery artifacts after a revision-checked UI confirmation. |

## Correct and Teach (`commands/correct_and_teach.rs`)

| Command | Parameters | Return Type | Description |
|---------|-----------|-------------|-------------|
| `propose_learned_correction` | `request: CorrectionProposalRequest` | `CorrectionProposalOutcome` | Computes one bounded local diff and stores only an ephemeral reviewed proposal. It never writes knowledge. |
| `confirm_learned_correction` | `proposal_id`, `scope` | `Result<KnowledgeEntry, String>` | Persists the exact reviewed replacement with `learned_correction` provenance and refreshes the next matcher generation. |
| `discard_learned_correction_proposal` | `proposal_id` | `()` | Discards the matching ephemeral proposal without persistence. |

## Models (`commands/models.rs`)

| Command | Parameters | Return Type | Description |
|---------|-----------|-------------|-------------|
| `check_model_exists` | _(none)_ | `bool` | Returns `true` if any transcription model exists. Used to determine whether the model download screen should be shown on first launch. |
| `check_specific_model_exists` | `model_name: String` | `bool` | Returns `true` if the specified model file or directory exists on disk. Includes path traversal protection (rejects `..`, `/`, `\` in model names). |
| `download_model` | `model_name: String` | `Result<(), String>` | Downloads a transcription model with streaming progress events. Allowed models: `large-v3-turbo`, `small.en`, `base.en`, `tiny.en`, `medium.en`. Also co-downloads the Silero VAD model if missing. Whisper models are downloaded as single `.bin` files from Hugging Face. |

## Tray (`commands/tray.rs`)

| Command | Parameters | Return Type | Description |
|---------|-----------|-------------|-------------|
| `update_tray_icon` | `_icon_state: String` | `Result<(), String>` | No-op. The tray icon is always a static white waveform. Command is retained for API compatibility. |

## Overlay (`commands/overlay.rs`)

| Command | Parameters | Return Type | Description |
|---------|-----------|-------------|-------------|
| `show_overlay` | _(none)_ | `Result<(), String>` | Positions and shows the always-on-top overlay window at the macOS notch area. Re-enables mouse events (disabled by `focusable:false`). |
| `hide_overlay` | _(none)_ | `Result<(), String>` | Hides the overlay window. Gracefully handles missing window. |
| `get_notch_info` | _(none)_ | `Option<NotchInfo>` | Returns cached notch dimensions as `{notch_width: f64, notch_height: f64}`, or `null` if no notch is detected. Dimensions are detected via NSScreen APIs. |

## Telemetry (`telemetry.rs`)

| Command | Parameters | Return Type | Description |
|---------|-----------|-------------|-------------|
| `get_event_history` | _(none)_ | `Vec<AppEvent>` | Returns all entries from the in-memory structured event ring buffer (up to 500 events). Each event has `timestamp`, `stream`, `level`, `summary`, and `data` fields. |
| `clear_event_history` | _(none)_ | `()` | Clears the in-memory event ring buffer. Does not delete the JSONL file on disk. |

## Resource Monitor (`resource_monitor.rs`)

| Command | Parameters | Return Type | Description |
|---------|-----------|-------------|-------------|
| `get_resource_usage` | _(none)_ | `ResourceUsage` | Returns current system CPU percentage and used memory in MB as `{cpu_percent: f32, memory_mb: u64}`. Uses a persistent `sysinfo::System` instance for accurate delta-based CPU measurement. First call returns approximately 0% CPU. |
