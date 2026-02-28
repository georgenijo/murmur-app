use crate::logging;

#[tauri::command]
pub fn get_log_contents(lines: usize) -> String {
    logging::read_last_lines(lines)
}

#[tauri::command]
pub fn clear_logs() -> Result<(), String> {
    logging::clear_logs()
}

#[tauri::command]
pub fn log_frontend(level: String, message: String) {
    logging::frontend(&level, &message);
}
