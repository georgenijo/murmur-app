//! Structured telemetry: tracing subscriber with file + event-emitter layers.

use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use tauri::{Emitter, Manager};

const REFACTOR_TEST_IDENTIFIER: &str = "com.localdictation.refactor-test";
const BENCH_IDENTIFIER: &str = "com.localdictation.bench";
const PRODUCTION_LOG_DIRECTORY: &str = "local-dictation";
const REFACTOR_TEST_LOG_DIRECTORY: &str = "local-dictation-refactor-test";
const BENCH_LOG_DIRECTORY: &str = "local-dictation-bench";

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
static LOG_DIRECTORY: OnceLock<PathBuf> = OnceLock::new();

fn get_event_buffer() -> Arc<Mutex<VecDeque<AppEvent>>> {
    EVENT_BUFFER
        .get_or_init(|| Arc::new(Mutex::new(VecDeque::with_capacity(500))))
        .clone()
}

fn log_directory_for(data_root: &Path, identifier: &str) -> PathBuf {
    let directory = match identifier {
        REFACTOR_TEST_IDENTIFIER => REFACTOR_TEST_LOG_DIRECTORY,
        BENCH_IDENTIFIER => BENCH_LOG_DIRECTORY,
        _ => PRODUCTION_LOG_DIRECTORY,
    };
    data_root.join(directory).join("logs")
}

fn logs_dir() -> Option<PathBuf> {
    LOG_DIRECTORY
        .get()
        .cloned()
        .or_else(|| dirs::data_dir().map(|root| log_directory_for(&root, "com.localdictation")))
}

pub(crate) fn is_internal_bundle(app_handle: &tauri::AppHandle) -> bool {
    matches!(
        app_handle.config().identifier.as_str(),
        REFACTOR_TEST_IDENTIFIER | BENCH_IDENTIFIER
    )
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

        let mut data = serde_json::Value::Object(visitor.fields);

        sanitize_event_data(&stream, &mut data, cfg!(debug_assertions));
        let summary = sanitized_summary(&stream, visitor.message, &data, cfg!(debug_assertions));

        let timestamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

        let app_event = AppEvent {
            timestamp,
            stream,
            level,
            summary,
            data,
        };

        self.app_handle
            .state::<crate::State>()
            .capture_health
            .observe(&app_event);

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
pub(crate) fn canonical_event_code(value: &str) -> Option<&'static str> {
    match value {
        "keyboard.listener_started" => Some("keyboard.listener_started"),
        "keyboard.listener_silent" => Some("keyboard.listener_silent"),
        "keyboard.listener_failed" => Some("keyboard.listener_failed"),
        "audio.capture_backend_timeout" => Some("audio.capture_backend_timeout"),
        "audio.capture_started" => Some("audio.capture_started"),
        "audio.fallback_started" => Some("audio.fallback_started"),
        "audio.capture_ready" => Some("audio.capture_ready"),
        "audio.capture_failed" => Some("audio.capture_failed"),
        "audio.lifecycle_failed" => Some("audio.lifecycle_failed"),
        "pipeline.dictation_requested" => Some("pipeline.dictation_requested"),
        "pipeline.dictation_stop_handoff" => Some("pipeline.dictation_stop_handoff"),
        "pipeline.dictation_terminal" => Some("pipeline.dictation_terminal"),
        "pipeline.dictation_completed" => Some("pipeline.dictation_completed"),
        "pipeline.dictation_failed" => Some("pipeline.dictation_failed"),
        "performance.store_operation_failed" => Some("performance.store_operation_failed"),
        "system.startup_baseline" => Some("system.startup_baseline"),
        "overlay.position_default" => Some("overlay.position_default"),
        "overlay.position_offset_applied" => Some("overlay.position_offset_applied"),
        "overlay.position_read_failed" => Some("overlay.position_read_failed"),
        "transform.pass_outcome" => Some("transform.pass_outcome"),
        "meeting.capture_started" => Some("meeting.capture_started"),
        "meeting.capture_stopped" => Some("meeting.capture_stopped"),
        "meeting.capture_failed" => Some("meeting.capture_failed"),
        "meeting.channel_active" => Some("meeting.channel_active"),
        "meeting.tap_active" => Some("meeting.tap_active"),
        "meeting.tap_destroyed" => Some("meeting.tap_destroyed"),
        "query.pass_state" => Some("query.pass_state"),
        "query.partial_tick" => Some("query.partial_tick"),
        "updater.check_current" => Some("updater.check_current"),
        "updater.check_failed" => Some("updater.check_failed"),
        "updater.install_blocked" => Some("updater.install_blocked"),
        "updater.install_ready" => Some("updater.install_ready"),
        "updater.install_failed" => Some("updater.install_failed"),
        _ => None,
    }
}

