# Agent 4 — Rust Core Findings

## User-Facing Features

### Recording Modes
- **Double-tap mode**: User double-taps a modifier key (Left Shift, Left Alt, or Right Ctrl) to start recording; single-taps the same key to stop. The double-tap-to-start requires two quick presses within 400ms with each hold under 200ms. When already recording, a single quick tap (under 200ms) fires a stop event.
- **Hold-down mode**: User holds a modifier key to record; releasing the key stops recording and triggers transcription. In hold-down-only mode, there is no minimum hold duration: a key press immediately emits `hold-down-start` and a key release immediately emits `hold-down-stop`, regardless of how short the press was. The frontend then receives both events and goes through the full start/stop/transcribe cycle even for very brief taps. The minimum recording duration check (0.3s / 4800 samples) in `stop_native_recording` handles discarding phantom triggers on the Rust pipeline side.
- **Both mode**: Combines double-tap and hold-down simultaneously. Uses deferred hold promotion: on key press, a background timer thread waits 200ms (MAX_HOLD_DURATION_MS). If the key is still held after that, the press is "promoted" to a real hold-down-start event. If released before 200ms, the event is handled as part of double-tap detection. This prevents false recording starts during double-tap sequences. The hold-down detector is suppressed during the second phase of a double-tap sequence (WaitingSecondDown or WaitingSecondUp states). **Important: `hold-down-cancel` is NOT emitted from Rust**. The event name `hold-down-cancel` does not appear anywhere in `keyboard.rs` or any other Rust source file. In Both mode, when a short tap occurs (key released before the 200ms timer promotes it), the hold was never promoted so `HOLD_PROMOTED` is false. If the double-tap detector also didn't fire, the code falls through to the comment "short single tap, no recording was started, nothing to do" and emits nothing at all. The frontend `useCombinedToggle.ts` registers a listener for `hold-down-cancel` (line 64) which would call `cancel_native_recording`, but this event is never actually emitted from the backend -- the listener is dead code.
- Supported hotkeys: `shift_l` (Left Shift), `alt_l` (Left Alt/Option), `ctrl_r` (Right Control). These are the only three mapped by `hotkey_to_rdev_key()`.
- Both detectors reject modifier+letter combos (e.g., Shift+A) to avoid triggering during normal typing.

### Dictation Status Lifecycle
- Three states: **Idle**, **Recording**, **Processing**. The DictationStatus enum is serialized as lowercase strings ("idle", "recording", "processing").
- Status transitions: Idle -> Recording (on start), Recording -> Processing (on stop), Processing -> Idle (after transcription completes or fails).
- An `IdleGuard` RAII pattern ensures status always resets to Idle on any error or early return in the pipeline.

