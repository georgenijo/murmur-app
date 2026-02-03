// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
mod audio;
mod injector;
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
            eprintln!("Warning: Mutex was poisoned, recovering data");
            poisoned.into_inner()
        })
    }
}

struct State {
    app_state: AppState,
}

#[tauri::command]
async fn init_dictation(_state: tauri::State<'_, State>) -> Result<serde_json::Value, String> {
    // Lazy initialization - don't load model until first transcription
    Ok(serde_json::json!({
        "type": "initialized",
        "state": "idle"
    }))
}

#[tauri::command]
async fn start_recording(state: tauri::State<'_, State>) -> Result<serde_json::Value, String> {
    let mut dictation = state.app_state.dictation.lock_or_recover();
    dictation.status = DictationStatus::Recording;
    Ok(serde_json::json!({
        "type": "recording_started"
    }))
}

#[tauri::command]
async fn stop_recording(state: tauri::State<'_, State>) -> Result<serde_json::Value, String> {
    let mut dictation = state.app_state.dictation.lock_or_recover();
    dictation.status = DictationStatus::Idle;
    Ok(serde_json::json!({
        "type": "recording_stopped"
    }))
}

#[tauri::command]
async fn process_audio(
    app_handle: tauri::AppHandle,
    audio_data: String,
    state: tauri::State<'_, State>,
) -> Result<serde_json::Value, String> {
    // Set status to processing
    {
        let mut dictation = state.app_state.dictation.lock_or_recover();
        dictation.status = DictationStatus::Processing;
    }

    // Get model name and language
    let (model_name, language) = {
        let dictation = state.app_state.dictation.lock_or_recover();
        (dictation.model_name.clone(), dictation.language.clone())
    };

    // Decode base64 audio
    let wav_bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &audio_data)
        .map_err(|e| format!("Failed to decode base64: {}", e))?;

    // Parse WAV to samples
    let samples = transcriber::parse_wav_to_samples(&wav_bytes)?;

    // Initialize or get whisper context
    let text = {
        let mut ctx_guard = state.app_state.whisper_context.lock_or_recover();

        // Lazy init if needed
        if ctx_guard.is_none() {
            *ctx_guard = Some(transcriber::init_whisper_context(&model_name)?);
        }

        let ctx = ctx_guard.as_ref().unwrap();
        transcriber::transcribe(ctx, &samples, &language)?
    };

    // Get auto_paste setting
    let auto_paste = {
        let dictation = state.app_state.dictation.lock_or_recover();
        dictation.auto_paste
    };

    // Inject text on main thread (macOS requires keyboard APIs to run on main thread)
    if !text.is_empty() {
        let text_to_inject = text.clone();
        app_handle
            .run_on_main_thread(move || {
                if let Err(e) = injector::inject_text(&text_to_inject, auto_paste) {
                    eprintln!("Failed to inject text: {}", e);
                }
            })
            .map_err(|e| format!("Failed to run on main thread: {}", e))?;
    }

    // Set status back to idle
    {
        let mut dictation = state.app_state.dictation.lock_or_recover();
        dictation.status = DictationStatus::Idle;
    }

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

#[tauri::command]
fn open_system_preferences() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone")
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(target_os = "macos"))]
    {
        return Err("System preferences shortcut not supported on this platform".to_string());
    }
    Ok(())
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
    {
        std::process::Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
            .spawn()
            .map_err(|e| format!("Failed to open System Settings: {}", e))?;
    }
    Ok(())
}

/// Request microphone permission (opens System Settings on macOS)
#[tauri::command]
fn request_microphone_permission() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone")
            .spawn()
            .map_err(|e| format!("Failed to open System Settings: {}", e))?;
    }
    Ok(())
}

#[tauri::command]
async fn start_native_recording(state: tauri::State<'_, State>) -> Result<serde_json::Value, String> {
    // Prevent duplicate starts
    {
        let dictation = state.app_state.dictation.lock_or_recover();
        if dictation.status == DictationStatus::Recording {
            return Ok(serde_json::json!({
                "type": "already_recording",
                "state": "recording"
            }));
        }
    }

    // Update dictation status
    {
        let mut dictation = state.app_state.dictation.lock_or_recover();
        dictation.status = DictationStatus::Recording;
    }

    // Start native audio recording
    if let Err(e) = audio::start_recording() {
        let mut dictation = state.app_state.dictation.lock_or_recover();
        dictation.status = DictationStatus::Idle;
        return Err(e);
    }

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
    // Prevent duplicate stops
    {
        let dictation = state.app_state.dictation.lock_or_recover();
        if dictation.status == DictationStatus::Processing {
            return Ok(serde_json::json!({
                "type": "already_processing",
                "state": "processing"
            }));
        }
        if dictation.status == DictationStatus::Idle {
            return Ok(serde_json::json!({
                "type": "not_recording",
                "state": "idle"
            }));
        }
    }

    // Update status to processing
    {
        let mut dictation = state.app_state.dictation.lock_or_recover();
        dictation.status = DictationStatus::Processing;
    }

    // Stop recording and get samples
    let samples = audio::stop_recording()?;

    // Skip if no audio captured
    if samples.is_empty() {
        let mut dictation = state.app_state.dictation.lock_or_recover();
        dictation.status = DictationStatus::Idle;
        return Ok(serde_json::json!({
            "type": "transcription",
            "text": "",
            "state": "idle"
        }));
    }

    // Get model and language settings
    let (model_name, language) = {
        let dictation = state.app_state.dictation.lock_or_recover();
        (dictation.model_name.clone(), dictation.language.clone())
    };

    // Initialize whisper context if needed and transcribe
    let text = {
        let mut ctx_guard = state.app_state.whisper_context.lock_or_recover();

        if ctx_guard.is_none() {
            *ctx_guard = Some(transcriber::init_whisper_context(&model_name)?);
        }

        let ctx = ctx_guard.as_ref().unwrap();
        transcriber::transcribe(ctx, &samples, &language)?
    };

    // Get auto_paste setting
    let auto_paste = {
        let dictation = state.app_state.dictation.lock_or_recover();
        dictation.auto_paste
    };

    // Inject text on main thread (macOS requires keyboard APIs to run on main thread)
    if !text.is_empty() {
        let text_to_inject = text.clone();
        app_handle
            .run_on_main_thread(move || {
                if let Err(e) = injector::inject_text(&text_to_inject, auto_paste) {
                    eprintln!("Failed to inject text: {}", e);
                }
            })
            .map_err(|e| format!("Failed to run on main thread: {}", e))?;
    }

    // Update status to idle
    {
        let mut dictation = state.app_state.dictation.lock_or_recover();
        dictation.status = DictationStatus::Idle;
    }

    Ok(serde_json::json!({
        "type": "transcription",
        "text": text,
        "state": "idle"
    }))
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
            start_recording,
            stop_recording,
            process_audio,
            get_status,
            configure_dictation,
            open_system_preferences,
            check_accessibility_permission,
            request_accessibility_permission,
            request_microphone_permission,
            start_native_recording,
            stop_native_recording
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .setup(|app| {
            // Create tray menu
            let show = MenuItem::with_id(app, "show", "Show Window", true, None::<&str>)?;
            let about = MenuItem::with_id(app, "about", "About Local Dictation", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &about, &quit])?;

            // Create tray icon
            let _tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "about" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                            let _ = window.emit("show-about", ());
                        }
                    }
                    "quit" => {
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app)?;

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