fn is_performance_store_event(data: &serde_json::Map<String, serde_json::Value>) -> bool {
    data.get("event_code").and_then(serde_json::Value::as_str)
        == Some("performance.store_operation_failed")
}

fn is_safe_performance_store_field(key: &str, value: &serde_json::Value) -> bool {
    match key {
        "event_code" => value.as_str() == Some("performance.store_operation_failed"),
        "operation" => value.as_str().is_some_and(|value| {
            matches!(
                value,
                "initialize" | "begin" | "update" | "complete" | "read" | "write" | "clear"
            )
        }),
        "error_class" => value.as_str().is_some_and(|value| {
            matches!(
                value,
                "busyLocked"
                    | "storageFull"
                    | "readOnly"
                    | "io"
                    | "corruptIntegrity"
                    | "schemaMigration"
                    | "invalidRecord"
                    | "unavailable"
            )
        }),
        "attempts" => value.as_u64().is_some_and(|value| (1..=3).contains(&value)),
        "recording_id" => value.as_u64().is_some_and(|value| value > 0),
        _ => false,
    }
}

fn is_safe_dictation_outcome(value: &str) -> bool {
    matches!(
        value,
        "success"
            | "no_speech"
            | "too_short"
            | "user_cancelled_starting"
            | "user_cancelled_recording"
            | "user_cancelled_processing"
            | "capture_init_failure"
            | "runtime_interruption"
            | "stop_failure"
            | "pipeline_failure"
            | "superseded"
    )
}

fn is_safe_dictation_error_code(value: &str) -> bool {
    matches!(
        value,
        "none"
            | "empty_audio"
            | "vad_no_speech"
            | "empty_output"
            | "coreml_vad_retry_exhausted"
            | "below_minimum_duration"
            | "cancelled_starting"
            | "cancelled_recording"
            | "cancelled_processing"
            | "missing_context"
            | "stale_owner"
            | "stop_finalization_failed"
            | "transcription_failed"
            | "runtime_failure"
            | "device_changed"
            | "system_sleep"
            | "system_wake"
            | "permission_denied"
            | "device_unavailable"
            | "host_unavailable"
            | "invalid_input"
            | "resource_exhausted"
            | "stream_invalidated"
            | "unsupported_config"
            | "backend_error"
            | "protocol_error"
            | "first_buffer_timeout"
            | "initialization_timeout"
            | "permission_prompt_timeout"
            | "termination_unconfirmed"
            | "worker_panicked"
            | "signature_invalid"
    )
}

fn sanitized_summary(
    stream: &str,
    summary: Option<String>,
    data: &serde_json::Value,
    debug_build: bool,
) -> String {
    if stream == "meeting" {
        // Event codes carry the useful lifecycle meaning. Keep the JSONL/UI
        // summary constant so formatted content cannot leak from a call site.
        "Meeting event".to_string()
    } else if !debug_build {
        // Release summaries must never depend on caller-provided text. Retain
        // useful semantics only through the exact event-code allowlist.
        data.get("event_code")
            .and_then(serde_json::Value::as_str)
            .and_then(canonical_event_code)
            .unwrap_or("Structured event")
            .to_string()
    } else {
        summary.unwrap_or_default()
    }
}

