// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
mod audio;
mod injector;
mod keyboard;
mod logging;
mod resource_monitor;
mod state;
pub mod transcriber;

use state::{AppState, DictationStatus};
use transcriber::TranscriptionBackend;
use std::sync::{Mutex, MutexGuard};
use tauri::{Emitter, Manager};

/// Helper trait to recover from poisoned mutexes
trait MutexExt<T> {
    fn lock_or_recover(&self) -> MutexGuard<'_, T>;
}

impl<T> MutexExt<T> for Mutex<T> {
    fn lock_or_recover(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|poisoned| {
            log_warn!("Mutex was poisoned, recovering data");
            poisoned.into_inner()
        })
    }
}

struct State {
    app_state: AppState,
    /// Cached notch dimensions (notch_width, menu_bar_height) from setup (main thread).
    notch_info: Mutex<Option<(f64, f64)>>,
}

#[tauri::command]
async fn init_dictation(_state: tauri::State<'_, State>) -> Result<serde_json::Value, String> {
    log_info!("init_dictation");
    Ok(serde_json::json!({
        "type": "initialized",
        "state": "idle"
    }))
}


/// RAII guard that resets dictation status to Idle on drop,
/// ensuring status is restored on any early return or error path.
struct IdleGuard<'a> {
    app_state: &'a AppState,
    disarmed: bool,
}

impl<'a> IdleGuard<'a> {
    fn new(app_state: &'a AppState) -> Self {
        Self { app_state, disarmed: false }
    }

    fn disarm(&mut self) {
        self.disarmed = true;
    }
}

impl Drop for IdleGuard<'_> {
    fn drop(&mut self) {
        if !self.disarmed {
            let mut dictation = self.app_state.dictation.lock_or_recover();
            dictation.status = DictationStatus::Idle;
        }
    }
}

/// Shared transcription pipeline: model init → transcribe → inject text → set idle
fn run_transcription_pipeline(
    samples: &[f32],
    app_handle: &tauri::AppHandle,
    app_state: &AppState,
) -> Result<String, String> {
    // Guard resets status to Idle on any return path (error or success)
    let _guard = IdleGuard::new(app_state);

    // Read all needed state in one lock
    let (model_name, language, auto_paste) = {
        let dictation = app_state.dictation.lock_or_recover();
        (dictation.model_name.clone(), dictation.language.clone(), dictation.auto_paste)
    };

    // Phase: Transcription (includes lazy model load on first run)
    let t_transcribe = std::time::Instant::now();
    let text = {
        let mut backend = app_state.backend.lock_or_recover();
        backend.load_model(&model_name)?;
        backend.transcribe(samples, &language)?
    };
    log_info!("pipeline: transcription ({} samples): {:?}", samples.len(), t_transcribe.elapsed());

    // Phase: Text injection (clipboard write + optional osascript paste)
    let t_inject = std::time::Instant::now();
    if !text.is_empty() {
        let text_to_inject = text.clone();
        let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
        app_handle
            .run_on_main_thread(move || {
                let _ = tx.send(injector::inject_text(&text_to_inject, auto_paste));
            })
            .map_err(|e| format!("Failed to dispatch to main thread: {}", e))?;
        match rx.recv_timeout(std::time::Duration::from_secs(2)) {
            Ok(Err(e)) => log_error!("Text injection failed: {}", e),
            Err(_) => log_warn!("Text injection timed out"),
            Ok(Ok(())) => {}
        }
    }
    log_info!("pipeline: inject (clipboard + paste): {:?}", t_inject.elapsed());

    Ok(text)
    // _guard drops here, setting status to Idle
}

#[tauri::command]
async fn process_audio(
    app_handle: tauri::AppHandle,
    audio_data: String,
    state: tauri::State<'_, State>,
) -> Result<serde_json::Value, String> {
    {
        let mut dictation = state.app_state.dictation.lock_or_recover();
        dictation.status = DictationStatus::Processing;
    }
    let _ = app_handle.emit("recording-status-changed", "processing");

    // Guard resets status to Idle if decode/parse fails before reaching the pipeline
    let mut guard = IdleGuard::new(&state.app_state);

    // Phase: Audio parse (base64 decode + WAV to samples)
    let t_parse = std::time::Instant::now();
    let wav_bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &audio_data)
        .map_err(|e| format!("Failed to decode base64: {}", e))?;
    let samples = transcriber::parse_wav_to_samples(&wav_bytes)?;
    log_info!("pipeline: audio parse (base64 + WAV): {:?}", t_parse.elapsed());

    // Pipeline has its own guard, so disarm this one
    guard.disarm();

    let pipeline_result = run_transcription_pipeline(&samples, &app_handle, &state.app_state);
    let _ = app_handle.emit("recording-status-changed", "idle");
    let text = pipeline_result?;

    Ok(serde_json::json!({
        "type": "transcription",
        "text": text
    }))
}

