//! File-based logging to a per-user directory (e.g. Application Support/local-dictation/logs).
//! Single log file with size-based rotation; thread-safe append.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

static LOG_MUX: Mutex<()> = Mutex::new(());

const MAX_LOG_SIZE: u64 = 5 * 1024 * 1024; // 5 MB
const LOG_FILE: &str = if cfg!(debug_assertions) { "app.dev.log" } else { "app.log" };
const ROTATED_FILE: &str = if cfg!(debug_assertions) { "app.dev.log.1" } else { "app.log.1" };
const FRONTEND_LOG_FILE: &str = if cfg!(debug_assertions) { "frontend.dev.log" } else { "frontend.log" };
const FRONTEND_ROTATED_FILE: &str = if cfg!(debug_assertions) { "frontend.dev.log.1" } else { "frontend.log.1" };
const TRANSCRIPTION_LOG_FILE: &str = if cfg!(debug_assertions) { "transcriptions.dev.jsonl" } else { "transcriptions.jsonl" };
const TRANSCRIPTION_ROTATED_FILE: &str = if cfg!(debug_assertions) { "transcriptions.dev.jsonl.1" } else { "transcriptions.jsonl.1" };
const MAX_TRANSCRIPTION_LOG_SIZE: u64 = 10 * 1024 * 1024; // 10 MB

fn log_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("local-dictation").join("logs"))
}

fn ensure_log_dir() -> Option<PathBuf> {
    let dir = log_dir()?;
    fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// Format current time as ISO 8601 UTC (e.g. "2026-02-17T11:30:45Z").
fn iso_timestamp() -> String {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();

    // Convert to civil time components (UTC, no TZ library needed for log files)
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Days since 1970-01-01 to Y-M-D (algorithm from Howard Hinnant)
    let z = days as i64 + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1461 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, m, d, hours, minutes, seconds)
}

/// Rotate a log file if it exceeds `max_size`. Keeps one rotated backup.
fn rotate_if_needed(dir: &PathBuf, log_file: &str, rotated_file: &str, max_size: u64) {
    let path = dir.join(log_file);
    let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    if size >= max_size {
        let rotated = dir.join(rotated_file);
        let _ = fs::rename(&path, &rotated);
    }
}

fn log_to_file(log_file: &str, rotated_file: &str, level: &str, message: &str) {
    let _guard = LOG_MUX.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = match ensure_log_dir() {
        Some(d) => d,
        None => return,
    };

    rotate_if_needed(&dir, log_file, rotated_file, MAX_LOG_SIZE);

    let path = dir.join(log_file);
    let mut file = match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(f) => f,
        Err(_) => return,
    };
    let line = format!("{} [{}] {}\n", iso_timestamp(), level, message);
    let _ = file.write_all(line.as_bytes());
    let _ = file.flush();
}

fn log_impl(level: &str, message: &str) {
    log_to_file(LOG_FILE, ROTATED_FILE, level, message);
}

/// Log an informational message.
pub fn info(message: &str) {
    log_impl("INFO", message);
}

/// Log a warning.
pub fn warn(message: &str) {
    log_impl("WARN", message);
}

/// Log an error (e.g. for debugging failures).
pub fn error(message: &str) {
    log_impl("ERROR", message);
}

/// Log with format args (convenience).
#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {{
        $crate::logging::info(&format!($($arg)*));
    }};
}
#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => {{
        $crate::logging::warn(&format!($($arg)*));
    }};
}
#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => {{
        $crate::logging::error(&format!($($arg)*));
    }};
}

/// Write a frontend log line to the separate frontend log file.
pub fn frontend(level: &str, message: &str) {
    log_to_file(FRONTEND_LOG_FILE, FRONTEND_ROTATED_FILE, level, message);
}

/// Return the last `n` lines of the active log file as a newline-joined string.
pub fn read_last_lines(n: usize) -> String {
    let _guard = LOG_MUX.lock().unwrap_or_else(|p| p.into_inner());
    let dir = match log_dir() {
        Some(d) => d,
        None => return String::new(),
    };
    let path = dir.join(LOG_FILE);
    let content = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return String::new(),
    };
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

/// Append a JSONL entry to the transcription log with model metadata and output text.
pub fn log_transcription(model: &str, backend: &str, audio_secs: f64, transcribe_secs: f64, text: &str) {
    let _guard = LOG_MUX.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = match ensure_log_dir() {
        Some(d) => d,
        None => return,
    };

    rotate_if_needed(&dir, TRANSCRIPTION_LOG_FILE, TRANSCRIPTION_ROTATED_FILE, MAX_TRANSCRIPTION_LOG_SIZE);

    let path = dir.join(TRANSCRIPTION_LOG_FILE);
    let mut file = match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(f) => f,
        Err(_) => return,
    };
    let entry = serde_json::json!({
        "ts": iso_timestamp(),
        "model": model,
        "backend": backend,
        "audio_secs": audio_secs,
        "transcribe_secs": transcribe_secs,
        "text": text,
    });
    let mut line = entry.to_string();
    line.push('\n');
    let _ = file.write_all(line.as_bytes());
    let _ = file.flush();
}

/// Truncate the active log file to zero bytes.
pub fn clear_logs() -> Result<(), String> {
    let _guard = LOG_MUX.lock().unwrap_or_else(|p| p.into_inner());
    let dir = match ensure_log_dir() {
        Some(d) => d,
        None => return Ok(()),
    };
    let path = dir.join(LOG_FILE);
    fs::write(&path, b"").map_err(|e| e.to_string())
}