### Transcription Pipeline
- **VAD pre-filtering**: Before transcription, audio is passed through Silero VAD (Voice Activity Detection) v5.1.2 to filter out silence and prevent Whisper hallucination loops. VAD threshold is computed as `1.0 - (vad_sensitivity / 100.0)` from user-configurable sensitivity (0-100, default 50). If VAD detects no speech, transcription is skipped entirely and an empty string is returned.
- **Two transcription backends**: Whisper (via whisper-rs with Metal GPU acceleration) and Moonshine (via sherpa-rs ONNX runtime on CPU). Backend is selected based on model name prefix: names starting with "moonshine-" use MoonshineBackend, everything else uses WhisperBackend.
- **Lazy model loading**: Models are loaded on first transcription, not at startup. The loaded model is cached (WhisperState persisted across transcriptions). If model name changes, the backend resets and reloads.
- **Whisper backend**: Uses greedy sampling (best_of=1), single segment mode, timestamps/progress/special tokens suppressed, blank suppression enabled. Searches for model files (`ggml-{name}.bin`) in multiple directories: `$WHISPER_MODEL_DIR`, `~/Library/Application Support/local-dictation/models`, `~/Library/Application Support/pywhispercpp/models`, `~/.cache/whisper.cpp`, `~/.cache/whisper`, `~/.whisper/models`.
- **WhisperState caching mechanism (v0.7.8)**: The `WhisperBackend` struct has three fields: `context: Option<WhisperContext>`, `state: Option<WhisperState>`, and `loaded_model_name: Option<String>`. In `load_model()`, the method first checks if `loaded_model_name` matches the requested model -- if so, it returns `Ok(())` immediately (no-op, reuses existing context and state). If the model name differs, it calls `reset()` which drops the existing state and context via `take()`. A new `WhisperContext` is created from the model file, and then `ctx.create_state()` is called exactly once to produce a `WhisperState`. Both are stored in the struct's `Option` fields. In `transcribe()`, the method calls `self.state.as_mut()` to get a mutable reference to the already-stored `WhisperState` and calls `state.full(params, samples)` on it. The state is never recreated between transcriptions -- it persists for the lifetime of the backend (or until `reset()` is called due to a model change). This is the v0.7.8 optimization: previously, `create_state()` was called per-transcription, causing expensive alloc/free cycles. Now the state is allocated once and reused, with a log message "whisper: reusing cached state for transcription" on each invocation.
- **Moonshine backend**: Uses ONNX int8 quantized models from sherpa-onnx. Provider is hardcoded to "cpu". Required model files: `preprocess.onnx`, `encode.int8.onnx`, `uncached_decode.int8.onnx`, `cached_decode.int8.onnx`, `tokens.txt`. The `_language` parameter in `transcribe()` is ignored (Moonshine models are English-only).
- **Minimum recording duration**: Recordings shorter than 0.3 seconds (4800 samples at 16kHz) are silently discarded as phantom triggers.
- **Audio level monitoring**: During recording, RMS audio level is computed per buffer chunk and emitted as "audio-level" events at ~60fps (throttled to 16ms minimum gap).

### Text Injection
- **Clipboard-first**: Transcribed text is always copied to clipboard via `arboard`.
- **Auto-paste** (optional): If enabled, uses osascript to simulate Cmd+V keystroke via System Events. Requires macOS Accessibility permission. Configurable delay before paste (10-500ms, default 50ms) to allow window focus settling.
- Auto-paste retries once on failure with a 100ms backoff.
- If auto-paste fails, an "auto-paste-failed" event is emitted with a paste hint message ("press Cmd+V to paste manually").
- Empty/whitespace-only text is silently skipped (not copied to clipboard).
- Accessibility check before paste: if not granted, text stays in clipboard with a warning log (no error to user).