#[tauri::command]
async fn get_status(state: tauri::State<'_, State>) -> Result<serde_json::Value, String> {
    let dictation = state.app_state.dictation.lock_or_recover();
    Ok(serde_json::json!({
        "type": "status",
        "state": dictation.status,
        "model": dictation.model_name,
        "language": dictation.language
    }))
}

#[tauri::command]
async fn configure_dictation(
    options: serde_json::Value,
    state: tauri::State<'_, State>,
) -> Result<serde_json::Value, String> {
    let model = options.get("model").and_then(|v| v.as_str()).map(String::from);
    let language = options.get("language").and_then(|v| v.as_str()).map(String::from);

    let mut dictation = state.app_state.dictation.lock_or_recover();

    let model_changed = if let Some(m) = model {
        if m != dictation.model_name {
            dictation.model_name = m;
            true
        } else {
            false
        }
    } else {
        false
    };

    if let Some(l) = language {
        dictation.language = l;
    }

    if let Some(auto_paste) = options.get("autoPaste").and_then(|v| v.as_bool()) {
        dictation.auto_paste = auto_paste;
    }

    // If model changed, swap backend type if needed, or just reset for reload
    if model_changed {
        let new_model = dictation.model_name.clone();
        drop(dictation); // Release dictation lock first
        let mut backend = state.app_state.backend.lock_or_recover();
        let needs_swap = transcriber::is_moonshine_model(&new_model) != (backend.name() == "moonshine");
        if needs_swap {
            *backend = if transcriber::is_moonshine_model(&new_model) {
                Box::new(transcriber::MoonshineBackend::new())
            } else {
                Box::new(transcriber::WhisperBackend::new())
            };
            log_info!("Switched transcription backend to {}", backend.name());
        } else {
            backend.reset();
        }
    }

    Ok(serde_json::json!({
        "type": "configured"
    }))
}

#[cfg(target_os = "macos")]
fn open_system_preference_pane(pane: &str) -> Result<(), String> {
    std::process::Command::new("open")
        .arg(format!(
            "x-apple.systempreferences:com.apple.preference.security?{}",
            pane
        ))
        .spawn()
        .map_err(|e| format!("Failed to open System Settings: {}", e))?;
    Ok(())
}

#[tauri::command]
fn open_system_preferences() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    { return open_system_preference_pane("Privacy_Microphone"); }
    #[cfg(not(target_os = "macos"))]
    { Err("System preferences shortcut not supported on this platform".to_string()) }
}

/// Check if accessibility permission is granted (macOS)
#[tauri::command]
fn check_accessibility_permission() -> bool {
    injector::is_accessibility_enabled()
}

/// Request accessibility permission (triggers system prompt + opens System Settings on macOS)
#[tauri::command]
fn request_accessibility_permission() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        // Trigger the system dialog and register the app in the Accessibility list.
        // Return value is the current trust status — we proceed to open System Settings
        // regardless, so the result is intentionally discarded here.
        let _ = injector::request_accessibility_prompt();
        return open_system_preference_pane("Privacy_Accessibility");
    }
    #[cfg(not(target_os = "macos"))]
    { Ok(()) }
}

/// Request microphone permission (opens System Settings on macOS)
#[tauri::command]
fn request_microphone_permission() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    { return open_system_preference_pane("Privacy_Microphone"); }
    #[cfg(not(target_os = "macos"))]
    { Ok(()) }
}

