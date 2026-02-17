//! File-based logging to a per-user directory (e.g. Application Support/local-dictation/logs).
//! Single log file with size-based rotation; thread-safe append.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

static LOG_MUX: Mutex<()> = Mutex::new(());

const MAX_LOG_SIZE: u64 = 5 * 1024 * 1024; // 5 MB
const LOG_FILE: &str = "app.log";
const ROTATED_FILE: &str = "app.log.1";

fn log_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("local-dictation").join("logs"))
}

fn ensure_log_dir() -> Option<PathBuf> {
    let dir = log_dir()?;
    let _ = fs::create_dir_all(&dir);
    Some(dir)
}

/// Format current time as ISO 8601 local time (e.g. "2026-02-17T11:30:45").
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

/// Rotate log if it exceeds MAX_LOG_SIZE. Keeps one rotated backup.
fn rotate_if_needed(dir: &PathBuf) {
    let path = dir.join(LOG_FILE);
    let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    if size >= MAX_LOG_SIZE {
        let rotated = dir.join(ROTATED_FILE);
        let _ = fs::rename(&path, &rotated);
    }
}

fn log_impl(level: &str, message: &str) {
    let _guard = LOG_MUX.lock().ok();
    let dir = match ensure_log_dir() {
        Some(d) => d,
        None => return,
    };

    rotate_if_needed(&dir);

    let path = dir.join(LOG_FILE);
    let mut file = match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(f) => f,
        Err(_) => return,
    };
    let line = format!("{} [{}] {}\n", iso_timestamp(), level, message);
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
