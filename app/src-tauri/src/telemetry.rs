//! Structured telemetry: tracing subscriber with file + event-emitter layers.

use std::collections::VecDeque;
use std::io::Write;
use std::sync::{Arc, Mutex, OnceLock};
use tauri::Emitter;

/// A structured event emitted to the frontend and stored in the ring buffer.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AppEvent {
    pub timestamp: String,
    pub stream: String,
    pub level: String,
    pub summary: String,
    pub data: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Shared ring buffer
// ---------------------------------------------------------------------------

static EVENT_BUFFER: OnceLock<Arc<Mutex<VecDeque<AppEvent>>>> = OnceLock::new();

fn get_event_buffer() -> Arc<Mutex<VecDeque<AppEvent>>> {
    EVENT_BUFFER
        .get_or_init(|| Arc::new(Mutex::new(VecDeque::with_capacity(500))))
        .clone()
}

// ---------------------------------------------------------------------------
// JsonVisitor — collects tracing fields into serde_json values
// ---------------------------------------------------------------------------

struct JsonVisitor {
    fields: serde_json::Map<String, serde_json::Value>,
    message: Option<String>,
}

impl JsonVisitor {
    fn new() -> Self {
        Self {
            fields: serde_json::Map::new(),
            message: None,
        }
    }
}

impl tracing::field::Visit for JsonVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        } else {
            self.fields.insert(
                field.name().to_string(),
                serde_json::Value::String(value.to_string()),
            );
        }
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.fields.insert(
            field.name().to_string(),
            serde_json::Value::Number(value.into()),
        );
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.fields.insert(
            field.name().to_string(),
            serde_json::Value::Number(value.into()),
        );
    }

    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        let num = serde_json::Number::from_f64(value)
            .map(serde_json::Value::Number)
            .unwrap_or_else(|| serde_json::Value::String(value.to_string()));
        self.fields.insert(field.name().to_string(), num);
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.fields
            .insert(field.name().to_string(), serde_json::Value::Bool(value));
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let s = format!("{:?}", value);
        if field.name() == "message" {
            self.message = Some(s);
        } else {
            self.fields
                .insert(field.name().to_string(), serde_json::Value::String(s));
        }
    }
}

// ---------------------------------------------------------------------------
// TauriEmitterLayer — custom tracing layer that emits events to the frontend
// ---------------------------------------------------------------------------

pub struct TauriEmitterLayer {
    app_handle: tauri::AppHandle,
    buffer: Arc<Mutex<VecDeque<AppEvent>>>,
    jsonl_writer: Mutex<std::io::BufWriter<std::fs::File>>,
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for TauriEmitterLayer {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let meta = event.metadata();

        // Stream = target (e.g. "pipeline", "audio", "system")
        let stream = meta.target().to_string();

        // Level
        let level = match *meta.level() {
            tracing::Level::TRACE => "trace",
            tracing::Level::DEBUG => "debug",
            tracing::Level::INFO => "info",
            tracing::Level::WARN => "warn",
            tracing::Level::ERROR => "error",
        }
        .to_string();

        // Collect fields
        let mut visitor = JsonVisitor::new();
        event.record(&mut visitor);

        let summary = visitor.message.unwrap_or_default();
        let mut data = serde_json::Value::Object(visitor.fields);

        sanitize_event_data(&stream, &mut data, cfg!(debug_assertions));

        let timestamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

        let app_event = AppEvent {
            timestamp,
            stream,
            level,
            summary,
            data,
        };

        // Push to ring buffer
        {
            let mut buf = self.buffer.lock().unwrap_or_else(|p| p.into_inner());
            if buf.len() >= 500 {
                buf.pop_front();
            }
            buf.push_back(app_event.clone());
        }

        // Write AppEvent JSON line to JSONL file
        if let Ok(mut writer) = self.jsonl_writer.lock() {
            if let Ok(json) = serde_json::to_string(&app_event) {
                let _ = writeln!(writer, "{}", json);
                let _ = writer.flush();
            }
        }

        // Emit to all windows
        let _ = self.app_handle.emit("app-event", &app_event);
    }
}

