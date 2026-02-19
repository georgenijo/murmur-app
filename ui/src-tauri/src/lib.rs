// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
mod audio;
mod injector;
mod keyboard;
mod logging;
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

    // Initialize whisper context if needed and transcribe
    let text = {
        let mut ctx_guard = app_state.whisper_context.lock_or_recover();
        if ctx_guard.is_none() {
            *ctx_guard = Some(transcriber::init_whisper_context(&model_name)?);
        }
        let ctx = ctx_guard.as_ref().unwrap();
        transcriber::transcribe(ctx, samples, &language)?
    };

    // Inject text on main thread (macOS requires keyboard APIs to run on main thread)
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

    // Decode base64 audio and parse WAV to samples
    let wav_bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &audio_data)
        .map_err(|e| format!("Failed to decode base64: {}", e))?;
    let samples = transcriber::parse_wav_to_samples(&wav_bytes)?;

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

/// Request accessibility permission (opens System Settings on macOS)
#[tauri::command]
fn request_accessibility_permission() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    { return open_system_preference_pane("Privacy_Accessibility"); }
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
    let _ = app_handle.emit("recording-status-changed", "processing");

    // Guard resets status to Idle if stop_recording fails or samples are empty;
    // disarmed before handing off to run_transcription_pipeline (which has its own guard)
    let mut guard = IdleGuard::new(&state.app_state);

    let samples = audio::stop_recording().map_err(|e| {
        log_error!("stop_native_recording: stop_recording failed: {}", e);
        e
    })?;

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

    log_info!("stop_native_recording: transcribed {} chars", text.len());
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
        _            => (140u8, 140u8, 140u8), // idle — gray
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
            hide_overlay
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
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

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