#[tauri::command]
async fn start_native_recording(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, State>,
) -> Result<serde_json::Value, String> {
    // Check and update status in one lock
    {
        let mut dictation = state.app_state.dictation.lock_or_recover();
        if dictation.status == DictationStatus::Recording {
            log_warn!("start_native_recording: already recording");
            return Ok(serde_json::json!({
                "type": "already_recording",
                "state": "recording"
            }));
        }
        dictation.status = DictationStatus::Recording;
    }

    if let Err(e) = audio::start_recording(Some(app_handle.clone())) {
        log_error!("start_native_recording: audio failed: {}", e);
        let mut dictation = state.app_state.dictation.lock_or_recover();
        dictation.status = DictationStatus::Idle;
        return Err(e);
    }
    let _ = app_handle.emit("recording-status-changed", "recording");
    log_info!("start_native_recording: started");

    Ok(serde_json::json!({
        "type": "recording_started",
        "state": "recording"
    }))
}

#[tauri::command]
async fn stop_native_recording(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, State>,
) -> Result<serde_json::Value, String> {
    // Atomic check-and-set in a single lock to avoid TOCTOU gap
    {
        let mut dictation = state.app_state.dictation.lock_or_recover();
        match dictation.status {
            DictationStatus::Processing => return Ok(serde_json::json!({
                "type": "already_processing",
                "state": "processing"
            })),
            DictationStatus::Idle => {
                log_warn!("stop_native_recording: not recording");
                return Ok(serde_json::json!({
                    "type": "not_recording",
                    "state": "idle"
                }));
            }
            DictationStatus::Recording => {
                dictation.status = DictationStatus::Processing;
            }
        }
    }
    log_info!("stop_native_recording: stopping");
    let _ = app_handle.emit("recording-status-changed", "processing");

    // Guard resets status to Idle if stop_recording fails or samples are empty;
    // disarmed before handing off to run_transcription_pipeline (which has its own guard)
    let mut guard = IdleGuard::new(&state.app_state);

    // Phase: Audio teardown + 16kHz resample
    let t_total = std::time::Instant::now();
    let samples = audio::stop_recording().map_err(|e| {
        log_error!("stop_native_recording: stop_recording failed: {}", e);
        e
    })?;
    log_info!("pipeline: audio teardown + resample: {:?}", t_total.elapsed());

    if samples.is_empty() {
        log_info!("stop_native_recording: no audio captured");
        // guard drops on return, resetting status to Idle
        let _ = app_handle.emit("recording-status-changed", "idle");
        return Ok(serde_json::json!({
            "type": "transcription",
            "text": "",
            "state": "idle"
        }));
    }

    // Hand off status management to the pipeline's own guard
    guard.disarm();

    let pipeline_result = run_transcription_pipeline(&samples, &app_handle, &state.app_state);
    let _ = app_handle.emit("recording-status-changed", "idle");
    let text = pipeline_result.map_err(|e| {
        log_error!("stop_native_recording: pipeline failed: {}", e);
        e
    })?;

    let recording_secs = samples.len() / 16_000;
    let word_count = if text.trim().is_empty() { 0 } else { text.split_whitespace().count() };
    let approx_tokens = (word_count as f64 * 1.3).round() as usize;
    log_info!("pipeline: total end-to-end: {:?} (duration={}s words={} tokens={} chars={})",
        t_total.elapsed(), recording_secs, word_count, approx_tokens, text.len());

    // Broadcast transcription result to all windows (so the main window can update
    // its history even when recording was initiated from the overlay).
    if !text.is_empty() {
        let _ = app_handle.emit("transcription-complete", serde_json::json!({
            "text": text,
            "duration": recording_secs
        }));
    }

    Ok(serde_json::json!({
        "type": "transcription",
        "text": text,
        "state": "idle"
    }))
}

#[tauri::command]
fn start_keyboard_listener(app_handle: tauri::AppHandle, hotkey: String, mode: String) -> Result<(), String> {
    const VALID_MODES: &[&str] = &["double_tap", "hold_down"];
    if !VALID_MODES.contains(&mode.as_str()) {
        log_error!("Invalid keyboard listener mode: {}", mode);
        return Err(format!("Invalid mode '{}'. Expected one of: {}", mode, VALID_MODES.join(", ")));
    }
    if !injector::is_accessibility_enabled() {
        return Err("Accessibility permission is required. Please grant it in System Settings.".to_string());
    }
    keyboard::start_listener(app_handle, &hotkey, &mode);
    log_info!("Keyboard listener started: mode={}, key={}", mode, hotkey);
    Ok(())
}

#[tauri::command]
fn stop_keyboard_listener() {
    keyboard::stop_listener();
    log_info!("Keyboard listener stopped");
}

