# Agent 3 — Rust Commands Findings

## User-Facing Features

### Recording & Transcription
- **Native audio recording** via `start_native_recording` / `stop_native_recording` commands. Audio is captured through `cpal` at the device's native sample rate, then resampled to 16 kHz mono for Whisper.
- **Device selection**: Users can specify an audio input device by name (`device_name` parameter on `start_native_recording`). Falls back to system default when `None`.
- **Cancel recording**: `cancel_native_recording` silently discards a speculative recording without transcribing. Used by "Both" mode to discard the hold-down recording started during a short tap.
- **Minimum recording length enforcement**: Recordings shorter than 0.3 seconds (4,800 samples at 16 kHz) are silently discarded as phantom triggers. No transcription is attempted.
- **Voice Activity Detection (VAD)**: Silero VAD v5.1.2 filters out silence before transcription, preventing Whisper hallucination loops. VAD sensitivity is user-configurable (0-100 scale, converted to a threshold of `1.0 - sensitivity/100`). When VAD detects no speech, transcription is skipped entirely and an empty string is returned.
- **Base64 audio path**: `process_audio` accepts base64-encoded WAV data from the frontend, decodes it, parses WAV to samples, and runs the same transcription pipeline. This is an alternative to native recording.
- **Text injection**: After transcription, text is always written to the system clipboard. If `auto_paste` is enabled, an osascript-based Cmd+V paste is triggered after a configurable delay (10-500ms, default 50ms). If auto-paste fails or times out (2 second timeout), an `auto-paste-failed` event is emitted with a hint message telling the user to paste manually.
- **Transcription result broadcast**: Non-empty transcriptions emit a `transcription-complete` event with the text and recording duration, allowing all windows (main and overlay) to display results.