fn is_safe_transform_string(key: &str, value: &str) -> bool {
    match key {
        "event_code" => canonical_event_code(value).is_some(),
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
        "phase" => matches!(
            value,
            "host_model_verification"
                | "helper_spawn"
                | "helper_model_verification"
                | "backend_initialization"
                | "model_load"
                | "ready_handshake"
                | "request_receipt"
                | "first_token"
                | "generation"
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
                | "model_verification_timeout"
                | "model_load_timeout"
                | "handshake_timeout"
                | "generation_timeout"
                | "helper_spawn_failed"
                | "handshake_protocol_failed"
                | "process_exit"
                | "model_verification_failed"
                | "model_load_failed"
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
                | "meeting_active"
                | "runtime_busy"
                | "transform_busy"
                | "query_busy"
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

fn is_safe_query_string(key: &str, value: &str) -> bool {
    match key {
        "event_code" => matches!(value, "query.pass_state" | "query.partial_tick"),
        "outcome" => matches!(
            value,
            "too_short"
                | "empty"
                | "emitted"
                | "in_flight"
                | "unsupported_model"
                | "no_session"
                | "stale"
        ),
        "state" => matches!(
            value,
            "idle" | "connecting" | "listening" | "transcribing" | "running" | "ready" | "failed"
        ),
        "error_code" => matches!(
            value,
            "not_configured"
                | "invalid_executable"
                | "invalid_arguments"
                | "invalid_timeout"
                | "invalid_environment"
                | "environment_unavailable"
                | "busy"
                | "audio_start_failed"
                | "audio_capture_failed"
                | "audio_not_ready"
                | "audio_stalled"
                | "audio_recovering"
                | "audio_recovery_stalled"
                | "no_speech"
                | "transcription_failed"
                | "empty_query"
                | "query_too_large"
                | "spawn_failed"
                | "timed_out"
                | "exit_nonzero"
                | "provider_not_authenticated"
                | "provider_error"
                | "empty_answer"
                | "auto_copy_disabled"
                | "auto_copy_unavailable"
                | "clipboard_unavailable"
                | "clipboard_superseded"
                | "output_too_large"
                | "process_failed"
                | "termination_unconfirmed"
                | "cancelled"
        ),
        _ => false,
    }
}

fn is_safe_query_field(key: &str, value: &serde_json::Value) -> bool {
    match key {
        "event_code" | "state" | "outcome" => value
            .as_str()
            .is_some_and(|value| is_safe_query_string(key, value)),
        "error_code" => {
            value.is_null()
                || value
                    .as_str()
                    .is_some_and(|value| is_safe_query_string(key, value))
        }
        "query_pass_id"
        | "sample_count"
        | "input_tokens"
        | "output_tokens"
        | "reasoning_output_tokens"
        | "cached_input_tokens"
        | "cache_creation_input_tokens"
        | "cost_microusd" => value.as_u64().is_some(),
        _ => false,
    }
}

fn sanitize_event_data(stream: &str, data: &mut serde_json::Value, debug_build: bool) {
    let Some(obj) = data.as_object_mut() else {
        return;
    };
    if is_performance_store_event(obj) {
        // Performance-store failures are shipped for fleet regression watches.
        // Treat them as a dedicated schema even in debug builds so a future
        // call site cannot add SQLite text, SQL, paths, or captured content.
        obj.retain(|key, value| is_safe_performance_store_field(key, value));
        return;
    }
    if !debug_build && stream == "pipeline" {
        obj.retain(|key, value| {
            !value.is_string()
                || value.as_str().is_some_and(|value| match key.as_str() {
                    "event_code" => canonical_event_code(value).is_some(),
                    "outcome" => is_safe_dictation_outcome(value),
                    "error_code" => is_safe_dictation_error_code(value),
                    _ => false,
                })
        });
        return;
    }
    if stream == "transform" {
        obj.retain(|key, value| match value.as_str() {
            Some(value) => is_safe_transform_string(key, value),
            None => true,
        });
    } else if stream == "query" {
        // Query content may contain anything the user can say or an agent can
        // return. Use an exact key-and-type allowlist, including for structured
        // values, so future instrumentation cannot accidentally retain it.
        obj.retain(|key, value| is_safe_query_field(key, value));
    }
    if stream == "meeting" {
        obj.retain(|key, value| match value.as_str() {
            Some(value) => match key.as_str() {
                "event_code" => canonical_event_code(value).is_some(),
                "phase" => matches!(
                    value,
                    "idle" | "starting" | "recording" | "stopping" | "processing" | "failed"
                ),
                "channel" => matches!(value, "microphone" | "system" | "both" | "none"),
                "error_code" => matches!(
                    value,
                    "unsupported_os"
                        | "system_audio_permission_denied"
                        | "microphone_permission_denied"
                        | "microphone_unavailable"
                        | "system_audio_unavailable"
                        | "system_audio_callback_stalled"
                        | "microphone_callback_stalled"
                        | "permission_prompt_timeout"
                        | "capture_setup_timeout"
                        | "protocol_error"
                        | "capture_backlog"
                        | "capture_failed"
                        | "capture_stop_timeout"
                        | "supervisor_panicked"
                        | "termination_unconfirmed"
                        | "spool_failed"
                        | "store_unavailable"
                        | "transcription_failed"
                        | "none"
                ),
                _ => false,
            },
            None => true,
        });
    }
}

// ---------------------------------------------------------------------------
// init() — set up the global tracing subscriber
// ---------------------------------------------------------------------------

fn jsonl_path() -> Option<std::path::PathBuf> {
    let name = if cfg!(debug_assertions) {
        "events.dev.jsonl"
    } else {
        "events.jsonl"
    };
    Some(logs_dir()?.join(name))
}

pub(crate) fn event_jsonl_paths() -> Vec<PathBuf> {
    let Some(current) = jsonl_path() else {
        return Vec::new();
    };
    vec![current.with_extension("jsonl.1"), current]
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

    let identifier = app_handle.config().identifier.clone();
    let log_dir = dirs::data_dir()
        .map(|root| log_directory_for(&root, &identifier))
        .expect("Could not determine log directory");
    let _ = LOG_DIRECTORY.set(log_dir.clone());
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

    tracing::info!(
        target: "system",
        log_namespace = match identifier.as_str() {
            REFACTOR_TEST_IDENTIFIER => "refactor-test",
            BENCH_IDENTIFIER => "bench",
            _ => "production",
        },
        "telemetry initialized"
    );

    // Leak guard to keep writer alive for app lifetime
    Box::leak(Box::new(pretty_guard));
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

pub fn read_pretty_log_tail(n: usize) -> String {
    let dir = match logs_dir() {
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
    let dir = match logs_dir() {
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
    fn refactor_test_logs_are_isolated_from_production() {
        let root = Path::new("/tmp/application-support");
        assert_eq!(
            log_directory_for(root, "com.localdictation"),
            root.join("local-dictation/logs")
        );
        assert_eq!(
            log_directory_for(root, REFACTOR_TEST_IDENTIFIER),
            root.join("local-dictation-refactor-test/logs")
        );
        assert_eq!(
            log_directory_for(root, BENCH_IDENTIFIER),
            root.join("local-dictation-bench/logs")
        );
    }

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
            "event_code": "transform.pass_outcome",
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

        assert_eq!(data["event_code"], "transform.pass_outcome");
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
            "event_code": sentinel,
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

    #[test]
    fn query_event_sanitizer_rejects_question_answer_context_command_and_paths() {
        let mut data = serde_json::json!({
            "event_code": "query.pass_state",
            "query_pass_id": 8,
            "state": "running",
            "error_code": "provider_not_authenticated",
            "question": "SENTINEL_QUESTION ; rm -rf",
            "answer": "SENTINEL_ANSWER",
            "text": "SENTINEL_PARTIAL",
            "partial": "SENTINEL_PARTIAL",
            "context": "SENTINEL_CONTEXT",
            "window_title": "SENTINEL_WINDOW",
            "selection": "SENTINEL_SELECTION",
            "command": "/Users/private/bin/agent",
            "arguments": ["SENTINEL_ARGUMENTS"],
            "structured_answer": { "content": "SENTINEL_OBJECT" },
            "stderr": "SENTINEL_STDERR secret-token",
            "unknown_numeric": 42
        });
        sanitize_event_data("query", &mut data, true);
        let encoded = serde_json::to_string(&data).unwrap();
        assert_eq!(data["event_code"], "query.pass_state");
        assert_eq!(data["query_pass_id"], 8);
        assert_eq!(data["state"], "running");
        assert_eq!(data["error_code"], "provider_not_authenticated");
        assert!(!encoded.contains("SENTINEL"));
        assert!(!encoded.contains("/Users/private"));
        assert!(!encoded.contains("rm -rf"));
        assert_eq!(data.as_object().unwrap().len(), 4);
    }

    #[test]
    fn query_event_sanitizer_allows_a_null_error_code() {
        let mut data = serde_json::json!({
            "event_code": "query.pass_state",
            "query_pass_id": 9,
            "state": "ready",
            "error_code": null
        });
        sanitize_event_data("query", &mut data, true);
        assert_eq!(data["error_code"], serde_json::Value::Null);
        assert_eq!(data.as_object().unwrap().len(), 4);
    }

    #[test]
    fn query_event_sanitizer_allows_only_declared_numeric_usage_fields() {
        let mut data = serde_json::json!({
            "event_code": "query.pass_state",
            "query_pass_id": 11,
            "state": "ready",
            "error_code": null,
            "input_tokens": 21,
            "output_tokens": 13,
            "reasoning_output_tokens": 5,
            "cached_input_tokens": 8,
            "cache_creation_input_tokens": 2,
            "cost_microusd": 12000,
            "provider": "claude",
            "question_bytes": 99,
            "cost_usd": 0.012,
            "usage": { "input_tokens": 21 },
            "answer": "SENTINEL_ANSWER"
        });
        sanitize_event_data("query", &mut data, true);

        assert_eq!(data["input_tokens"], 21);
        assert_eq!(data["output_tokens"], 13);
        assert_eq!(data["reasoning_output_tokens"], 5);
        assert_eq!(data["cached_input_tokens"], 8);
        assert_eq!(data["cache_creation_input_tokens"], 2);
        assert_eq!(data["cost_microusd"], 12000);
        assert!(data.get("provider").is_none());
        assert!(data.get("question_bytes").is_none());
        assert!(data.get("cost_usd").is_none());
        assert!(data.get("usage").is_none());
        assert!(data.get("answer").is_none());
        assert_eq!(data.as_object().unwrap().len(), 10);
    }

    #[test]
    fn query_event_sanitizer_rejects_wrong_usage_field_types() {
        let mut data = serde_json::json!({
            "event_code": "query.pass_state",
            "query_pass_id": 12,
            "state": "ready",
            "error_code": null,
            "input_tokens": "21",
            "output_tokens": -1,
            "reasoning_output_tokens": 1.5,
            "cached_input_tokens": null,
            "cache_creation_input_tokens": true,
            "cost_microusd": 0.1
        });
        sanitize_event_data("query", &mut data, true);

        assert_eq!(data.as_object().unwrap().len(), 4);
    }

    #[test]
    fn query_event_sanitizer_allows_typed_provider_error_without_detail() {
        let mut data = serde_json::json!({
            "event_code": "query.pass_state",
            "query_pass_id": 10,
            "state": "failed",
            "error_code": "provider_error",
            "provider_detail": "SENTINEL_PROVIDER_CONTENT",
            "usage": { "input_tokens": 12, "output_tokens": 3 }
        });
        sanitize_event_data("query", &mut data, true);
        assert_eq!(data["error_code"], "provider_error");
        assert!(data.get("provider_detail").is_none());
        assert!(data.get("usage").is_none());
    }

    #[test]
    fn query_event_sanitizer_allows_only_declared_environment_and_clipboard_codes() {
        for (state, error_code) in [
            ("failed", "invalid_environment"),
            ("failed", "environment_unavailable"),
            ("failed", "audio_capture_failed"),
            ("ready", "auto_copy_disabled"),
            ("ready", "auto_copy_unavailable"),
            ("ready", "clipboard_superseded"),
        ] {
            let mut data = serde_json::json!({
                "event_code": "query.pass_state",
                "query_pass_id": 13,
                "state": state,
                "error_code": error_code,
            });
            sanitize_event_data("query", &mut data, true);
            assert_eq!(data["error_code"], error_code);
        }

        let mut data = serde_json::json!({
            "event_code": "query.pass_state",
            "query_pass_id": 13,
            "state": "failed",
            "error_code": "environment_failed: /Users/private/SENTINEL",
        });
        sanitize_event_data("query", &mut data, true);
        assert!(data.get("error_code").is_none());
    }

    #[test]
    fn query_event_sanitizer_allows_content_free_partial_ticks() {
        let mut data = serde_json::json!({
            "event_code": "query.partial_tick",
            "query_pass_id": 4,
            "outcome": "empty",
            "sample_count": 16000,
            "text": "SENTINEL_PARTIAL",
            "partial": "SENTINEL_PARTIAL"
        });
        sanitize_event_data("query", &mut data, true);
        let encoded = serde_json::to_string(&data).unwrap();
        assert_eq!(data["event_code"], "query.partial_tick");
        assert_eq!(data["query_pass_id"], 4);
        assert_eq!(data["outcome"], "empty");
        assert_eq!(data["sample_count"], 16000);
        assert!(!encoded.contains("SENTINEL"));
        assert_eq!(data.as_object().unwrap().len(), 4);
    }

    #[test]
    fn release_pipeline_keeps_only_allowlisted_dictation_lifecycle_strings() {
        let mut data = serde_json::json!({
            "recording_id": 9,
            "total_ms": 420,
            "event_code": "pipeline.dictation_terminal",
            "outcome": "runtime_interruption",
            "error_code": "stream_invalidated",
            "model": "PRIVATE_MODEL",
            "error": "/Users/private/project"
        });

        sanitize_event_data("pipeline", &mut data, false);

        assert_eq!(data["recording_id"], 9);
        assert_eq!(data["total_ms"], 420);
        assert_eq!(data["event_code"], "pipeline.dictation_terminal");
        assert_eq!(data["outcome"], "runtime_interruption");
        assert_eq!(data["error_code"], "stream_invalidated");
        assert!(data.get("model").is_none());
        assert!(data.get("error").is_none());

        data["event_code"] = serde_json::Value::String("private.content".to_string());
        data["outcome"] = serde_json::Value::String("private transcript".to_string());
        data["error_code"] = serde_json::Value::String("/Users/private".to_string());
        sanitize_event_data("pipeline", &mut data, false);
        assert!(data.get("event_code").is_none());
        assert!(data.get("outcome").is_none());
        assert!(data.get("error_code").is_none());
    }

    #[test]
    fn performance_store_failure_schema_is_content_free_in_every_build() {
        for debug_build in [true, false] {
            let mut data = serde_json::json!({
                "event_code": "performance.store_operation_failed",
                "operation": "begin",
                "error_class": "busyLocked",
                "attempts": 3,
                "recording_id": 35,
                "error": "database is locked at /Users/private/diagnostics.sqlite3",
                "sql": "INSERT INTO runs VALUES ('SENTINEL_TRANSCRIPT')",
                "path": "/Users/private/diagnostics.sqlite3",
                "content": "SENTINEL_TRANSCRIPT"
            });

            sanitize_event_data("system", &mut data, debug_build);

            assert_eq!(data["event_code"], "performance.store_operation_failed");
            assert_eq!(data["operation"], "begin");
            assert_eq!(data["error_class"], "busyLocked");
            assert_eq!(data["attempts"], 3);
            assert_eq!(data["recording_id"], 35);
            assert_eq!(data.as_object().unwrap().len(), 5);
            let encoded = serde_json::to_string(&data).unwrap();
            assert!(!encoded.contains("Users"));
            assert!(!encoded.contains("INSERT"));
            assert!(!encoded.contains("SENTINEL"));
        }
    }

    #[test]
    fn performance_store_failure_schema_rejects_unknown_strings_and_types() {
        let mut data = serde_json::json!({
            "event_code": "performance.store_operation_failed",
            "operation": "SELECT private_path",
            "error_class": "/Users/private",
            "attempts": "three",
            "recording_id": -1,
        });

        sanitize_event_data("system", &mut data, true);

        assert_eq!(data["event_code"], "performance.store_operation_failed");
        assert_eq!(data.as_object().unwrap().len(), 1);
    }

    #[test]
    fn performance_store_failure_schema_is_exact_even_on_the_wrong_stream() {
        let mut data = serde_json::json!({
            "event_code": "performance.store_operation_failed",
            "operation": "begin",
            "error_class": "busyLocked",
            "attempts": 3,
            "recording_id": 35,
            "error": "database is locked at /Users/private/performance.sqlite3",
            "content": "SENTINEL_TRANSCRIPT",
        });

        sanitize_event_data("pipeline", &mut data, true);

        assert_eq!(data["event_code"], "performance.store_operation_failed");
        assert_eq!(data["operation"], "begin");
        assert_eq!(data["error_class"], "busyLocked");
        assert_eq!(data["attempts"], 3);
        assert_eq!(data["recording_id"], 35);
        assert_eq!(data.as_object().unwrap().len(), 5);
    }

    #[test]
    fn meeting_event_sanitizer_rejects_transcript_and_audio_paths() {
        let mut data = serde_json::json!({
            "generation": 12,
            "event_code": "meeting.capture_failed",
            "phase": "failed",
            "channel": "system",
            "error_code": "capture_failed",
            "transcript": "SENTINEL_PRIVATE_TRANSCRIPT",
            "audio_path": "/Users/private/meeting.wav",
            "session_id": "private-session-id",
            "model": "private-model"
        });
        sanitize_event_data("meeting", &mut data, true);
        let encoded = serde_json::to_string(&data).unwrap();
        assert_eq!(data["generation"], 12);
        assert_eq!(data["event_code"], "meeting.capture_failed");
        assert!(!encoded.contains("SENTINEL"));
        assert!(!encoded.contains("/Users/private"));
        assert!(!encoded.contains("private-session"));
        assert!(!encoded.contains("private-model"));
        let summary = sanitized_summary(
            "meeting",
            Some("SENTINEL_PRIVATE_TRANSCRIPT".to_string()),
            &data,
            true,
        );
        assert_eq!(summary, "Meeting event");
        assert!(!summary.contains("SENTINEL"));
    }

    #[test]
    fn release_summaries_use_only_allowlisted_event_codes() {
        let safe_data = serde_json::json!({
            "event_code": "pipeline.dictation_terminal"
        });
        let safe = sanitized_summary(
            "pipeline",
            Some("SENTINEL_PRIVATE_TRANSCRIPT".to_string()),
            &safe_data,
            false,
        );
        assert_eq!(safe, "pipeline.dictation_terminal");

        let unsafe_data = serde_json::json!({
            "event_code": "SENTINEL_PRIVATE_EVENT"
        });
        let unsafe_summary = sanitized_summary(
            "system",
            Some("/Users/private/project".to_string()),
            &unsafe_data,
            false,
        );
        assert_eq!(unsafe_summary, "Structured event");
        assert!(!unsafe_summary.contains("SENTINEL"));
        assert!(!unsafe_summary.contains("/Users/private"));
    }
}