### Model Management
- **Allowed models for download**: `large-v3-turbo`, `small.en`, `base.en`, `tiny.en`, `medium.en`, `moonshine-tiny`, `moonshine-base`. Any other model name is rejected.
- **Default model**: `base.en` (set in DictationState::default()).
- **Download progress**: Emits "download-progress" events with `{received, total}` during streaming downloads.
- **Whisper models**: Downloaded from `https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-{name}.bin`.
- **Moonshine models**: Downloaded from `https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/` as tar.bz2 archives, extracted via bzip2+tar.
- **VAD model co-download**: The Silero VAD model (`ggml-silero-v5.1.2.bin`, ~1.8MB) is co-downloaded alongside any transcription model. If missing at transcription time, a background download is kicked off for next time.
- **Path traversal protection**: `check_specific_model_exists` rejects model names containing `..`, `/`, or `\`.

### Overlay Window (Notch Bar)
- An always-on-top overlay window positioned in the macOS notch area. Window level is set to 25 (NSMainMenuWindowLevel + 1) to appear above the menu bar.
- Width = notch_width + 120px (60px expansion on each side). Height = menu bar height. Fallback dimensions: 200px wide, 37px tall if no notch detected.
- Positioned horizontally centered at the top of the screen (y=0).
- Uses `_setPreventsActivation:` private API to prevent overlay clicks from activating the app (which would unhide the main window).
- Mouse events are explicitly re-enabled (`setIgnoreCursorEvents(false)`) because `focusable:false` disables them on macOS.
- Notch dimensions are detected via NSScreen APIs (safeAreaInsets, auxiliaryTopLeftArea, auxiliaryTopRightArea) and cached in State.
- Screen change observer: Listens for `NSApplicationDidChangeScreenParametersNotification` to re-detect notch and reposition overlay on monitor plug/unplug or lid open/close. Emits "notch-info-changed" event to frontend.

### Tray Icon
- System tray icon with a 66x66 RGBA pixel waveform/equalizer pattern (5 vertical capsule bars at varying heights). Rendered at 3x resolution for Retina displays (22pt icon).
- Static white icon (no dynamic state changes). The `update_tray_icon` command is a no-op.
- Tray menu: "Show Murmur" and "Quit Murmur" items with separator.
- Left-click on tray icon shows and focuses the main window.
- Menu events: "show" shows/focuses main window, "quit" exits app.

### Window Behavior
- **Main window** and **log-viewer window**: Close requests are intercepted; windows are hidden instead of destroyed (persistent windows).
- **Reopen behavior**: macOS app reactivation (e.g., dock click) only shows the main window when there are truly no visible windows. This prevents the overlay click from unhiding the main window.

### Permissions
- **Accessibility permission**: Checked via `AXIsProcessTrusted()` FFI. Requested via `AXIsProcessTrustedWithOptions()` with prompt option, then opens System Settings Accessibility pane.
- **Microphone permission**: Opens System Settings Microphone privacy pane.
- Accessibility is required for both keyboard listener (start_keyboard_listener validates it) and auto-paste functionality.

### Logging and Telemetry
- **No separate `logging.rs` file**: There is no top-level `logging.rs` module in `app/src-tauri/src/`. The logging infrastructure lives entirely in `telemetry.rs` (declared as `pub mod telemetry` in `lib.rs`). The file `commands/logging.rs` exists but is just a thin command layer that delegates to `telemetry.rs` functions (`read_pretty_log_tail`, `clear_all_logs`, `clear_event_history`). The CLAUDE.md file map's reference to `logging.rs` is outdated -- it was likely renamed/refactored to `telemetry.rs` when the structured event system was added.
- **Structured event system**: All logging goes through `tracing` with two layers:
  1. Pretty-printed text file (`app.log` / `app.dev.log`) via `tracing_appender`
  2. Structured JSONL file (`events.jsonl` / `events.dev.jsonl`) + Tauri event emission to frontend
- **Ring buffer**: Last 500 events kept in memory (VecDeque), seeded from existing JSONL on startup.
- **JSONL rotation**: Files rotated when exceeding 5MB (renamed to `.jsonl.1`).
- **Privacy**: In release builds, all string fields are stripped from "pipeline" stream events.
- **Log viewer**: Dedicated `log-viewer` window that can be shown via command.
- **Frontend logging**: `log_frontend` command routes frontend logs through Rust tracing as INFO/WARN/ERROR with source="frontend".
- **Clear logs**: Removes 18 known log file variants including dated rolling files.
- Tracing targets (streams): "system", "pipeline", "audio", "keyboard".

### Resource Monitor
- Reports CPU usage percentage and used memory (in MB) via `sysinfo` crate. Uses a persistent `System` instance in a static Mutex. First call returns ~0% CPU (baseline measurement behavior).

### Auto-updater
- Plugin `tauri_plugin_updater` is initialized for in-app updates.

### Autostart
- Plugin `tauri_plugin_autostart` configured with `MacosLauncher::LaunchAgent`.

## Internal Systems

### Audio Capture Pipeline (audio.rs)
- Uses `cpal` for platform audio input.
- **Device selection**: Optionally accepts a device name; falls back to system default if not found or not specified.
- **Multi-channel to mono**: Interleaved multi-channel samples are averaged to mono in the cpal callback.
- **Sample format support**: F32 and I16 only. Other formats return an error.
- **Resampling**: Linear interpolation resampler converts device sample rate to 16kHz (WHISPER_SAMPLE_RATE). Only runs if device rate differs from 16kHz.
- **Thread architecture**: Audio capture runs on a dedicated thread (`run_audio_capture`). Communication via `mpsc::channel` with `AudioCommand::Stop`. Thread loops with 100ms recv timeouts.
- **Buffer management**: Fresh `Arc<Mutex<Vec<f32>>>` created per recording. Taken (moved out) on stop, leaving None for the next recording. No stale data possible.
- **Recording state**: Global `OnceLock<Mutex<RecordingState>>` holds the command sender, thread handle, shared buffer, sample rate, start timestamp, and device name.
- **Privacy**: Device name is redacted to "<redacted>" in release build logs.
- **Initialization handshake**: `start_recording` waits up to 5 seconds for the audio thread to signal ready (sends back device sample rate and name).

### Keyboard Event System (keyboard.rs)
- **Single rdev listener thread**: Spawned once via `compare_exchange` on `LISTENER_THREAD_SPAWNED`. The thread runs forever; `LISTENER_ACTIVE` bool gates whether events are processed.
- **rdev main thread safety**: `set_is_main_thread(false)` called before `listen()` to force rdev to dispatch TIS/TSM API calls to the main queue via dispatch_sync.
- **Heartbeat monitor**: Separate thread logs trace-level heartbeat every 60 seconds while listener is active.
- **Processing state management**: `set_processing(true/false)` is called when entering/leaving the transcription pipeline. During processing:
  - Both-mode callback ignores all key events
  - Pending hold-promotion timers are invalidated via HOLD_PRESS_COUNTER
  - Both detectors are reset
  - On exit from processing, cooldowns are applied to prevent accidental re-triggers
- **Global statics**: `LISTENER_ACTIVE` (AtomicBool), `LISTENER_THREAD_SPAWNED` (AtomicBool), `ACTIVE_MODE` (Mutex<DetectorMode>), `DOUBLE_TAP_DETECTOR` (Mutex<Option<DoubleTapDetector>>), `HOLD_DOWN_DETECTOR` (Mutex<Option<HoldDownDetector>>), `HOLD_PRESS_COUNTER` (AtomicU64), `HOLD_PROMOTED` (AtomicBool), `IS_PROCESSING` (AtomicBool).

### Both-Mode Arbitration (keyboard.rs)
- On key press: the hold-down detector fires Start, but instead of emitting immediately, a background timer thread is spawned. The timer sleeps for MAX_HOLD_DURATION_MS (200ms) then checks if (a) the press ID matches HOLD_PRESS_COUNTER and (b) the hold detector is still in Held state. If both conditions hold, HOLD_PROMOTED is set to true and "hold-down-start" is emitted.
- On key release: if HOLD_PROMOTED is true, "hold-down-stop" is emitted (real hold ended). If not promoted but double-tap fired, "double-tap-toggle" is emitted. Otherwise (short single tap with no double-tap), nothing is emitted.
- The hold-down detector is suppressed (not fed events) when the double-tap detector is in its second phase (WaitingSecondDown or WaitingSecondUp) and the window hasn't expired. This prevents hold-down-start from firing during the second tap of a double-tap.

### Transcription Backend Trait (transcriber/mod.rs)
- `TranscriptionBackend` trait: `Send + Sync`, methods: `name()`, `load_model(model_name)`, `transcribe(samples, language)`, `model_exists()`, `models_dir()`, `reset()`.
- `AppState.backend` holds a `Mutex<Box<dyn TranscriptionBackend>>`.
- Backend swapping: When model changes between whisper and moonshine types, the entire backend Box is replaced. When model changes within the same backend type, just `reset()` is called to force a reload.

### WAV Parsing (transcriber/mod.rs)
- `parse_wav_to_samples`: Parses base64-decoded WAV bytes. Validates: 16kHz sample rate, mono, 16-bit integer PCM. Converts i16 samples to f32 by dividing by i16::MAX.

### Voice Activity Detection (vad.rs)
- Uses Silero VAD v5.1.2 via whisper-rs `WhisperVadContext`.
- Configured: 1 thread, GPU disabled.
- Segments returned as centisecond timestamps, converted to sample indices.
- Returns `VadResult::NoSpeech` if no segments or all segments are empty after extraction.
- `filter_speech` is `!Send` (WhisperVadContext), must run via `spawn_blocking`.

### Mutex Poison Recovery (lib.rs)
- `MutexExt` trait with `lock_or_recover()`: recovers from poisoned mutexes using `into_inner()` with a warning log. Used throughout the codebase for all state mutexes.

### Telemetry/Logging Architecture (telemetry.rs)
- `TauriEmitterLayer`: Custom tracing_subscriber Layer that:
  1. Collects tracing fields via `JsonVisitor` into serde_json values
  2. Builds an `AppEvent` struct (timestamp, stream, level, summary, data)
  3. Pushes to ring buffer (capped at 500, evicts oldest)
  4. Writes JSONL line to file
  5. Emits "app-event" to all Tauri windows
- `init()`: Sets up global tracing subscriber with EnvFilter "info" level, pretty file layer, and emitter layer. Leaks the non-blocking writer guard to keep it alive for app lifetime.
- Separate log files for dev vs release builds.

### State Architecture (state.rs, lib.rs)
- `State` (lib.rs): Top-level Tauri managed state. Contains `AppState` and `notch_info: Mutex<Option<(f64, f64)>>`.
- `AppState` (state.rs): Contains `dictation: Mutex<DictationState>` and `backend: Mutex<Box<dyn TranscriptionBackend>>`.
- `DictationState`: `status` (DictationStatus), `model_name` (String, default "base.en"), `language` (String, default "en"), `auto_paste` (bool, default false), `auto_paste_delay_ms` (u64, default 50), `vad_sensitivity` (u32, default 50).

## Commands / Hooks / Events

### Tauri Commands

| Command | Description |
|---------|-------------|
| `init_dictation` | Returns a JSON response indicating the dictation system is initialized and idle. |
| `process_audio` | Accepts base64-encoded WAV audio data, decodes it, runs VAD + transcription pipeline, and returns transcribed text. |
| `get_status` | Returns current dictation status (idle/recording/processing), model name, and language. |
| `configure_dictation` | Updates dictation settings: model, language, autoPaste, autoPasteDelayMs, vadSensitivity. Swaps backend if model type changes. |
| `start_native_recording` | Begins native audio capture with optional device name; sets status to Recording. |
| `stop_native_recording` | Stops native recording, runs VAD + transcription pipeline, injects text, and returns transcribed text. |
| `cancel_native_recording` | Cancels recording without transcription (discards audio); used by Both mode for speculative recordings from short taps. |
| `open_system_preferences` | Opens macOS System Settings to the Microphone privacy pane. |
| `check_accessibility_permission` | Returns boolean indicating whether macOS Accessibility permission is granted. |
| `request_accessibility_permission` | Triggers macOS Accessibility permission prompt and opens System Settings Accessibility pane. |
| `request_microphone_permission` | Opens macOS System Settings to the Microphone privacy pane. |
| `list_audio_devices` | Returns list of available audio input device names. |
| `start_keyboard_listener` | Starts the rdev keyboard listener with given hotkey and mode (double_tap/hold_down/both). Validates mode and requires Accessibility permission. |
| `stop_keyboard_listener` | Stops processing keyboard events (thread stays alive but idle). |
| `update_keyboard_key` | Updates the target hotkey without restarting the listener. Emits hold-down-stop if key changed while held. |
| `set_keyboard_recording` | Tells the double-tap detector whether recording is active (affects single-tap-to-stop behavior). |
| `get_log_contents` | Returns the last N lines from the pretty-printed log file. |
| `clear_logs` | Removes all log files and clears the in-memory event ring buffer. |
| `log_frontend` | Routes a frontend log message (level + message) through Rust tracing. |
| `open_log_viewer` | Shows and focuses the log-viewer window. |
| `check_model_exists` | Returns true if any model file exists for either backend (whisper or moonshine). |
| `check_specific_model_exists` | Returns true if a specific model file/directory exists on disk (with path traversal protection). |
| `download_model` | Downloads a model (whisper or moonshine) with streaming progress events, plus co-downloads VAD model if missing. |
| `update_tray_icon` | No-op command (tray icon is static white). |
| `show_overlay` | Positions and shows the always-on-top overlay window. |
| `hide_overlay` | Hides the overlay window. |
| `get_notch_info` | Returns cached notch dimensions (notch_width, notch_height) or None if no notch. |
| `get_event_history` | Returns all AppEvent entries from the in-memory ring buffer (up to 500). |
| `clear_event_history` | Clears the in-memory event ring buffer. |
| `get_resource_usage` | Returns current CPU percentage and used memory in MB. |

### Emitted Events (Rust -> Frontend)

| Event | Payload | Description |
|-------|---------|-------------|
| `audio-level` | f32 (RMS 0.0-1.0) | Real-time audio level during recording, throttled to ~60fps. |
| `recording-status-changed` | String ("recording", "processing", "idle", "downloading-vad") | Status transitions during the recording/transcription lifecycle. |
| `transcription-complete` | `{text, duration}` | Broadcast to all windows after successful transcription with non-empty text. |
| `auto-paste-failed` | String (paste hint) | Emitted when auto-paste fails (text is in clipboard, user can paste manually). |
| `download-progress` | `{received, total}` | Streaming download progress for model files. |
| `double-tap-toggle` | () | Double-tap hotkey detected (toggle recording). |
| `hold-down-start` | () | Hold-down hotkey press detected (start recording). |
| `hold-down-stop` | () | Hold-down hotkey release detected (stop recording). |
| `hold-down-cancel` | -- | **NOT EMITTED**. The frontend (`useCombinedToggle.ts` line 64) registers a listener for this event, but no Rust code ever emits it. This is dead code on the frontend side. In Both mode, short taps that are not promoted simply emit nothing. |
| `keyboard-listener-error` | String (error message) | rdev listener thread encountered an error. |
| `notch-info-changed` | Optional NotchInfo `{notch_width, notch_height}` | Screen parameters changed; notch dimensions updated. |
| `app-event` | AppEvent `{timestamp, stream, level, summary, data}` | Every tracing event is emitted as a structured event to all windows. |

### Modules Declared in lib.rs

`audio`, `commands`, `injector`, `keyboard`, `resource_monitor`, `state`, `telemetry` (pub), `transcriber` (pub), `vad`.

## Gaps / Unclear

1. **Hardcoded hotkey mappings**: Only three hotkeys are supported (`shift_l`, `alt_l`, `ctrl_r`). The `hotkey_to_rdev_key()` function returns `None` for any other string, which silently disables the keyboard detector (target_key is None, handle_event always returns false). No error is reported to the user if an unsupported hotkey is configured.

2. **Moonshine language parameter ignored**: `MoonshineBackend::transcribe()` takes a `_language` parameter that is completely ignored. The moonshine models are English-only, but the user could configure a non-English language and switch to a moonshine model without any warning.

3. **Whisper logging suppression is global and permanent**: `suppress_whisper_logs()` installs logging hooks via `Once`, routing whisper.cpp logs to Rust's log crate. Since only `tracing` is configured (not `log`), whisper.cpp logs effectively go nowhere. This is intentional but could make debugging whisper issues harder.

4. **No frontend log file writing**: The `log_frontend` command routes frontend messages through tracing (which writes to app.log and events.jsonl), but the old `frontend.log` / `frontend.dev.log` files listed in `clear_all_logs()` suggest there was previously a separate frontend log file mechanism that has been removed. These filenames are still cleaned up on log clear, which is fine but indicates legacy code paths.

5. **`update_tray_icon` is a no-op**: The command exists in the invoke_handler but does nothing. The comment says "tray icon is static white. Kept so the registered command doesn't break." This suggests the frontend still calls this command even though it has no effect.

6. **`open_system_preferences` and `request_microphone_permission` do the same thing**: Both open the same System Settings pane (Privacy_Microphone). They appear to be separate commands for semantic reasons but are functionally identical on macOS.

7. **VAD model path assumption**: `vad_model_path()` returns `data_dir/local-dictation/models/{filename}`, but the whisper and moonshine backends each have their own `models_dir()` that returns the same path. The VAD model always lives in the same directory as transcription models, but this coupling is implicit.

8. **Error handling on stream_download**: If the download stream fails partway, the temp file is removed but no partial download resume is supported. Large model downloads (e.g., large-v3-turbo) would need to restart from scratch.

9. **`is_recording()` function is `#[allow(dead_code)]`**: Defined in audio.rs but apparently unused. May be leftover from a previous implementation.