### Model Management
- **Model existence check**: `check_model_exists` checks both the currently configured backend AND the other backend type, so the download screen does not appear if any model is installed (whisper or moonshine).
- **Specific model check**: `check_specific_model_exists` verifies a named model exists on disk. Includes path traversal protection (rejects `..`, `/`, `\` in model names). Supports both whisper (ggml-{name}.bin) and moonshine (directory-based) models.
- **Model download**: `download_model` downloads models with streaming progress. Allowed models are hardcoded: `large-v3-turbo`, `small.en`, `base.en`, `tiny.en`, `medium.en`, `moonshine-tiny`, `moonshine-base`.
  - Whisper models: Downloaded as single `.bin` files from HuggingFace (`ggerganov/whisper.cpp`).
  - Moonshine models: Downloaded as `.tar.bz2` archives, extracted via bzip2+tar on a blocking thread. Partial extractions are cleaned up on failure.
  - Downloads use a temp file (`.tmp` suffix) that is atomically renamed on success, or deleted on failure.
  - **VAD model co-download**: When downloading any transcription model, the VAD model (~1.8MB) is automatically co-downloaded if not already present. VAD download failure is non-fatal.
- **Lazy VAD download**: `ensure_vad_model` is a fallback for users who upgrade from a pre-VAD version. If the VAD model is missing at transcription time, a background download is kicked off for next time. During explicit download, it emits `recording-status-changed` with value `"downloading-vad"`.

### Configuration
- **Dictation configuration**: `configure_dictation` accepts a JSON object with optional fields: `model` (string), `language` (string), `autoPaste` (bool), `autoPasteDelayMs` (u64, clamped 10-500), `vadSensitivity` (u64, clamped 0-100).
- **Backend switching**: When the model name changes, the system detects whether a backend swap is needed (whisper vs moonshine). If so, it replaces the `Box<dyn TranscriptionBackend>`. If the same backend type, it calls `reset()` to force model reload.
- **Default configuration**: model=`base.en`, language=`en`, auto_paste=`false`, auto_paste_delay_ms=`50`, vad_sensitivity=`50`.

### Keyboard Listener
- **Start listener**: `start_keyboard_listener` accepts a hotkey string and a mode (`double_tap`, `hold_down`, or `both`). Validates the mode and requires accessibility permission before starting.
- **Stop listener**: `stop_keyboard_listener` stops the global rdev keyboard listener.
- **Update key**: `update_keyboard_key` changes the target hotkey at runtime. If the key is changed while held down, it emits `hold-down-stop` to force-stop the current recording, preventing stuck states.
- **Recording state sync**: `set_keyboard_recording` synchronizes the keyboard module's recording state flag.

### Permissions
- **Open System Preferences**: `open_system_preferences` opens the macOS System Settings to the Microphone privacy pane.
- **Check accessibility**: `check_accessibility_permission` returns a boolean indicating whether accessibility is granted.
- **Request accessibility**: `request_accessibility_permission` triggers the macOS system accessibility prompt and then opens System Settings to the Accessibility pane.
- **Request microphone**: `request_microphone_permission` opens System Settings to the Microphone privacy pane.
- **List audio devices**: `list_audio_devices` returns a list of input audio device names.

### Overlay Window
- **Notch-aware overlay**: The overlay window is positioned to overlap the MacBook notch. It detects notch dimensions via NSScreen APIs (safe area insets, auxiliary top areas) and sizes the window to match.
- **Show overlay**: `show_overlay` positions, sizes, and shows the overlay window. Re-enables mouse events (which focusable:false disables).
- **Hide overlay**: `hide_overlay` hides the overlay window. Gracefully handles missing window.
- **Get notch info**: `get_notch_info` returns cached notch dimensions (width and height) to the frontend for precise content positioning.
- **Window level**: The overlay is raised to NSMainMenuWindowLevel + 1 (level 25), above the menu bar, using NSWindow APIs.
- **Click-through prevention**: Uses private API `_setPreventsActivation:` to prevent clicking the overlay from activating the app (which would unhide the main window). Guarded by `respondsToSelector:` for forward compatibility.
- **Screen change observer**: Registers an NSNotification observer for `NSApplicationDidChangeScreenParametersNotification` to re-detect notch info and reposition the overlay when monitors are plugged/unplugged or the lid opens/closes.

### Tray Icon
- **Static tray icon**: A 66x66 RGBA icon (3x resolution for 22pt Retina menu bar) showing 5 vertical capsule bars in a waveform/equalizer style, rendered as white with anti-aliased edges.
- **No-op update**: `update_tray_icon` is a registered no-op command. The tray icon is always static white. Command kept to avoid breaking registered handler.
- **Tray menu**: "Show Murmur" (shows and focuses main window) and "Quit Murmur" (exits app). Left-click on tray icon also shows the main window.

### Logging & Diagnostics
- **Log viewer**: `open_log_viewer` shows and focuses the `log-viewer` window (must be pre-configured in tauri.conf.json).
- **Get logs**: `get_log_contents` reads the last N lines from the pretty-printed log file (`app.log` or `app.dev.log`).
- **Clear logs**: `clear_logs` removes all known log files (including rotated variants, JSONL event files, frontend logs, transcription logs) and clears the in-memory event ring buffer.
- **Frontend logging**: `log_frontend` allows the frontend to log messages through the Rust tracing system with INFO/WARN/ERROR levels.
- **Event history**: `get_event_history` returns the entire in-memory ring buffer (up to 500 structured events). `clear_event_history` empties it.
- **Resource monitor**: `get_resource_usage` returns current system CPU percentage and used memory in MB. Uses a persistent `sysinfo::System` instance for accurate delta-based CPU measurements (first call yields ~0%).

### Window Behavior
- **Close-to-hide**: Both the `main` and `log-viewer` windows intercept close requests and hide instead of being destroyed, preserving state.
- **Reopen behavior**: On macOS dock icon click, the main window only shows if there are truly no visible windows. This prevents clicking the overlay from unhiding the main window.

## Internal Systems

### Transcription Pipeline (`run_transcription_pipeline`)
Full pipeline executed on `stop_native_recording` and `process_audio`:
1. **IdleGuard creation** — RAII guard that resets `DictationStatus` to `Idle` and `keyboard::set_processing(false)` on drop, ensuring cleanup on any error path.
2. **State read** — Model name, language, auto-paste settings, paste delay, VAD sensitivity read in a single lock acquisition.
3. **Pre-VAD diagnostics** — Computes RMS and peak amplitude of raw audio. Logs device name for mic diagnosis.
4. **VAD phase** — If VAD model exists, runs Silero VAD on a blocking thread to filter out non-speech. Three outcomes: NoSpeech (skip transcription), Speech (trimmed samples), Error (fallback to unfiltered). If VAD model missing, spawns background download for next time.
5. **Transcription phase** — Locks the backend mutex, calls `load_model()` (lazy, only loads on first run or after reset), then `transcribe()`. Model loading and inference happen under the same lock.
6. **Text injection phase** — Dispatched to the main thread via `run_on_main_thread`. Clipboard write + optional osascript paste. 2-second timeout; on failure emits `auto-paste-failed`.
7. **Structured logging** — Logs vad_ms, inference_ms, paste_ms, total_ms, audio_secs, word_count, char_count, model name, backend name.

### State Machine (DictationStatus)
Three states: `Idle`, `Recording`, `Processing`. Serialized as lowercase strings.
- `start_native_recording`: Idle -> Recording. If already Recording or Processing, returns early with descriptive JSON.
- `stop_native_recording`: Recording -> Processing. If Processing, returns early. If Idle, returns "not_recording".
- `cancel_native_recording`: Recording -> Idle. If not Recording, returns Ok(()).
- Pipeline completion: Processing -> Idle (via IdleGuard drop).

### IdleGuard (RAII)
- Created at the start of each pipeline invocation.
- On drop (unless disarmed), sets `DictationStatus::Idle` and calls `keyboard::set_processing(false)`.
- Both `stop_native_recording` and `process_audio` create an outer guard for the decode/parse phase, disarm it, then let `run_transcription_pipeline` create its own guard.

### MutexExt Trait
- Provides `lock_or_recover()` which recovers from poisoned mutexes by calling `into_inner()` on the poison error, logging a warning. Applied to all mutex locks in the codebase.

### Telemetry / Structured Event System
- **TauriEmitterLayer**: A custom `tracing_subscriber::Layer` that intercepts all tracing events and:
  1. Converts them to `AppEvent` structs (timestamp, stream/target, level, summary/message, data/fields).
  2. Pushes to an in-memory ring buffer (capacity 500, FIFO eviction).
  3. Writes as JSONL to a persistent file (`events.jsonl` or `events.dev.jsonl`).
  4. Emits to all frontend windows via `app-event`.
- **Privacy stripping**: In release builds, all string fields from `pipeline` target events are stripped from the data object.
- **JSONL rotation**: File is rotated (renamed to `.jsonl.1`) when it exceeds 5 MB.
- **Buffer seeding**: On startup, the ring buffer is pre-populated with up to 500 events from the existing JSONL file.
- **Log file**: A separate pretty-printed log file (`app.log`) is maintained via `tracing_appender`.

### Stream Download System
- `stream_download()`: Generic async streaming download with progress events. Uses `reqwest` with 30s connect timeout and 15-minute overall timeout. Writes chunks to a temp file, emits `download-progress` events with `{received, total}` JSON. Cleans up temp file on error.

### Backend Architecture
- `TranscriptionBackend` is a trait with implementations `WhisperBackend` and `MoonshineBackend`.
- Stored as `Box<dyn TranscriptionBackend>` behind a Mutex.
- `model_exists()`, `models_dir()`, `load_model()`, `transcribe()`, `reset()`, `name()` are the trait methods used by commands.
- Backend type is swapped when model changes cross the whisper/moonshine boundary.

### Notch Detection
- Uses `objc2_app_kit::NSScreen` APIs on macOS: `safeAreaInsets()` for menu bar height, `auxiliaryTopLeftArea()` and `auxiliaryTopRightArea()` for non-notch menu bar areas. Notch width = screen width - left - right.
- Cached in `State.notch_info` (Mutex-wrapped Option).
- Fallback: 200px wide, 37px tall when no notch detected.
- Overlay expanded by 120px (60px per side) beyond the notch width.

## Commands / Hooks / Events

### Tauri Commands (registered in invoke_handler)

**Recording module (`commands/recording.rs`)**:
- `init_dictation` — Returns a JSON "initialized" / "idle" response. No-op initialization marker.
- `process_audio` — Accepts base64-encoded WAV audio, decodes, and runs the full transcription pipeline.
- `get_status` — Returns current dictation status, model name, and language as JSON.
- `configure_dictation` — Updates model, language, auto-paste, paste delay, and VAD sensitivity settings.
- `start_native_recording` — Begins native audio capture via cpal; transitions to Recording state.
- `stop_native_recording` — Stops audio capture, runs VAD + transcription + text injection pipeline.
- `cancel_native_recording` — Cancels an in-progress recording without transcribing (discards audio).

**Permissions module (`commands/permissions.rs`)**:
- `open_system_preferences` — Opens macOS System Settings to the Microphone privacy pane.
- `check_accessibility_permission` — Returns boolean for accessibility permission status.
- `request_accessibility_permission` — Triggers accessibility system prompt and opens Settings.
- `request_microphone_permission` — Opens macOS System Settings to the Microphone pane.
- `list_audio_devices` — Returns Vec<String> of available audio input device names.

**Keyboard module (`commands/keyboard.rs`)**:
- `start_keyboard_listener` — Starts the rdev global keyboard listener with a specified hotkey and mode.
- `stop_keyboard_listener` — Stops the rdev global keyboard listener.
- `update_keyboard_key` — Changes the target hotkey at runtime; emits stop if key changed while held.
- `set_keyboard_recording` — Synchronizes the keyboard module's internal recording state flag.

**Logging module (`commands/logging.rs`)**:
- `get_log_contents` — Returns the last N lines of the pretty-printed log file.
- `clear_logs` — Deletes all log files and clears the event ring buffer.
- `log_frontend` — Routes a frontend log message through the Rust tracing system.
- `open_log_viewer` — Shows and focuses the log-viewer window.

**Models module (`commands/models.rs`)**:
- `check_model_exists` — Checks if any transcription model exists (either backend).
- `check_specific_model_exists` — Checks if a named model exists on disk.
- `download_model` — Downloads a transcription model (whisper or moonshine) with progress events.

**Tray module (`commands/tray.rs`)**:
- `update_tray_icon` — No-op; tray icon is always static. Kept for API compatibility.

**Overlay module (`commands/overlay.rs`)**:
- `show_overlay` — Positions and shows the notch overlay window.
- `hide_overlay` — Hides the notch overlay window.
- `get_notch_info` — Returns cached notch dimensions (width, height) or None.

**Telemetry module (`telemetry.rs`)**:
- `get_event_history` — Returns the full in-memory structured event ring buffer.
- `clear_event_history` — Clears the in-memory event ring buffer.

**Resource monitor (`resource_monitor.rs`)**:
- `get_resource_usage` — Returns current CPU percentage and memory usage in MB.

### Emitted Events (Rust -> Frontend)

- `recording-status-changed` — Payload: string (`"recording"`, `"processing"`, `"idle"`, `"downloading-vad"`). Emitted at every dictation state transition.
- `download-progress` — Payload: `{"received": u64, "total": u64}`. Emitted during model/VAD downloads with byte counts.
- `hold-down-stop` — Payload: `()`. Emitted when the hotkey is changed while held down, forcing recording stop.
- `auto-paste-failed` — Payload: string hint ("Text is in your clipboard -- press Cmd+V to paste manually."). Emitted when text injection fails or times out.
- `transcription-complete` — Payload: `{"text": string, "duration": usize}`. Emitted after successful non-empty transcription for cross-window result broadcast.
- `notch-info-changed` — Payload: `Option<NotchInfo>` (`{notch_width, notch_height}` or null). Emitted when screen parameters change (monitor plug/unplug, lid open/close).
- `app-event` — Payload: `AppEvent` struct (`{timestamp, stream, level, summary, data}`). Emitted for every tracing event by TauriEmitterLayer.

### Internal (non-command) Functions

- `make_tray_icon_data()` — Generates 66x66 RGBA pixel data for the static tray icon.
- `detect_notch_info()` — Detects MacBook notch width and menu bar height via NSScreen APIs.
- `register_screen_change_observer()` — Registers NSNotification observer for display changes.
- `raise_window_above_menubar()` — Sets overlay NSWindow level to 25 (above menu bar).
- `position_overlay_default()` — Computes and applies overlay position/size based on notch info and monitor geometry.
- `run_transcription_pipeline()` — Core async pipeline: VAD -> transcribe -> inject text.
- `ensure_vad_model()` — Downloads VAD model if missing (fallback for upgrades).
- `stream_download()` — Generic async streaming file download with progress events.
- `download_whisper_model()` — Downloads a single whisper ggml .bin file.
- `download_moonshine_model()` — Downloads and extracts a moonshine tar.bz2 archive.
- `open_system_preference_pane()` — Opens a specific macOS System Settings pane via x-apple.systempreferences URL.

## Gaps / Unclear

1. **`update_tray_icon` is a dead command**: The function is a no-op. The `_icon_state` parameter is accepted but ignored. The comment says "Kept so the registered command doesn't break" — this suggests the frontend still calls it somewhere but it does nothing. Could be removed along with the frontend call.

2. **`init_dictation` does nothing**: Returns a static JSON response with no side effects. It does not actually initialize anything. Unclear why it exists — possibly a leftover from an earlier WebSocket-based protocol.

3. **`process_audio` duplicates pipeline logic**: Both `stop_native_recording` and `process_audio` have nearly identical post-pipeline logging (vad_ms, inference_ms, etc.) but `process_audio` does not emit `transcription-complete`. This means transcriptions via base64 audio do not broadcast to all windows.

4. **Hardcoded allowed models list**: The `ALLOWED_MODELS` array in `download_model` is a static list. Adding new models requires a code change and recompile. No dynamic model discovery.

5. **Model directory name "local-dictation"**: The app stores data under `~/Library/Application Support/local-dictation/` rather than `murmur`. This appears to be a legacy name from before the app was renamed.

6. **Non-cross-platform**: Nearly all permission, overlay, and tray commands are macOS-only. The `#[cfg(not(target_os = "macos"))]` branches either return `Ok(())` or `Err("not supported")` with no Windows/Linux implementation. The overlay notch detection returns `None` on non-macOS.

7. **Integer truncation in duration**: In `stop_native_recording`, recording duration is calculated as `samples.len() / 16_000` (integer division), losing sub-second precision. The `transcription-complete` event therefore reports duration in whole seconds only.

8. **VAD model path hardcoded to "local-dictation"**: `vad_model_path()` uses `dirs::data_dir().join("local-dictation").join("models")`, while the transcription models use the backend's `models_dir()` method. If these ever diverge, the VAD model could end up in a different directory.

9. **Single-threaded VAD context**: `WhisperVadContext` is created and destroyed on every transcription (inside `filter_speech`). Unlike the transcription model (which is cached), the VAD context is not reused. This may add unnecessary initialization overhead.

10. **`check_model_exists` cross-backend check is asymmetric**: When the active backend is whisper, it also checks moonshine. When moonshine, it also checks whisper. But it always instantiates a fresh backend just to check, rather than using a shared check.

11. **Auto-paste delay clamping mismatch**: `configure_dictation` clamps `autoPasteDelayMs` to 10-500, but the default is 50. The frontend might expose a different range than what the backend enforces.

12. **`clear_logs` also clears event history**: The command calls both `clear_all_logs()` (file deletion) and `clear_event_history()` (ring buffer clear). This coupling is implicit — the function name doesn't suggest it also clears in-memory events.

13. **Privacy stripping is coarse**: In release builds, ALL string fields in pipeline events are stripped. This means structured fields like model name, backend name, etc. are removed from the data — only numeric fields survive. The summary (message) is NOT stripped, which may contain sensitive text in some logging paths.

14. **No error event for VAD download during pipeline**: When VAD model is missing during transcription, `ensure_vad_model` emits `recording-status-changed` as `"downloading-vad"` then `"processing"`, but if the download fails, the error is only logged — the frontend gets no explicit failure notification for the VAD download attempt.

## Notes

1. **Mutex poisoning is universally handled**: Every mutex lock in the commands layer uses `lock_or_recover()`, meaning a panic in one thread will not permanently lock out others. This is a deliberate resilience decision.

2. **RAII pattern for status management**: The `IdleGuard` struct ensures dictation status is always reset to Idle, even on panics or early returns. This is a well-designed safety net. Both the outer command and inner pipeline have their own guards with a disarm handoff pattern.

3. **Thread safety for text injection**: Text injection (clipboard + paste) is dispatched to the main thread via `run_on_main_thread` with a oneshot channel and 2-second timeout. This is necessary because `arboard`/clipboard access and osascript require main thread on macOS.

4. **Structured telemetry is comprehensive**: Every tracing event in the entire application is captured, stored in a ring buffer, persisted to JSONL, and emitted to the frontend. This provides full observability through the log viewer.

5. **Download atomicity**: All downloads use a temp-file-then-rename pattern, ensuring partial downloads never appear as valid models. Moonshine archives also clean up partially extracted directories on failure.

6. **Overlay uses private macOS APIs**: `_setPreventsActivation:` is used but guarded by `respondsToSelector:`. The code is aware this may break in future macOS versions and handles the failure gracefully.

7. **Three tauri plugins are registered**: autostart (LaunchAgent), updater, and notification. The updater and notification plugins are initialized but their commands are not in the commands/ directory — they likely provide their own built-in commands.

8. **The `app-event` emission creates a real-time event stream**: Every tracing log in the entire Rust backend is broadcast to all frontend windows via the `app-event` event. This is the backbone of the log viewer feature.

9. **Keyboard module has a bidirectional state sync**: `set_keyboard_recording` lets the frontend tell the keyboard module about recording state, while keyboard events flow the other direction. `set_processing` is called by the pipeline to prevent hotkey triggers during transcription.

10. **Resource monitor uses a static System instance**: The `sysinfo::System` is stored in a `static Mutex<Option<System>>` and initialized on first call. This ensures accurate CPU delta measurements on subsequent polls, though the first reading is always ~0%.
