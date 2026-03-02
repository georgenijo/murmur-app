# Tauri Commands Reference

This document lists all 30 registered Tauri commands exposed from the Rust backend to the frontend via `invoke()`. Commands are grouped by their source module under `app/src-tauri/src/`.

For event-based communication (Rust to frontend), see [events.md](events.md). For frontend hooks that call these commands, see [hooks.md](hooks.md).

---

## Recording (`commands/recording.rs`)

| Command | Parameters | Return Type | Description |
|---------|-----------|-------------|-------------|
| `init_dictation` | _(none)_ | `Result<JSON, String>` | Returns a static `{"type":"initialized","state":"idle"}` response. No-op initialization marker. |
| `process_audio` | `audio_data: String` | `Result<JSON, String>` | Accepts base64-encoded WAV audio, decodes it, runs the full VAD + transcription + text injection pipeline, and returns `{"type":"transcription","text":"..."}`. |
| `get_status` | _(none)_ | `Result<JSON, String>` | Returns current dictation status, model name, and language as `{"type":"status","state":"...","model":"...","language":"..."}`. |
| `configure_dictation` | `options: JSON` | `Result<JSON, String>` | Updates dictation settings. Accepts optional fields: `model` (string), `language` (string), `autoPaste` (bool), `autoPasteDelayMs` (u64, clamped 10-500), `vadSensitivity` (u64, clamped 0-100). Swaps transcription backend if model type changes between Whisper and Moonshine. |
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

## Models (`commands/models.rs`)

| Command | Parameters | Return Type | Description |
|---------|-----------|-------------|-------------|
| `check_model_exists` | _(none)_ | `bool` | Returns `true` if any transcription model exists for either backend (Whisper or Moonshine). Used to determine whether the model download screen should be shown on first launch. |
| `check_specific_model_exists` | `model_name: String` | `bool` | Returns `true` if the specified model file or directory exists on disk. Includes path traversal protection (rejects `..`, `/`, `\` in model names). |
| `download_model` | `model_name: String` | `Result<(), String>` | Downloads a transcription model with streaming progress events. Allowed models: `large-v3-turbo`, `small.en`, `base.en`, `tiny.en`, `medium.en`, `moonshine-tiny`, `moonshine-base`. Also co-downloads the Silero VAD model if missing. Whisper models are downloaded as single `.bin` files; Moonshine models as `.tar.bz2` archives that are extracted on a blocking thread. |

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