10. **Screen change observer is leaked**: The NSNotificationCenter observer is intentionally leaked (`std::mem::forget`) for app-lifetime observation. This is noted in the code comment but could surprise anyone looking for cleanup logic.

11. **`_setPreventsActivation:` private API**: Used to prevent overlay clicks from activating the app. The code checks `respondsToSelector:` for safety, but this is an undocumented API that could break in future macOS versions.

12. **Transcription timing in `stop_native_recording`**: The `recording_secs` calculation uses integer division (`samples.len() / 16_000`) which truncates. A 0.5-second recording would show as 0 seconds. The field name is `duration` in the emitted event, but it's rounded down.

13. **`process_audio` command (base64 WAV path)**: This appears to be an older code path where audio is sent from the frontend as base64-encoded WAV. The newer native recording path (`start_native_recording`/`stop_native_recording`) captures audio directly in Rust. Both paths still exist and are registered. The `process_audio` path does not check for minimum recording duration like `stop_native_recording` does.

14. **Backend default is always WhisperBackend**: `AppState::default()` creates `WhisperBackend::new()`, but if the user's saved settings specify a moonshine model, the backend won't be swapped until `configure_dictation` is called. There's a potential first-transcription failure if the frontend doesn't call `configure_dictation` before the first recording.

15. **Moonshine provider hardcoded to CPU**: `MoonshineConfig.provider` is set to `Some("cpu".to_string())`. There's no option for GPU acceleration for Moonshine models, unlike Whisper which uses Metal.

