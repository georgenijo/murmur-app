//! File-based logging to a per-user directory (e.g. Application Support/local-dictation/logs).
//! One log file per install; thread-safe append.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

static LOG_MUX: Mutex<()> = Mutex::new(());

fn log_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("local-dictation").join("logs"))
}

fn ensure_log_dir() -> Option<PathBuf> {
    let dir = log_dir()?;
    let _ = fs::create_dir_all(&dir);
    Some(dir)
}

fn log_impl(level: &str, message: &str) {
    let _guard = LOG_MUX.lock().ok();
    let dir = match ensure_log_dir() {
        Some(d) => d,
        None => return,
    };
    let path = dir.join("app.log");
    let mut file = match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(f) => f,
        Err(_) => return,
    };
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let line = format!("{} [{}] {}\n", ts, level, message);
    let _ = file.write_all(line.as_bytes());
    let _ = file.flush();
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
