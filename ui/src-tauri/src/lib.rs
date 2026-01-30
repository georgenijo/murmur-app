// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager, State,
};

fn get_project_root() -> std::path::PathBuf {
    // CARGO_MANIFEST_DIR is ui/src-tauri, go up 2 levels to project root
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.parent().unwrap().parent().unwrap().to_path_buf()
}

fn get_python_path() -> std::path::PathBuf {
    get_project_root().join("venv/bin/python")
}

fn get_script_path() -> std::path::PathBuf {
    get_project_root().join("dictation_bridge.py")
}

struct PythonBridge {
    process: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl PythonBridge {
    fn send_command(&mut self, command: &str) -> Result<DictationResponse, String> {
        // Wrap simple commands in JSON format expected by Python
        let json_cmd = format!(r#"{{"cmd": "{}"}}"#, command);
        self.send_raw(&json_cmd)
    }

    fn send_raw(&mut self, json_command: &str) -> Result<DictationResponse, String> {
        // Send the command (already in JSON format)
        writeln!(self.stdin, "{}", json_command)
            .map_err(|e| format!("Failed to write to stdin: {}", e))?;
        self.stdin
            .flush()
            .map_err(|e| format!("Failed to flush stdin: {}", e))?;

        // Read the response
        let mut line = String::new();
        self.stdout
            .read_line(&mut line)
            .map_err(|e| format!("Failed to read from stdout: {}", e))?;

        // Parse the JSON response
        serde_json::from_str(&line)
            .map_err(|e| format!("Failed to parse response '{}': {}", line.trim(), e))
    }
}

impl Drop for PythonBridge {
    fn drop(&mut self) {
        // Try to send quit command gracefully (in JSON format)
        let _ = writeln!(self.stdin, r#"{{"cmd": "quit"}}"#);
        let _ = self.stdin.flush();
        // Kill the process if it doesn't exit
        let _ = self.process.kill();
    }
}

pub struct AppState {
    bridge: Mutex<Option<PythonBridge>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DictationResponse {
    #[serde(rename = "type")]
    pub response_type: String,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub device: Option<String>,
}

#[tauri::command]
fn init_dictation(state: State<AppState>) -> Result<DictationResponse, String> {
    let mut bridge_guard = state
        .bridge
        .lock()
        .map_err(|e| format!("Failed to acquire lock: {}", e))?;

    // Check if already initialized
    if bridge_guard.is_some() {
        return Err("Dictation bridge already initialized".to_string());
    }

    // Spawn the Python process
    let mut process = Command::new(get_python_path())
        .arg("-u") // Unbuffered output
        .arg(get_script_path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn Python process: {}", e))?;

    let stdin = process
        .stdin
        .take()
        .ok_or("Failed to get stdin handle")?;
    let stdout = process
        .stdout
        .take()
        .ok_or("Failed to get stdout handle")?;

    let mut stdout_reader = BufReader::new(stdout);

    // Wait for the "ready" message
    let mut ready_line = String::new();
    stdout_reader
        .read_line(&mut ready_line)
        .map_err(|e| format!("Failed to read ready message: {}", e))?;

    let ready_response: DictationResponse = serde_json::from_str(&ready_line)
        .map_err(|e| format!("Failed to parse ready message '{}': {}", ready_line.trim(), e))?;

    if ready_response.response_type != "ready" {
        return Err(format!(
            "Expected 'ready' message, got '{}'",
            ready_response.response_type
        ));
    }

    // Store the bridge
    *bridge_guard = Some(PythonBridge {
        process,
        stdin,
        stdout: stdout_reader,
    });

    Ok(ready_response)
}

#[tauri::command]
fn start_recording(state: State<AppState>) -> Result<DictationResponse, String> {
    let mut bridge_guard = state
        .bridge
        .lock()
        .map_err(|e| format!("Failed to acquire lock: {}", e))?;

    let bridge = bridge_guard
        .as_mut()
        .ok_or("Dictation bridge not initialized. Call init_dictation first.")?;

    bridge.send_command("start_recording")
}

#[tauri::command]
fn stop_recording(state: State<AppState>) -> Result<DictationResponse, String> {
    let mut bridge_guard = state
        .bridge
        .lock()
        .map_err(|e| format!("Failed to acquire lock: {}", e))?;

    let bridge = bridge_guard
        .as_mut()
        .ok_or("Dictation bridge not initialized. Call init_dictation first.")?;

    bridge.send_command("stop_recording")
}

#[tauri::command]
fn process_audio(state: State<AppState>, audio_data: String) -> Result<DictationResponse, String> {
    let mut bridge_guard = state
        .bridge
        .lock()
        .map_err(|e| format!("Failed to acquire lock: {}", e))?;

    let bridge = bridge_guard
        .as_mut()
        .ok_or("Dictation bridge not initialized. Call init_dictation first.")?;

    // Decode base64 audio data
    let audio_bytes = BASE64
        .decode(&audio_data)
        .map_err(|e| format!("Failed to decode base64 audio: {}", e))?;

    // Write to temporary file
    let temp_dir = std::env::temp_dir();
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let temp_path = temp_dir.join(format!("dictation_recording_{}.wav", timestamp));

    let mut file = File::create(&temp_path)
        .map_err(|e| format!("Failed to create temp file: {}", e))?;
    file.write_all(&audio_bytes)
        .map_err(|e| format!("Failed to write audio data: {}", e))?;
    file.flush()
        .map_err(|e| format!("Failed to flush temp file: {}", e))?;

    // Send transcribe command to Python with file path
    let command = serde_json::json!({
        "cmd": "transcribe_file",
        "path": temp_path.to_string_lossy()
    });

    bridge.send_raw(&command.to_string())
}

#[tauri::command]
fn get_status(state: State<AppState>) -> Result<DictationResponse, String> {
    let mut bridge_guard = state
        .bridge
        .lock()
        .map_err(|e| format!("Failed to acquire lock: {}", e))?;

    let bridge = bridge_guard
        .as_mut()
        .ok_or("Dictation bridge not initialized. Call init_dictation first.")?;

    bridge.send_command("get_status")
}

#[derive(Deserialize, Debug)]
pub struct ConfigureOptions {
    pub model: Option<String>,
    pub language: Option<String>,
}

#[tauri::command]
fn configure_dictation(
    state: State<AppState>,
    options: ConfigureOptions,
) -> Result<DictationResponse, String> {
    let mut bridge_guard = state
        .bridge
        .lock()
        .map_err(|e| format!("Failed to acquire lock: {}", e))?;

    let bridge = bridge_guard
        .as_mut()
        .ok_or("Dictation bridge not initialized. Call init_dictation first.")?;

    // Build the JSON command
    let command = serde_json::json!({
        "cmd": "configure",
        "model": options.model,
        "language": options.language
    });

    bridge.send_raw(&command.to_string())
}

#[tauri::command]
fn open_system_preferences() -> Result<(), String> {
    std::process::Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone")
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(AppState {
            bridge: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            init_dictation,
            start_recording,
            stop_recording,
            process_audio,
            get_status,
            configure_dictation,
            open_system_preferences
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