16. **`hold-down-cancel` event is never emitted (dead frontend listener)**: The frontend `useCombinedToggle.ts` (line 64) registers a listener for `hold-down-cancel` that would call `cancel_native_recording`. However, this event name does not appear anywhere in `keyboard.rs` or any other Rust source file. In Both mode, short taps (released before the 200ms timer promotes them) simply fall through with no event emitted -- the comment at line 596 reads "short single tap, no recording was started, nothing to do". In hold-down-only mode, every press/release pair emits `hold-down-start` and `hold-down-stop` regardless of duration, so there is no cancellation path there either. The `hold-down-cancel` listener is dead code.

## Notes

1. **Extensive test coverage**: The keyboard module has 25+ unit tests covering double-tap, hold-down, both-mode, cooldowns, timeouts, modifier+letter rejection, key repeat handling, and full start-stop cycles. Audio module has 9 tests for RMS/peak computation. Transcriber module has 7 tests for WAV parsing. Recording module has 3 tests for IdleGuard. Tray module has 3 tests for icon generation.

2. **Thread model**: The app uses at least 4 background threads:
   - rdev keyboard listener thread (spawned once, runs forever)
   - Keyboard heartbeat monitor thread (spawned alongside listener)
   - Audio capture thread (spawned per recording session)
   - Hold-promotion timer threads (spawned per Both-mode key press, short-lived)
   Plus Tokio async tasks for VAD (spawn_blocking), downloads, and text injection dispatch.