#[tauri::command]
fn update_keyboard_key(app_handle: tauri::AppHandle, hotkey: String) {
    let should_stop = keyboard::set_target_key(&hotkey);
    if should_stop {
        let _ = app_handle.emit("hold-down-stop", ());
        log_info!("Keyboard key changed while held — emitted stop");
    }
    log_info!("Keyboard key updated to: {}", hotkey);
}

#[tauri::command]
fn set_keyboard_recording(recording: bool) {
    keyboard::set_recording_state(recording);
}

#[tauri::command]
fn get_log_contents(lines: usize) -> String {
    logging::read_last_lines(lines)
}

#[tauri::command]
fn clear_logs() -> Result<(), String> {
    logging::clear_logs()
}

#[tauri::command]
fn log_frontend(level: String, message: String) {
    logging::frontend(&level, &message);
}

#[tauri::command]
fn check_model_exists(state: tauri::State<'_, State>) -> bool {
    let backend = state.app_state.backend.lock_or_recover();
    if backend.model_exists() {
        return true;
    }
    // Also check the other backend type so the model downloader screen
    // doesn't appear when a model from the other engine is already installed.
    if backend.name() == "whisper" {
        transcriber::MoonshineBackend::new().model_exists()
    } else {
        transcriber::WhisperBackend::new().model_exists()
    }
}

#[tauri::command]
fn check_specific_model_exists(model_name: String) -> bool {
    if transcriber::is_moonshine_model(&model_name) {
        let backend = transcriber::MoonshineBackend::new();
        let models_dir = match backend.models_dir() {
            Ok(d) => d,
            Err(_) => return false,
        };
        models_dir.join(transcriber::moonshine::model_dir_name(&model_name)).exists()
    } else {
        let backend = transcriber::WhisperBackend::new();
        let models_dir = match backend.models_dir() {
            Ok(d) => d,
            Err(_) => return false,
        };
        models_dir.join(format!("ggml-{}.bin", model_name)).exists()
    }
}

#[tauri::command]
async fn download_model(app_handle: tauri::AppHandle, model_name: String, state: tauri::State<'_, State>) -> Result<(), String> {
    const ALLOWED_MODELS: &[&str] = &[
        "large-v3-turbo", "small.en", "base.en", "tiny.en", "medium.en",
        "moonshine-tiny", "moonshine-base",
    ];
    if !ALLOWED_MODELS.contains(&model_name.as_str()) {
        return Err(format!("Unknown model '{}'. Allowed: {}", model_name, ALLOWED_MODELS.join(", ")));
    }

    let models_dir = state.app_state.backend.lock_or_recover().models_dir()?;
    tokio::fs::create_dir_all(&models_dir)
        .await
        .map_err(|e| format!("Failed to create models directory: {}", e))?;

    if transcriber::is_moonshine_model(&model_name) {
        download_moonshine_model(&app_handle, &model_name, &models_dir).await
    } else {
        download_whisper_model(&app_handle, &model_name, &models_dir).await
    }
}

/// Download a single whisper ggml .bin file from Hugging Face.
async fn download_whisper_model(
    app_handle: &tauri::AppHandle,
    model_name: &str,
    models_dir: &std::path::Path,
) -> Result<(), String> {
    let filename = format!("ggml-{}.bin", model_name);
    let url = format!(
        "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}",
        filename
    );
    let dest_path = models_dir.join(&filename);
    let temp_path = models_dir.join(format!("{}.tmp", filename));

    let received = stream_download(app_handle, &url, &temp_path).await?;

    tokio::fs::rename(&temp_path, &dest_path)
        .await
        .map_err(|e| {
            let _ = std::fs::remove_file(&temp_path);
            format!("Failed to finalize download: {}", e)
        })?;

    log_info!("Model downloaded: {} ({} bytes)", filename, received);
    Ok(())
}