/// Enforce the structured-event privacy boundary independently of call-site
/// discipline. Transform traces may retain only stable enum/bucket fields;
/// arbitrary strings (and therefore transform content, paths, app/device
/// identifiers, or raw errors) are discarded in both debug and release builds.
fn is_safe_transform_string(key: &str, value: &str) -> bool {
    match key {
        "event" => matches!(value, "hold_start" | "hold_stop" | "armed" | "stopped"),
        "reason" => matches!(
            value,
            "released"
                | "combo_cancelled"
                | "detector_stop"
                | "escape"
                | "listener_stopped"
                | "key_reconfigured"
        ),
        "from" | "to" => matches!(
            value,
            "idle" | "capturing" | "listening" | "thinking" | "review_pending" | "applying"
        ),
        "outcome" => matches!(
            value,
            "ok" | "error"
                | "failed"
                | "ready"
                | "cancelled"
                | "applied"
                | "undone"
                | "capture_aborted"
                | "empty"
                | "audio_empty"
                | "transcription_error"
                | "transcript_blank"
                | "accessibility_denied"
                | "secure_field"
                | "no_selection"
                | "too_large"
                | "ax_unavailable"
                | "secure_check_failed"
                | "sentinel_write_failed"
                | "ax"
                | "ax_unverified"
                | "paste"
        ),
        "stage" => matches!(
            value,
            "start"
                | "capture"
                | "instruction"
                | "sidecar"
                | "audio_start"
                | "retry_without_session"
                | "retry_audio_start"
                | "apply"
                | "undo"
                | "linger_complete"
                | "superseded"
                | "pipeline_superseded"
                | "idle"
                | "capturing"
                | "listening"
                | "thinking"
                | "review_pending"
                | "applying"
        ),
        "error_code" => matches!(
            value,
            "accessibility_denied"
                | "secure_field"
                | "no_selection"
                | "too_large"
                | "ax_unavailable"
                | "unsupported"
                | "model_not_downloaded"
                | "disabled"
                | "busy"
                | "invalid_request"
                | "crashed"
                | "model_unreadable"
                | "timeout"
                | "cancelled"
                | "output_invalid"
                | "resource_limit"
                | "internal"
                | "no_session"
                | "no_proposed_text"
                | "already_applied"
                | "not_applied"
                | "clipboard_unavailable"
                | "target_gone"
                | "selection_changed"
                | "paste_failed"
                | "stale_pass"
                | "dictation_active"
                | "benchmark_running"
                | "file_transcribing"
                | "runtime_busy"
                | "transform_busy"
                | "audio_start_failed"
                | "no_instruction"
                | "show_failed"
                | "expand_failed"
                | "set_size_failed"
                | "set_position_failed"
                | "window_missing"
                | "hide_failed"
        ),
        "length_bucket" => matches!(
            value,
            "0" | "1-16" | "17-64" | "65-256" | "257-1024" | "1025-4096" | "4097-16384" | ">16384"
        ),
        "via" => matches!(value, "preflight" | "ax_attempt" | "clipboard_fallback"),
        "effect" => matches!(
            value,
            "show" | "hide" | "expand" | "focusable" | "apply" | "undo"
        ),
        "ax_outcome" => matches!(
            value,
            "accessibility_denied"
                | "secure_field"
                | "no_selection"
                | "too_large"
                | "ax_unavailable"
                | "secure_check_failed"
        ),
        _ => false,
    }
}

fn sanitize_event_data(stream: &str, data: &mut serde_json::Value, debug_build: bool) {
    let Some(obj) = data.as_object_mut() else {
        return;
    };
    if !debug_build && stream == "pipeline" {
        obj.retain(|_, value| !value.is_string());
        return;
    }
    if stream == "transform" {
        obj.retain(|key, value| match value.as_str() {
            Some(value) => is_safe_transform_string(key, value),
            None => true,
        });
    }
}

// ---------------------------------------------------------------------------
// init() — set up the global tracing subscriber
// ---------------------------------------------------------------------------

fn jsonl_path() -> Option<std::path::PathBuf> {
    let dir = dirs::data_dir()?.join("local-dictation").join("logs");
    let name = if cfg!(debug_assertions) {
        "events.dev.jsonl"
    } else {
        "events.jsonl"
    };
    Some(dir.join(name))
}

