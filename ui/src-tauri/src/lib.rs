// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
mod audio;
mod injector;
mod keyboard;
mod logging;
mod resource_monitor;
mod state;
mod transcriber;

use state::{AppState, DictationStatus};
use std::sync::{Mutex, MutexGuard};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager,
};

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

/// Shared transcription pipeline: whisper init → transcribe → inject text → set idle
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

    // Segment callback: emit partial text to the frontend as each whisper segment decodes.
    // The frontend listens for "transcription-partial" and displays it in-progress.
    let handle_for_cb = app_handle.clone();
    let on_segment = move |text: String| {
        let _ = handle_for_cb.emit("transcription-partial", &text);
    };

    // Phase: Whisper inference (includes lazy context init on first run)
    let t_whisper = std::time::Instant::now();
    let text = {
        let mut ctx_guard = app_state.whisper_context.lock_or_recover();
        if ctx_guard.is_none() {
            *ctx_guard = Some(transcriber::init_whisper_context(&model_name)?);
        }
        let ctx = ctx_guard.as_ref().unwrap();
        transcriber::transcribe(ctx, samples, &language, Some(on_segment))?
    };
    log_info!("pipeline: whisper inference ({} samples): {:?}", samples.len(), t_whisper.elapsed());

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
    let _ = update_tray_icon(app_handle.clone(), "processing".into());

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
    let _ = update_tray_icon(app_handle.clone(), "idle".into());
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

    // If model changed, clear the whisper context so it reloads
    if model_changed {
        drop(dictation); // Release dictation lock first
        let mut ctx = state.app_state.whisper_context.lock_or_recover();
        *ctx = None;
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
    let _ = update_tray_icon(app_handle.clone(), "recording".into());
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
    let _ = update_tray_icon(app_handle.clone(), "processing".into());

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
        let _ = update_tray_icon(app_handle.clone(), "idle".into());
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
    let _ = update_tray_icon(app_handle.clone(), "idle".into());
    let text = pipeline_result.map_err(|e| {
        log_error!("stop_native_recording: pipeline failed: {}", e);
        e
    })?;

    let recording_secs = samples.len() / 16_000;
    let word_count = if text.trim().is_empty() { 0 } else { text.split_whitespace().count() };
    let approx_tokens = (word_count as f64 * 1.3).round() as usize;
    log_info!("pipeline: total end-to-end: {:?} (duration={}s words={} tokens={} chars={})",
        t_total.elapsed(), recording_secs, word_count, approx_tokens, text.len());
    Ok(serde_json::json!({
        "type": "transcription",
        "text": text,
        "state": "idle"
    }))
}

#[tauri::command]
fn start_double_tap_listener(app_handle: tauri::AppHandle, hotkey: String) -> Result<(), String> {
    if !injector::is_accessibility_enabled() {
        return Err("Accessibility permission is required for double-tap mode. Please grant it in System Settings.".to_string());
    }
    keyboard::start_listener(app_handle, &hotkey);
    log_info!("Double-tap listener started for key: {}", hotkey);
    Ok(())
}

#[tauri::command]
fn stop_double_tap_listener() {
    keyboard::stop_listener();
    log_info!("Double-tap listener stopped");
}

#[tauri::command]
fn update_double_tap_key(hotkey: String) {
    keyboard::set_target_key(&hotkey);
    log_info!("Double-tap key updated to: {}", hotkey);
}

#[tauri::command]
fn set_double_tap_recording(recording: bool) {
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
fn check_model_exists() -> bool {
    transcriber::check_model_exists()
}

#[tauri::command]
async fn download_model(app_handle: tauri::AppHandle, model_name: String) -> Result<(), String> {
    const ALLOWED_MODELS: &[&str] = &["large-v3-turbo", "small.en", "base.en"];
    if !ALLOWED_MODELS.contains(&model_name.as_str()) {
        return Err(format!("Unknown model '{}'. Allowed: {}", model_name, ALLOWED_MODELS.join(", ")));
    }

    let models_dir = transcriber::get_models_dir()?;
    tokio::fs::create_dir_all(&models_dir)
        .await
        .map_err(|e| format!("Failed to create models directory: {}", e))?;

    let filename = format!("ggml-{}.bin", model_name);
    let url = format!(
        "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}",
        filename
    );
    let dest_path = models_dir.join(&filename);
    let temp_path = models_dir.join(format!("{}.tmp", filename));

    let client = reqwest::Client::builder()
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Download request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Download failed with status: {}", response.status()));
    }

    let total = response.content_length().unwrap_or(0);
    let mut received: u64 = 0;

    use tokio::io::AsyncWriteExt;
    let mut file = tokio::fs::File::create(&temp_path)
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
        let _ = tokio::fs::remove_file(&temp_path).await;
        return Err(e);
    }

    tokio::fs::rename(&temp_path, &dest_path)
        .await
        .map_err(|e| {
            let _ = std::fs::remove_file(&temp_path);
            format!("Failed to finalize download: {}", e)
        })?;

    log_info!("Model downloaded: {} ({} bytes)", filename, received);
    Ok(())
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
        "recording"  => (220u8, 50u8,  50u8),
        "processing" => (200u8, 150u8, 40u8),
        _ if cfg!(debug_assertions) => (251u8, 191u8, 36u8), // idle dev — amber
        _            => (140u8, 140u8, 140u8), // idle prod — gray
    };
    let data = make_tray_icon_data(r, g, b);
    if let Some(tray) = app.tray_by_id("main-tray") {
        tray.set_icon(Some(tauri::image::Image::new(&data, 22, 22)))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Show the always-on-top overlay window.
#[tauri::command]
fn show_overlay(app: tauri::AppHandle) -> Result<(), String> {
    match app.get_webview_window("overlay") {
        Some(overlay) => overlay.show().map_err(|e| e.to_string()),
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

fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(State {
            app_state: AppState::default(),
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
            start_double_tap_listener,
            stop_double_tap_listener,
            update_double_tap_key,
            set_double_tap_recording,
            update_tray_icon,
            show_overlay,
            hide_overlay,
            get_log_contents,
            clear_logs,
            check_model_exists,
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
            log_info!("app setup");
            let show = MenuItem::with_id(app, "show", "Show Window", true, None::<&str>)?;
            let toggle_overlay = MenuItem::with_id(app, "toggle_overlay", "Toggle Overlay", true, None::<&str>)?;
            let about = MenuItem::with_id(app, "about", "About Local Dictation", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &toggle_overlay, &about, &quit])?;

            // Named ID so we can retrieve and update the icon later via update_tray_icon
            let _tray = TrayIconBuilder::with_id("main-tray")
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show_main_window(app),
                    "toggle_overlay" => {
                        if let Some(overlay) = app.get_webview_window("overlay") {
                            if overlay.is_visible().unwrap_or(false) {
                                let _ = overlay.hide();
                            } else {
                                let _ = overlay.show();
                            }
                        }
                    }
                    "about" => {
                        show_main_window(app);
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.emit("show-about", ());
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray: &tauri::tray::TrayIcon, event: TrayIconEvent| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_main_window(tray.app_handle());
                    }
                })
                .build(app)?;

            // Set the initial tray icon color (amber for dev, gray for prod)
            let _ = update_tray_icon(app.app_handle().clone(), "idle".into());


            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