/// Download a moonshine model archive (tar.bz2) and extract it.
async fn download_moonshine_model(
    app_handle: &tauri::AppHandle,
    model_name: &str,
    models_dir: &std::path::Path,
) -> Result<(), String> {
    let archive_name = transcriber::moonshine::archive_filename(model_name);
    let url = transcriber::moonshine::download_url(model_name);
    let temp_path = models_dir.join(format!("{}.tmp", archive_name));

    let received = stream_download(app_handle, &url, &temp_path).await?;

    // Extract tar.bz2 archive on a blocking thread
    let temp_clone = temp_path.clone();
    let models_dir_owned = models_dir.to_path_buf();
    let dir_name = transcriber::moonshine::model_dir_name(model_name);
    let extracted_dir = models_dir.join(&dir_name);
    let extracted_dir_clone = extracted_dir.clone();
    let extraction_result = tokio::task::spawn_blocking(move || {
        let file = std::fs::File::open(&temp_clone)
            .map_err(|e| format!("Failed to open archive: {}", e))?;
        let decompressor = bzip2::read::BzDecoder::new(file);
        let mut archive = tar::Archive::new(decompressor);
        archive
            .unpack(&models_dir_owned)
            .map_err(|e| {
                // Clean up partially extracted directory
                let _ = std::fs::remove_dir_all(&extracted_dir_clone);
                format!("Failed to extract archive: {}", e)
            })?;
        Ok::<(), String>(())
    })
    .await
    .map_err(|e| format!("Extraction task failed: {}", e))?;

    // Clean up temp archive file regardless of extraction result
    let _ = tokio::fs::remove_file(&temp_path).await;

    extraction_result?;

    log_info!("Moonshine model downloaded and extracted: {} ({} bytes)", dir_name, received);
    Ok(())
}

/// Stream a file download with progress events. Returns total bytes received.
async fn stream_download(
    app_handle: &tauri::AppHandle,
    url: &str,
    dest: &std::path::Path,
) -> Result<u64, String> {
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Download request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Download failed with status: {}", response.status()));
    }

    let total = response.content_length().unwrap_or(0);
    let mut received: u64 = 0;

    use tokio::io::AsyncWriteExt;
    let mut file = tokio::fs::File::create(dest)
        .await
        .map_err(|e| format!("Failed to create temp file: {}", e))?;

    let mut stream = response.bytes_stream();
    use futures_util::StreamExt;
    let stream_result = async {
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("Download error: {}", e))?;
            file.write_all(&chunk)
                .await
                .map_err(|e| format!("Failed to write to file: {}", e))?;
            received += chunk.len() as u64;
            let _ = app_handle.emit("download-progress", serde_json::json!({
                "received": received,
                "total": total
            }));
        }
        file.flush()
            .await
            .map_err(|e| format!("Failed to flush file: {}", e))?;
        Ok::<(), String>(())
    }.await;

    if let Err(e) = stream_result {
        let _ = tokio::fs::remove_file(dest).await;
        return Err(e);
    }

    Ok(received)
}

/// Generate 22×22 RGBA pixel data for a solid circle of the given colour.
fn make_tray_icon_data(r: u8, g: u8, b: u8) -> Vec<u8> {
    const SIZE: u32 = 22;
    let mut data = vec![0u8; (SIZE * SIZE * 4) as usize];
    let center = (SIZE as i32) / 2;
    let radius_sq = ((SIZE as i32 / 2) - 2).pow(2);
    for y in 0..SIZE as i32 {
        for x in 0..SIZE as i32 {
            let dx = x - center;
            let dy = y - center;
            if dx * dx + dy * dy <= radius_sq {
                let idx = ((y as u32 * SIZE + x as u32) * 4) as usize;
                data[idx] = r;
                data[idx + 1] = g;
                data[idx + 2] = b;
                data[idx + 3] = 255;
            }
        }
    }
    data
}