/// Read the last `n` AppEvent entries from the JSONL file to seed the ring buffer.
fn seed_buffer_from_jsonl(buffer: &Arc<Mutex<VecDeque<AppEvent>>>, n: usize) {
    let path = match jsonl_path() {
        Some(p) if p.exists() => p,
        _ => return,
    };
    let content = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return,
    };
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(n);
    let mut buf = buffer.lock().unwrap_or_else(|p| p.into_inner());
    for line in &lines[start..] {
        if let Ok(event) = serde_json::from_str::<AppEvent>(line) {
            if buf.len() >= 500 {
                buf.pop_front();
            }
            buf.push_back(event);
        }
    }
}

/// Rotate the JSONL file if it exceeds 5 MB.
fn rotate_jsonl_if_needed() {
    let path = match jsonl_path() {
        Some(p) => p,
        None => return,
    };
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    if size >= 5 * 1024 * 1024 {
        let rotated = path.with_extension("jsonl.1");
        let _ = std::fs::rename(&path, &rotated);
    }
}

pub fn init(app_handle: tauri::AppHandle) {
    use tracing_subscriber::prelude::*;

    let log_dir = dirs::data_dir()
        .map(|d| d.join("local-dictation").join("logs"))
        .expect("Could not determine log directory");
    std::fs::create_dir_all(&log_dir).ok();

    let log_file_name = if cfg!(debug_assertions) {
        "app.dev.log"
    } else {
        "app.log"
    };

    // Seed ring buffer from existing JSONL before subscribing
    let buffer = get_event_buffer();
    seed_buffer_from_jsonl(&buffer, 500);

    // Rotate JSONL if too large
    rotate_jsonl_if_needed();

    // Open JSONL file for appending (AppEvent format)
    let jsonl_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(jsonl_path().expect("Could not determine JSONL path"))
        .expect("Could not open JSONL file");
    let jsonl_writer = std::io::BufWriter::new(jsonl_file);

    // Layer 1: Pretty file
    let (pretty_writer, pretty_guard) =
        tracing_appender::non_blocking(tracing_appender::rolling::never(&log_dir, log_file_name));
    let pretty_layer = tracing_subscriber::fmt::layer()
        .with_writer(pretty_writer)
        .with_target(true)
        .with_level(true)
        .with_ansi(false);

    // Layer 2: Tauri event emitter (also writes JSONL)
    let emitter_layer = TauriEmitterLayer {
        app_handle,
        buffer,
        jsonl_writer: Mutex::new(jsonl_writer),
    };

    let filter = tracing_subscriber::EnvFilter::new("info");

    let subscriber = tracing_subscriber::registry()
        .with(filter)
        .with(pretty_layer)
        .with(emitter_layer);

    tracing::subscriber::set_global_default(subscriber).expect("Failed to set tracing subscriber");

    // Leak guard to keep writer alive for app lifetime
    Box::leak(Box::new(pretty_guard));
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

pub fn read_pretty_log_tail(n: usize) -> String {
    let dir = match dirs::data_dir().map(|d| d.join("local-dictation").join("logs")) {
        Some(d) => d,
        None => return String::new(),
    };
    let log_file = if cfg!(debug_assertions) {
        "app.dev.log"
    } else {
        "app.log"
    };
    let path = dir.join(log_file);
    let content = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return String::new(),
    };
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

pub fn clear_all_logs() -> Result<(), String> {
    let dir = match dirs::data_dir().map(|d| d.join("local-dictation").join("logs")) {
        Some(d) => d,
        None => return Ok(()),
    };
    // Remove known log files
    let files = [
        "app.log",
        "app.log.1",
        "app.dev.log",
        "app.dev.log.1",
        "frontend.log",
        "frontend.log.1",
        "frontend.dev.log",
        "frontend.dev.log.1",
        "transcriptions.jsonl",
        "transcriptions.jsonl.1",
        "transcriptions.dev.jsonl",
        "transcriptions.dev.jsonl.1",
        "events.jsonl",
        "events.jsonl.1",
        "events.dev.jsonl",
        "events.dev.jsonl.1",
    ];
    for file in files {
        let _ = std::fs::remove_file(dir.join(file));
    }
    // Clean up dated rolling files (e.g. app.dev.log.2026-03-02, events.dev.jsonl.2026-03-02)
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.contains(".log.") || name.contains(".jsonl.") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn get_event_history() -> Vec<AppEvent> {
    let buffer = get_event_buffer();
    let guard = buffer.lock().unwrap_or_else(|p| p.into_inner());
    guard.iter().cloned().collect()
}

#[tauri::command]
pub fn clear_event_history() {
    let buffer = get_event_buffer();
    let mut guard = buffer.lock().unwrap_or_else(|p| p.into_inner());
    guard.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transform_event_sanitizer_keeps_only_stable_string_fields() {
        let mut data = serde_json::json!({
            "transform_pass_id": 17,
            "duration_ms": 12,
            "won": false,
            "outcome": "failed",
            "error_code": "timeout",
            "length_bucket": "17-64",
            "instruction": "SENTINEL_INSTRUCTION",
            "selected_text": "SENTINEL_SELECTION",
            "proposal": "SENTINEL_PROPOSAL",
            "clipboard": "SENTINEL_CLIPBOARD",
            "path": "/Users/private/project",
            "bundle_id": "com.private.Editor",
            "device": "Private Microphone",
            "model": "Private Model Setting"
        });
        sanitize_event_data("transform", &mut data, true);
        let encoded = serde_json::to_string(&data).unwrap();

        assert!(encoded.contains("\"transform_pass_id\":17"));
        assert!(encoded.contains("\"error_code\":\"timeout\""));
        assert!(!encoded.contains("SENTINEL"));
        assert!(!encoded.contains("/Users/private"));
        assert!(!encoded.contains("com.private"));
        assert!(!encoded.contains("Private Microphone"));
        assert!(!encoded.contains("Private Model"));
    }

    #[test]
    fn transform_event_sanitizer_keeps_documented_stable_values() {
        let mut data = serde_json::json!({
            "event": "hold_stop",
            "reason": "released",
            "from": "listening",
            "to": "thinking",
            "outcome": "failed",
            "stage": "sidecar",
            "error_code": "timeout",
            "length_bucket": "17-64",
            "via": "clipboard_fallback",
            "effect": "focusable",
            "ax_outcome": "no_selection"
        });

        sanitize_event_data("transform", &mut data, true);

        assert_eq!(data["event"], "hold_stop");
        assert_eq!(data["reason"], "released");
        assert_eq!(data["from"], "listening");
        assert_eq!(data["to"], "thinking");
        assert_eq!(data["outcome"], "failed");
        assert_eq!(data["stage"], "sidecar");
        assert_eq!(data["error_code"], "timeout");
        assert_eq!(data["length_bucket"], "17-64");
        assert_eq!(data["via"], "clipboard_fallback");
        assert_eq!(data["effect"], "focusable");
        assert_eq!(data["ax_outcome"], "no_selection");
    }

    #[test]
    fn transform_event_sanitizer_rejects_content_in_every_allowed_string_field() {
        let sentinel = "SENTINEL transcript /Users/private/project";
        let mut data = serde_json::json!({
            "transform_pass_id": 23,
            "duration_ms": 9,
            "won": true,
            "event": sentinel,
            "reason": sentinel,
            "from": sentinel,
            "to": sentinel,
            "outcome": sentinel,
            "stage": sentinel,
            "error_code": sentinel,
            "length_bucket": sentinel,
            "via": sentinel,
            "effect": sentinel,
            "ax_outcome": sentinel
        });

        sanitize_event_data("transform", &mut data, true);
        let encoded = serde_json::to_string(&data).unwrap();

        assert_eq!(data["transform_pass_id"], 23);
        assert_eq!(data["duration_ms"], 9);
        assert_eq!(data["won"], true);
        assert!(!encoded.contains("SENTINEL"));
        assert!(!encoded.contains("/Users/private"));
        assert_eq!(data.as_object().unwrap().len(), 3);
    }
}
