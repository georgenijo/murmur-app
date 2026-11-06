//! Main-only coordinated Quit compatibility. Permission definitions remain
//! supplied by tauri-plugin-process; updater relaunch has no webview permission.
fn main_only(window: &tauri::WebviewWindow) -> Result<(), String> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err("Process actions belong to the main window".into())
    }
}
#[tauri::command]
fn exit(app: tauri::AppHandle, window: tauri::WebviewWindow, code: i32) -> Result<(), String> {
    main_only(&window)?;
    let _ = code; // shutdown uses the common content-free terminal receipt
    crate::shutdown::request(&app, murmur_core::shutdown::Request::Quit);
    Ok(())
}
pub(crate) fn init() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::new("process")
        .invoke_handler(tauri::generate_handler![exit])
        .build()
}