/// Update the tray icon to reflect the current dictation state.
/// `icon_state`: "idle" | "recording" | "processing"
#[tauri::command]
fn update_tray_icon(app: tauri::AppHandle, icon_state: String) -> Result<(), String> {
    let (r, g, b) = match icon_state.as_str() {
        "recording"  => (220u8,  50u8,  50u8), // red
        "processing" => (200u8, 150u8,  40u8), // amber
        _ if cfg!(debug_assertions) => (251u8, 191u8, 36u8), // dev — amber
        _            => (140u8, 140u8, 140u8), // prod — gray
    };
    let data = make_tray_icon_data(r, g, b);
    if let Some(tray) = app.tray_by_id("main-tray") {
        tray.set_icon(Some(tauri::image::Image::new(&data, 22, 22)))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Detect notch width and configure the overlay as a notch-level window.
/// Uses native NSScreen APIs — no subprocess needed.
#[cfg(target_os = "macos")]
fn detect_notch_info() -> Option<(f64, f64)> {
    // Returns (notch_width, menu_bar_height) in logical points
    use objc2_app_kit::NSScreen;
    use objc2_foundation::MainThreadMarker;

    let mtm = unsafe { MainThreadMarker::new_unchecked() };
    let screen = NSScreen::mainScreen(mtm)?;
    let insets = screen.safeAreaInsets();
    if insets.top <= 0.0 {
        return None; // No notch
    }
    let frame = screen.frame();
    let left_w = screen.auxiliaryTopLeftArea().size.width;
    let right_w = screen.auxiliaryTopRightArea().size.width;
    let notch_w = frame.size.width - left_w - right_w;
    log_info!("detect_notch_info: notch_w={}, menu_bar_h={}, screen_w={}", notch_w, insets.top, frame.size.width);
    Some((notch_w, insets.top))
}

#[cfg(not(target_os = "macos"))]
fn detect_notch_info() -> Option<(f64, f64)> { None }

/// Raise the overlay window above the menu bar so it overlaps the notch.
#[cfg(target_os = "macos")]
fn raise_window_above_menubar(overlay: &tauri::WebviewWindow) {
    // NSMainMenuWindowLevel = 24, so +1 = 25 puts us just above the menu bar.
    // This is what boring.notch and mew-notch use.
    let raw = overlay.ns_window();
    if let Ok(ptr) = raw {
        let ns_window: &objc2_app_kit::NSWindow = unsafe { &*(ptr.cast()) };
        ns_window.setLevel(25);
        ns_window.setHasShadow(false);
        // Prevent clicking the overlay from activating the app (which unhides the main window).
        // Private API — macOSPrivateApi is already enabled in tauri.conf.json.
        // Tested on macOS 15 (Sequoia). Guard with respondsToSelector in case Apple
        // removes this in a future version.
        let sel = objc2::sel!(_setPreventsActivation:);
        let responds: bool = unsafe { objc2::msg_send![ns_window, respondsToSelector: sel] };
        if responds {
            let _: () = unsafe { objc2::msg_send![ns_window, _setPreventsActivation: true] };
        } else {
            log_warn!("_setPreventsActivation: not available on this macOS version");
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn raise_window_above_menubar(_overlay: &tauri::WebviewWindow) {}

const NOTCH_EXPAND: f64 = 120.0; // 60px expansion room on each side
const FALLBACK_OVERLAY_W: f64 = 200.0;

#[derive(serde::Serialize, Clone)]
struct NotchInfo {
    notch_width: f64,
    notch_height: f64,
}

/// Return cached notch dimensions so the frontend can position content precisely.
#[tauri::command]
fn get_notch_info(state: tauri::State<'_, State>) -> Option<NotchInfo> {
    state.notch_info.lock_or_recover().map(|(w, h)| NotchInfo { notch_width: w, notch_height: h })
}

/// Position and size the overlay to match the notch, anchored at the top of the screen.
/// The window is notch-height tall and wide enough for horizontal expansion.
/// Takes cached notch_info to avoid calling NSScreen APIs off the main thread.
fn position_overlay_default(overlay: &tauri::WebviewWindow, notch_info: Option<(f64, f64)>) {
    let overlay_w = notch_info.map(|(w, _)| w + NOTCH_EXPAND).unwrap_or(FALLBACK_OVERLAY_W);
    let overlay_h = notch_info.map(|(_, h)| h).unwrap_or(37.0);
    log_info!("position_overlay_default: notch_info={:?}, overlay_w={}, overlay_h={}", notch_info, overlay_w, overlay_h);

    // Resize window to match notch area
    if let Err(e) = overlay.set_size(tauri::LogicalSize::new(overlay_w, overlay_h)) {
        log_warn!("position_overlay_default: set_size({}, {}) failed: {}", overlay_w, overlay_h, e);
    }

    // Raise above the menu bar so the window can overlap the notch
    raise_window_above_menubar(overlay);

    if let Some(monitor) = overlay.current_monitor().ok().flatten() {
        let size = monitor.size();
        let sf = monitor.scale_factor();
        let x = (size.width as f64 / sf - overlay_w) / 2.0;
        log_info!("position_overlay_default: x={}, y=0, sf={}", x, sf);
        if let Err(e) = overlay.set_position(tauri::LogicalPosition::new(x, 0.0)) {
            log_warn!("position_overlay_default: set_position({}, 0) failed: {}", x, e);
        }
    } else {
        log_warn!("position_overlay_default: no current monitor, falling back to (100, 100)");
        let _ = overlay.set_position(tauri::LogicalPosition::new(100.0, 100.0));
    }
}

/// Show the always-on-top overlay window.
#[tauri::command]
fn show_overlay(app: tauri::AppHandle, state: tauri::State<'_, State>) -> Result<(), String> {
    let notch = *state.notch_info.lock_or_recover();
    match app.get_webview_window("overlay") {
        Some(overlay) => {
            position_overlay_default(&overlay, notch);
            overlay.show().map_err(|e| e.to_string())?;
            // Re-enable mouse events (focusable:false disables them on macOS)
            let _ = overlay.set_ignore_cursor_events(false);
            Ok(())
        }
        None => {
            log_warn!("show_overlay: overlay window not found — skipping");
            Ok(())
        }
    }
}

/// Hide the always-on-top overlay window.
#[tauri::command]
fn hide_overlay(app: tauri::AppHandle) -> Result<(), String> {
    match app.get_webview_window("overlay") {
        Some(overlay) => overlay.hide().map_err(|e| e.to_string()),
        None => {
            log_warn!("hide_overlay: overlay window not found — skipping");
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: usize = 22;

    #[test]
    fn tray_icon_data_correct_size() {
        let data = make_tray_icon_data(255, 0, 0);
        assert_eq!(data.len(), SIZE * SIZE * 4);
    }

    #[test]
    fn tray_icon_center_pixel_is_opaque_and_colored() {
        let data = make_tray_icon_data(220, 50, 50);
        let idx = (11 * SIZE + 11) * 4;
        assert_eq!(data[idx],     220, "R");
        assert_eq!(data[idx + 1],  50, "G");
        assert_eq!(data[idx + 2],  50, "B");
        assert_eq!(data[idx + 3], 255, "A should be opaque");
    }

    #[test]
    fn tray_icon_corner_pixel_is_transparent() {
        let data = make_tray_icon_data(220, 50, 50);
        // Corners are outside the inscribed circle
        for &(row, col) in &[(0, 0), (0, 21), (21, 0), (21, 21)] {
            let idx = (row * SIZE + col) * 4;
            assert_eq!(data[idx + 3], 0, "corner ({row},{col}) alpha should be 0 (transparent)");
        }
    }

    #[test]
    fn tray_icon_distinct_colors_for_each_state() {
        let idle       = make_tray_icon_data(140, 140, 140);
        let recording  = make_tray_icon_data(220,  50,  50);
        let processing = make_tray_icon_data(200, 150,  40);
        let center = (11 * SIZE + 11) * 4;
        // All three center pixels must differ
        assert_ne!(idle[center],      recording[center]);
        assert_ne!(recording[center], processing[center]);
    }
}


#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .manage(State {
            app_state: AppState::default(),
            notch_info: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            init_dictation,
            process_audio,
            get_status,
            configure_dictation,
            open_system_preferences,
            check_accessibility_permission,
            request_accessibility_permission,
            request_microphone_permission,
            start_native_recording,
            stop_native_recording,
            start_keyboard_listener,
            stop_keyboard_listener,
            update_keyboard_key,
            set_keyboard_recording,
            update_tray_icon,
            show_overlay,
            hide_overlay,
            get_notch_info,
            get_log_contents,
            clear_logs,
            log_frontend,
            check_model_exists,
            check_specific_model_exists,
            download_model,
            resource_monitor::get_resource_usage
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
                log_info!("window hidden on close request");
            }
        })
        .setup(|app| {
            log_info!("app setup — Local Dictation v{}", env!("CARGO_PKG_VERSION"));

            // Cache notch dimensions on the main thread (safe for NSScreen APIs).
            let notch = detect_notch_info();
            {
                let state = app.state::<State>();
                *state.notch_info.lock_or_recover() = notch;
            }

            // Re-enable mouse events on the overlay window.
            // focusable:false sets ignoresMouseEvents=true on macOS;
            // we override that while keeping the window non-activating.
            if let Some(overlay) = app.get_webview_window("overlay") {
                log_info!("setup: overlay window found, enabling cursor events");
                position_overlay_default(&overlay, notch);
                let _ = overlay.show();
                if let Err(e) = overlay.set_ignore_cursor_events(false) {
                    log_warn!("Failed to set overlay cursor events: {}", e);
                }
            } else {
                log_warn!("setup: overlay window NOT found");
            }

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|_, _| {});
}