3. **Data directory**: All models and logs are stored under `~/Library/Application Support/local-dictation/` (the app's data directory name is "local-dictation", not "murmur" -- this appears to be the original project name).

4. **Version logging**: On startup, the app logs its version from `CARGO_PKG_VERSION` (currently "Murmur v{version}").

5. **Plugins**: The app uses 5 Tauri plugins: `tauri_plugin_opener`, `tauri_plugin_autostart`, `tauri_plugin_updater`, `tauri_plugin_notification`, `tauri_plugin_process`.

6. **Structured events architecture**: The telemetry system uses tracing "targets" as stream names (pipeline, audio, system, keyboard). Each tracing event becomes an AppEvent with timestamp, stream, level, summary (the message), and structured data (all other fields as JSON). This powers the log viewer window and provides a unified observability layer.

7. **Text injection runs on main thread**: `inject_text` is dispatched to the main thread via `app_handle.run_on_main_thread()`, with a 2-second timeout on the oneshot channel response. This is necessary because clipboard operations may need the main thread on macOS.

8. **Pipeline timing telemetry**: Each transcription logs detailed timing breakdowns: `vad_ms`, `inference_ms`, `paste_ms`, `total_ms`, plus `audio_secs`, `word_count`, `char_count`, `model`, and `backend`. These are emitted as structured tracing fields visible in the event system.

9. **Two windows defined**: "main" (the settings/UI window) and "overlay" (the notch bar). A third window "log-viewer" is referenced by commands but its creation is handled by Tauri's window configuration (tauri.conf.json), not in Rust code. All three windows are hidden rather than destroyed on close.

10. **Whisper.cpp log suppression**: `install_logging_hooks()` is called once via `std::sync::Once` to route whisper.cpp's verbose C logging through a Rust trampoline that effectively drops the logs since only `tracing` (not the `log` crate) is wired up.
