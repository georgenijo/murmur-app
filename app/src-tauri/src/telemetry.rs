//! Structured telemetry: tracing subscriber with file + event-emitter layers.

use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use tauri::{Emitter, Manager};

mod key_identity;
pub(crate) use key_identity::sanitize_persisted_line;

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
    jsonl_writer: Mutex<SizeRotatingFile>,
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for TauriEmitterLayer {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        // A log write and window emit on the main thread is main-thread work.
        crate::main_thread_watch::section("telemetry.event", || self.record_event(event));
    }
}

impl TauriEmitterLayer {
    fn record_event(&self, event: &tracing::Event<'_>) {
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

        let (summary, data) = sanitized_event_payload(&stream, event, cfg!(debug_assertions));

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
            if let Ok(mut line) = serde_json::to_string(&app_event) {
                line.push('\n');
                // One write per event: the rotation check runs between
                // events, so a rotated file always ends on a line boundary.
                let _ = writer.write_all(line.as_bytes());
            }
        }

        // Emit to all windows
        let _ = self.app_handle.emit("app-event", &app_event);
    }
}

/// Collect an event's fields and apply the structured-event privacy boundary.
/// Returns the summary and data persisted to JSONL and emitted to windows.
fn sanitized_event_payload(
    stream: &str,
    event: &tracing::Event<'_>,
    debug_build: bool,
) -> (String, serde_json::Value) {
    let mut visitor = JsonVisitor::new();
    event.record(&mut visitor);
    let mut data = serde_json::Value::Object(visitor.fields);
    sanitize_event_data(stream, &mut data, debug_build);
    let summary = sanitized_summary(stream, visitor.message, &data, debug_build);
    (summary, data)
}

/// Enforce the structured-event privacy boundary independently of call-site
/// discipline. Transform traces may retain only stable enum/bucket fields;
/// arbitrary strings (and therefore transform content, paths, app/device
/// identifiers, or raw errors) are discarded in both debug and release builds.
pub(crate) fn canonical_event_code(value: &str) -> Option<&'static str> {
    match value {
        "system.launch_process_start" => Some("system.launch_process_start"),
        "system.launch_native_setup_done" => Some("system.launch_native_setup_done"),
        "system.launch_listener_armed" => Some("system.launch_listener_armed"),
        "system.launch_model_ready" => Some("system.launch_model_ready"),
        "system.launch_frontend_hydrated" => Some("system.launch_frontend_hydrated"),
        "keyboard.listener_started" => Some("keyboard.listener_started"),
        "keyboard.listener_silent" => Some("keyboard.listener_silent"),
        "keyboard.listener_failed" => Some("keyboard.listener_failed"),
        "keyboard.escape_cancel" => Some("keyboard.escape_cancel"),
        "keyboard.hotkey_arming_cancelled" => Some("keyboard.hotkey_arming_cancelled"),
        "keyboard.fn_capture_skipped" => Some("keyboard.fn_capture_skipped"),
        "keyboard.globe_companion_ignored" => Some("keyboard.globe_companion_ignored"),
        "audio.capture_backend_timeout" => Some("audio.capture_backend_timeout"),
        "audio.system_audio_graph_observed" => Some("audio.system_audio_graph_observed"),
        "audio.device_reresolution_started" => Some("audio.device_reresolution_started"),
        "audio.input_resolution_observed" => Some("audio.input_resolution_observed"),
        "audio.capture_started" => Some("audio.capture_started"),
        "audio.fallback_started" => Some("audio.fallback_started"),
        "audio.capture_ready" => Some("audio.capture_ready"),
        "audio.recording_status_timing" => Some("audio.recording_status_timing"),
        "audio.recording_context_ready" => Some("audio.recording_context_ready"),
        "audio.standby_policy_applied" => Some("audio.standby_policy_applied"),
        "audio.standby_entered" => Some("audio.standby_entered"),
        "audio.speculative_capture_refused" => Some("audio.speculative_capture_refused"),
        "audio.speculative_capture_flowing" => Some("audio.speculative_capture_flowing"),
        "audio.speculative_capture_adopted" => Some("audio.speculative_capture_adopted"),
        "audio.speculative_capture_discarded" => Some("audio.speculative_capture_discarded"),
        "audio.speculative_capture_closed" => Some("audio.speculative_capture_closed"),
        "audio.capture_failed" => Some("audio.capture_failed"),
        "audio.lifecycle_failed" => Some("audio.lifecycle_failed"),
        "audio.permission_prompt_changed" => Some("audio.permission_prompt_changed"),
        "pipeline.dictation_requested" => Some("pipeline.dictation_requested"),
        "pipeline.dictation_state_changed" => Some("pipeline.dictation_state_changed"),
        "pipeline.dictation_presentation" => Some("pipeline.dictation_presentation"),
        "pipeline.dictation_stop_handoff" => Some("pipeline.dictation_stop_handoff"),
        "pipeline.dictation_terminal" => Some("pipeline.dictation_terminal"),
        "pipeline.dictation_completed" => Some("pipeline.dictation_completed"),
        "pipeline.dictation_metrics" => Some("pipeline.dictation_metrics"),
        "pipeline.dictation_failed" => Some("pipeline.dictation_failed"),
        "pipeline.dictation_partial_tick" => Some("pipeline.dictation_partial_tick"),
        "pipeline.preview_vad_disagreement" => Some("pipeline.preview_vad_disagreement"),
        "pipeline.dictation_preview_presentation" => {
            Some("pipeline.dictation_preview_presentation")
        }
        "pipeline.delivery_target_verified" => Some("pipeline.delivery_target_verified"),
        "pipeline.smart_destination_delivery" => Some("pipeline.smart_destination_delivery"),
        "pipeline.paste_safety" => Some("pipeline.paste_safety"),
        "pipeline.main_thread_paste_wait" => Some("pipeline.main_thread_paste_wait"),
        "system.main_thread_watch_started" => Some("system.main_thread_watch_started"),
        "system.main_thread_span" => Some("system.main_thread_span"),
        "system.main_thread_paste_span" => Some("system.main_thread_paste_span"),
        "system.main_thread_stack" => Some("system.main_thread_stack"),
        "system.main_thread_rates" => Some("system.main_thread_rates"),
        "pipeline.dictation_stage_gaps" => Some("pipeline.dictation_stage_gaps"),
        "homophone_context.backoff_entered" => Some("homophone_context.backoff_entered"),
        "homophone_context.backoff_exited" => Some("homophone_context.backoff_exited"),
        "homophone_context.prewarm_requested" => Some("homophone_context.prewarm_requested"),
        "homophone_context.pass" => Some("homophone_context.pass"),
        "performance.store_operation_failed" => Some("performance.store_operation_failed"),
        "system.startup_baseline" => Some("system.startup_baseline"),
        "system.model_install_started" => Some("system.model_install_started"),
        "system.model_install_phase" => Some("system.model_install_phase"),
        "system.model_install_terminal" => Some("system.model_install_terminal"),
        "overlay.placement" => Some("overlay.placement"),
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
        "meeting.echo_cancellation_state_changed" => {
            Some("meeting.echo_cancellation_state_changed")
        }
        "meeting.permission_probe_started" => Some("meeting.permission_probe_started"),
        "meeting.permission_probe_finished" => Some("meeting.permission_probe_finished"),
        "meeting.permission_probe_failed" => Some("meeting.permission_probe_failed"),
        "updater.check_current" => Some("updater.check_current"),
        "updater.check_failed" => Some("updater.check_failed"),
        "updater.install_blocked" => Some("updater.install_blocked"),
        "updater.install_ready" => Some("updater.install_ready"),
        "updater.install_failed" => Some("updater.install_failed"),
        "updater.restart_failed" => Some("updater.restart_failed"),
        "updater.update_available" => Some("updater.update_available"),
        "updater.freshness_check_failed" => Some("updater.freshness_check_failed"),
        "updater.download_starting" => Some("updater.download_starting"),
        "updater.download_started" => Some("updater.download_started"),
        "updater.download_finished" => Some("updater.download_finished"),
        "updater.download_ready" => Some("updater.download_ready"),
        _ => None,
    }
}

pub(crate) fn is_safe_updater_version(version: &str) -> bool {
    let version = version.strip_prefix('v').unwrap_or(version);
    version.len() <= 32
        && version.split('.').count() == 3
        && version
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}

fn is_model_install_event(data: &serde_json::Map<String, serde_json::Value>) -> bool {
    matches!(
        data.get("event_code").and_then(serde_json::Value::as_str),
        Some(
            "system.model_install_started"
                | "system.model_install_phase"
                | "system.model_install_terminal"
        )
    )
}

fn is_safe_model_install_field(key: &str, value: &serde_json::Value) -> bool {
    match key {
        "event_code" => value.as_str().is_some_and(|value| {
            matches!(
                value,
                "system.model_install_started"
                    | "system.model_install_phase"
                    | "system.model_install_terminal"
            )
        }),
        "attempt_id" => value.as_u64().is_some_and(|value| value > 0),
        "install_kind" => value
            .as_str()
            .is_some_and(|value| matches!(value, "coreml" | "parakeet" | "whisper")),
        "phase" => value.as_str().is_some_and(|value| {
            matches!(
                value,
                "preparing" | "repairing" | "initializing" | "validating"
            )
        }),
        "outcome_code" => value.as_str().is_some_and(|value| {
            matches!(
                value,
                "success"
                    | "install_failed"
                    | "installer_timeout"
                    | "cancelled"
                    | "cache_repair_failed"
                    | "repair_state_unavailable"
                    | "validation_failed"
                    | "termination_unconfirmed"
                    | "native_initialization_failed"
                    | "cache_unavailable"
                    | "unknown_model"
                    | "spawn_failed"
                    | "protocol_error"
                    | "worker_exited_early"
                    | "worker_task_failed"
                    | "state_publish_failed"
            )
        }),
        "repaired_cache" | "repeated_repair" | "termination_confirmed" => value.is_boolean(),
        _ => false,
    }
}

fn valid_model_install_shape(data: &serde_json::Map<String, serde_json::Value>) -> bool {
    let event_code = data.get("event_code").and_then(serde_json::Value::as_str);
    let base_valid = data
        .get("attempt_id")
        .and_then(serde_json::Value::as_u64)
        .is_some_and(|value| value > 0)
        && data
            .get("install_kind")
            .and_then(serde_json::Value::as_str)
            .is_some();
    if !base_valid {
        return false;
    }
    match event_code {
        Some("system.model_install_started") => data.len() == 3,
        Some("system.model_install_phase") => {
            data.len() == 5
                && data
                    .get("phase")
                    .and_then(serde_json::Value::as_str)
                    .is_some()
                && data
                    .get("repeated_repair")
                    .and_then(serde_json::Value::as_bool)
                    .is_some_and(|repeated| {
                        !repeated
                            || data.get("phase").and_then(serde_json::Value::as_str)
                                == Some("repairing")
                    })
        }
        Some("system.model_install_terminal") => {
            let repaired = data
                .get("repaired_cache")
                .and_then(serde_json::Value::as_bool);
            let repeated = data
                .get("repeated_repair")
                .and_then(serde_json::Value::as_bool);
            let termination = data
                .get("termination_confirmed")
                .and_then(serde_json::Value::as_bool);
            data.len() == 7
                && data
                    .get("outcome_code")
                    .and_then(serde_json::Value::as_str)
                    .is_some()
                && repaired.is_some()
                && repeated.is_some()
                && termination.is_some()
                && !(repeated == Some(true) && repaired != Some(true))
                && (termination != Some(false)
                    || data.get("outcome_code").and_then(serde_json::Value::as_str)
                        == Some("termination_unconfirmed"))
        }
        _ => false,
    }
}

fn sanitize_model_install_event(data: &mut serde_json::Map<String, serde_json::Value>) {
    let has_invalid_known_field = data.iter().any(|(key, value)| {
        matches!(
            key.as_str(),
            "event_code"
                | "attempt_id"
                | "install_kind"
                | "phase"
                | "outcome_code"
                | "repaired_cache"
                | "repeated_repair"
                | "termination_confirmed"
        ) && !is_safe_model_install_field(key, value)
    });
    data.retain(|key, value| is_safe_model_install_field(key, value));
    if has_invalid_known_field || !valid_model_install_shape(data) {
        data.retain(|key, _| key == "event_code");
    }
}

fn is_dictation_partial_tick_event(data: &serde_json::Map<String, serde_json::Value>) -> bool {
    data.get("event_code").and_then(serde_json::Value::as_str)
        == Some("pipeline.dictation_partial_tick")
}

fn sanitize_dictation_partial_tick_event(data: &mut serde_json::Map<String, serde_json::Value>) {
    data.retain(|key, value| match key.as_str() {
        "event_code" => value.as_str() == Some("pipeline.dictation_partial_tick"),
        "recording_id" => value.as_u64().is_some_and(|value| value > 0),
        "sample_count" => value.as_u64().is_some_and(|value| value <= 320_000),
        "outcome" => value.as_str().is_some_and(|value| {
            matches!(
                value,
                "unsupported_model"
                    | "in_flight"
                    | "no_context"
                    | "too_short"
                    | "emitted"
                    | "emit_failed"
                    | "stale"
                    | "empty"
                    | "stopped"
                    | "model_unavailable"
                    | "backend_unavailable"
                    | "decode_error"
                    | "worker_failed"
            )
        }),
        _ => false,
    });
}

fn is_main_thread_watch_event(data: &serde_json::Map<String, serde_json::Value>) -> bool {
    matches!(
        data.get("event_code").and_then(serde_json::Value::as_str),
        Some(
            "pipeline.main_thread_paste_wait"
                | "system.main_thread_watch_started"
                | "system.main_thread_span"
                | "system.main_thread_paste_span"
                | "system.main_thread_stack"
                | "system.main_thread_rates"
        )
    )
}

fn is_dictation_stage_gaps_event(data: &serde_json::Map<String, serde_json::Value>) -> bool {
    data.get("event_code").and_then(serde_json::Value::as_str)
        == Some("pipeline.dictation_stage_gaps")
}

/// Stage-gap timings are numbers of microseconds and one recording id. Keep
/// that exact schema in every build.
fn sanitize_dictation_stage_gaps_event(data: &mut serde_json::Map<String, serde_json::Value>) {
    data.retain(|key, value| match key.as_str() {
        "event_code" => true,
        "recording_id" => value.as_u64().is_some_and(|id| id > 0),
        "total_us"
        | "unattributed_us"
        | "capture_finalization_us"
        | "handoff_us"
        | "pipeline_prologue_us"
        | "vad_us"
        | "vad_blocking_queue_us"
        | "pre_backend_us"
        | "backend_acquire_us"
        | "backend_model_queue_us"
        | "backend_model_load_us"
        | "backend_decode_us"
        | "backend_release_us"
        | "transform_setup_us"
        | "transform_us"
        | "post_transform_us"
        | "delivery_us"
        | "pipeline_return_us"
        | "idle_status_emit_us"
        | "tail_us" => value.as_u64().is_some(),
        _ => false,
    });
}

/// Main-thread diagnostics carry only static labels, ids, monotonic
/// timestamps, and durations. Keep that exact schema in every build.
fn sanitize_main_thread_watch_event(data: &mut serde_json::Map<String, serde_json::Value>) {
    let code = data
        .get("event_code")
        .and_then(serde_json::Value::as_str)
        .and_then(canonical_event_code)
        .unwrap_or("");
    data.retain(|key, value| {
        let number = value.as_u64().is_some();
        let boolean = value.is_boolean();
        let label = value
            .as_str()
            .is_some_and(crate::main_thread_watch::is_safe_label);
        match (code, key.as_str()) {
            (_, "event_code") => true,
            ("system.main_thread_watch_started", key) => match key {
                "epoch_unix_ms" | "log_span_min_us" | "slow_paste_us" | "stack_first_sample_us" => {
                    number
                }
                "observers_installed" | "reporter_started" | "stack_sampling" => boolean,
                _ => false,
            },
            ("system.main_thread_span" | "system.main_thread_paste_span", key) => match key {
                "recording_id" => {
                    code == "system.main_thread_paste_span"
                        && value.as_u64().is_some_and(|id| id > 0)
                }
                "kind" => value.as_str().is_some_and(|kind| {
                    matches!(
                        kind,
                        "job"
                            | "command"
                            | "callback"
                            | "section"
                            | "runloop_busy"
                            | "runloop_segment"
                    )
                }),
                "label" => label,
                // Empty for spans that are not run-loop busy spans.
                "longest_phase" => label || value.as_str() == Some(""),
                "start_us" | "start_unix_ms" | "duration_us" | "queue_us" | "longest_phase_us"
                | "segments" => number,
                "overlap_us" => code == "system.main_thread_paste_span" && number,
                "suppressed" | "dropped" => code == "system.main_thread_span" && number,
                "on_main" => boolean,
                _ => false,
            },
            ("system.main_thread_stack", key) => match key {
                "sample" => matches!(value.as_u64(), Some(1 | 2)),
                "busy_us" | "start_us" | "start_unix_ms" | "suppressed" | "held_us" => number,
                "sampling_stopped" => boolean,
                "frame_count" => value.as_u64().is_some_and(|count| {
                    (1..=crate::main_thread_watch::stack::MAX_FRAMES as u64).contains(&count)
                }),
                "frames" => value
                    .as_str()
                    .is_some_and(crate::main_thread_watch::stack::is_safe_frames),
                _ => false,
            },
            (
                "system.main_thread_rates",
                "window_us"
                | "start_unix_ms"
                | "audio_level_emits"
                | "overlay_audio_level_emits"
                | "ipc_invokes"
                | "ipc_main_invokes"
                | "ipc_main_us",
            ) => number,
            ("pipeline.main_thread_paste_wait", key) => match key {
                "recording_id" => value.as_u64().is_some_and(|id| id > 0),
                "outcome" => matches!(value.as_str(), Some("started" | "not_started")),
                "longest_label" => label,
                "longest_kind" => value.as_str().is_some_and(|kind| {
                    matches!(kind, "none" | "job" | "command" | "callback" | "section")
                }),
                "inject_queue_us"
                | "inject_job_us"
                | "queued_us"
                | "started_us"
                | "queued_unix_ms"
                | "runloop_busy_us"
                | "runloop_idle_us"
                | "labelled_us"
                | "unlabelled_busy_us"
                | "job_us"
                | "command_us"
                | "callback_us"
                | "section_us"
                | "event_loop_drain_us"
                | "longest_us"
                | "span_count"
                | "spans_logged" => number,
                "slow" | "spans_truncated" => boolean,
                "preview_hide" => matches!(
                    value.as_str(),
                    Some("none" | "before_paste" | "during_wait" | "after_paste")
                ),
                _ => false,
            },
            _ => false,
        }
    });
}

fn is_dictation_preview_presentation_event(
    data: &serde_json::Map<String, serde_json::Value>,
) -> bool {
    data.get("event_code").and_then(serde_json::Value::as_str)
        == Some("pipeline.dictation_preview_presentation")
}

fn sanitize_dictation_preview_presentation_event(
    data: &mut serde_json::Map<String, serde_json::Value>,
) {
    data.retain(|key, value| match key.as_str() {
        "event_code" => value.as_str() == Some("pipeline.dictation_preview_presentation"),
        "recording_id" => value.as_u64().is_some_and(|value| value > 0),
        "outcome" => value.as_str().is_some_and(|value| {
            matches!(value, "shown" | "show_failed" | "hidden" | "hide_failed")
        }),
        "trigger" => value
            .as_str()
            .is_some_and(|value| matches!(value, "after_paste" | "no_paste" | "recording_ended")),
        _ => false,
    });
}

fn is_safe_audio_owner_kind(value: &str) -> bool {
    matches!(
        value,
        "dictation" | "transform" | "preview" | "microphone_benchmark" | "corpus"
    )
}

fn is_audio_input_resolution_event(data: &serde_json::Map<String, serde_json::Value>) -> bool {
    data.get("event_code").and_then(serde_json::Value::as_str)
        == Some("audio.input_resolution_observed")
}

fn is_safe_audio_input_resolution_field(key: &str, value: &serde_json::Value) -> bool {
    match key {
        "event_code" => value.as_str() == Some("audio.input_resolution_observed"),
        "capture_id" | "owner" => value.as_u64().is_some_and(|value| value > 0),
        "resolution_pass" => value.as_u64().is_some_and(|value| (1..=3).contains(&value)),
        "backend_attempt" => value.as_u64().is_some_and(|value| (1..=2).contains(&value)),
        "owner_kind" => value.as_str().is_some_and(is_safe_audio_owner_kind),
        "backend" => value
            .as_str()
            .is_some_and(|value| matches!(value, "auhal" | "cpal")),
        "microphone_mode" => value
            .as_str()
            .is_some_and(|value| matches!(value, "pinned" | "system_default")),
        "input_enumeration_ok"
        | "requested_present"
        | "requested_present_known"
        | "input_device_count_capped"
        | "default_input_available" => value.is_boolean(),
        "input_device_count" => value.as_u64().is_some_and(|value| {
            value <= murmur_capture_helper_protocol::MAX_INPUT_DEVICE_COUNT as u64
        }),
        _ => false,
    }
}

fn valid_audio_input_resolution_shape(data: &serde_json::Map<String, serde_json::Value>) -> bool {
    let string = |key: &str| data.get(key).and_then(serde_json::Value::as_str);
    let boolean = |key: &str| data.get(key).and_then(serde_json::Value::as_bool);
    let unsigned = |key: &str| data.get(key).and_then(serde_json::Value::as_u64);

    let Some(mode) = string("microphone_mode") else {
        return false;
    };
    let Some(enumeration_ok) = boolean("input_enumeration_ok") else {
        return false;
    };
    let Some(requested_present) = boolean("requested_present") else {
        return false;
    };
    let Some(requested_present_known) = boolean("requested_present_known") else {
        return false;
    };
    let Some(input_device_count) = unsigned("input_device_count") else {
        return false;
    };
    let Some(input_device_count_capped) = boolean("input_device_count_capped") else {
        return false;
    };
    if boolean("default_input_available").is_none()
        || string("backend").is_none()
        || string("owner_kind").is_none()
        || unsigned("capture_id").is_none()
        || unsigned("owner").is_none()
        || unsigned("resolution_pass").is_none()
        || unsigned("backend_attempt").is_none()
    {
        return false;
    }
    if input_device_count_capped
        && input_device_count != murmur_capture_helper_protocol::MAX_INPUT_DEVICE_COUNT as u64
    {
        return false;
    }
    if !enumeration_ok
        && (input_device_count != 0 || input_device_count_capped || requested_present_known)
    {
        return false;
    }
    match mode {
        "system_default" => !requested_present_known && !requested_present,
        "pinned" => {
            requested_present_known == enumeration_ok
                && (requested_present_known || !requested_present)
                && !(requested_present && input_device_count == 0)
        }
        _ => false,
    }
}

fn sanitize_audio_input_resolution_event(data: &mut serde_json::Map<String, serde_json::Value>) {
    data.retain(|key, value| is_safe_audio_input_resolution_field(key, value));
    if !valid_audio_input_resolution_shape(data) {
        // Keep only the stable event identity rather than persisting a partial
        // or internally contradictory hardware observation.
        data.retain(|key, _| key == "event_code");
    }
}

fn is_audio_graph_observation_event(data: &serde_json::Map<String, serde_json::Value>) -> bool {
    data.get("event_code").and_then(serde_json::Value::as_str)
        == Some("audio.system_audio_graph_observed")
}

fn is_safe_audio_graph_observation_field(key: &str, value: &serde_json::Value) -> bool {
    match key {
        "event_code" => value.as_str() == Some("audio.system_audio_graph_observed"),
        "capture_id" => value.as_u64().is_some_and(|value| value > 0),
        // Bounded by the module's own object-list caps; the same numbers ship
        // from every install, so nothing here may become a string.
        "running_audio_process_count" | "tap_count" | "device_count" | "devices_running_count" => {
            value.as_u64().is_some_and(|value| value <= 4_096)
        }
        "elapsed_ms" => value.as_u64().is_some_and(|value| value <= 60_000),
        "query_timed_out" | "probe_unavailable" => value.is_boolean(),
        _ => false,
    }
}

fn valid_audio_graph_observation_shape(data: &serde_json::Map<String, serde_json::Value>) -> bool {
    let unsigned = |key: &str| data.get(key).and_then(serde_json::Value::as_u64);
    let boolean = |key: &str| data.get(key).and_then(serde_json::Value::as_bool);
    let (Some(timed_out), Some(probe_unavailable)) =
        (boolean("query_timed_out"), boolean("probe_unavailable"))
    else {
        return false;
    };
    let (Some(processes), Some(taps), Some(devices), Some(devices_running)) = (
        unsigned("running_audio_process_count"),
        unsigned("tap_count"),
        unsigned("device_count"),
        unsigned("devices_running_count"),
    ) else {
        return false;
    };
    if unsigned("capture_id").is_none() || unsigned("elapsed_ms").is_none() {
        return false;
    }
    // A probe that never started cannot also have observed a wedge.
    if timed_out && probe_unavailable {
        return false;
    }
    // Neither a timed-out nor an unstarted query observed anything.
    if (timed_out || probe_unavailable)
        && (processes != 0 || taps != 0 || devices != 0 || devices_running != 0)
    {
        return false;
    }
    devices_running <= devices
}

/// This event ships from every install, armed or not. Enforce an exact
/// content-free schema in every build so a future call site cannot append a
/// PID, a device or tap name, or a UID.
fn sanitize_audio_graph_observation_event(data: &mut serde_json::Map<String, serde_json::Value>) {
    data.retain(|key, value| is_safe_audio_graph_observation_field(key, value));
    if !valid_audio_graph_observation_shape(data) {
        data.retain(|key, _| key == "event_code");
    }
}

fn is_audio_device_reresolution_event(data: &serde_json::Map<String, serde_json::Value>) -> bool {
    data.get("event_code").and_then(serde_json::Value::as_str)
        == Some("audio.device_reresolution_started")
}

fn is_safe_audio_device_reresolution_field(key: &str, value: &serde_json::Value) -> bool {
    match key {
        "event_code" => value.as_str() == Some("audio.device_reresolution_started"),
        "owner" => value.as_u64().is_some_and(|value| value > 0),
        "owner_kind" => value.as_str().is_some_and(is_safe_audio_owner_kind),
        "completed_pass" => value.as_u64().is_some_and(|value| (1..=2).contains(&value)),
        "next_pass" => value.as_u64().is_some_and(|value| (2..=3).contains(&value)),
        "retry_delay_ms" => value.as_u64() == Some(500),
        "error_kind" => value.as_str() == Some("device_unavailable"),
        _ => false,
    }
}

fn valid_audio_device_reresolution_shape(
    data: &serde_json::Map<String, serde_json::Value>,
) -> bool {
    let Some(completed_pass) = data
        .get("completed_pass")
        .and_then(serde_json::Value::as_u64)
    else {
        return false;
    };
    let Some(next_pass) = data.get("next_pass").and_then(serde_json::Value::as_u64) else {
        return false;
    };
    data.get("owner")
        .and_then(serde_json::Value::as_u64)
        .is_some()
        && data
            .get("owner_kind")
            .and_then(serde_json::Value::as_str)
            .is_some()
        && data
            .get("retry_delay_ms")
            .and_then(serde_json::Value::as_u64)
            == Some(500)
        && data.get("error_kind").and_then(serde_json::Value::as_str) == Some("device_unavailable")
        && next_pass == completed_pass + 1
}

fn sanitize_audio_device_reresolution_event(data: &mut serde_json::Map<String, serde_json::Value>) {
    data.retain(|key, value| is_safe_audio_device_reresolution_field(key, value));
    if !valid_audio_device_reresolution_shape(data) {
        data.retain(|key, _| key == "event_code");
    }
}

fn is_audio_permission_prompt_event(data: &serde_json::Map<String, serde_json::Value>) -> bool {
    data.get("event_code").and_then(serde_json::Value::as_str)
        == Some("audio.permission_prompt_changed")
}

fn is_safe_audio_permission_prompt_field(key: &str, value: &serde_json::Value) -> bool {
    match key {
        "event_code" => value.as_str() == Some("audio.permission_prompt_changed"),
        "recording_id" | "owner" => value.as_u64().is_some_and(|value| value > 0),
        "owner_kind" => value.as_str() == Some("dictation"),
        "state" => value
            .as_str()
            .is_some_and(|value| matches!(value, "pending" | "resolved")),
        "prompt_pending_ms" => value.as_u64().is_some_and(|value| value <= 300_000),
        _ => false,
    }
}

fn valid_audio_permission_prompt_shape(data: &serde_json::Map<String, serde_json::Value>) -> bool {
    let recording_id = data.get("recording_id").and_then(serde_json::Value::as_u64);
    let owner = data.get("owner").and_then(serde_json::Value::as_u64);
    if recording_id.is_none() || owner != recording_id {
        return false;
    }
    if data.get("owner_kind").and_then(serde_json::Value::as_str) != Some("dictation") {
        return false;
    }
    match data.get("state").and_then(serde_json::Value::as_str) {
        Some("pending") => data.get("prompt_pending_ms").is_none(),
        Some("resolved") => data
            .get("prompt_pending_ms")
            .and_then(serde_json::Value::as_u64)
            .is_some(),
        _ => false,
    }
}

fn sanitize_audio_permission_prompt_event(data: &mut serde_json::Map<String, serde_json::Value>) {
    let has_invalid_known_field = data.iter().any(|(key, value)| {
        matches!(
            key.as_str(),
            "event_code" | "recording_id" | "owner" | "owner_kind" | "state" | "prompt_pending_ms"
        ) && !is_safe_audio_permission_prompt_field(key, value)
    });
    data.retain(|key, value| is_safe_audio_permission_prompt_field(key, value));
    if has_invalid_known_field || !valid_audio_permission_prompt_shape(data) {
        data.retain(|key, _| key == "event_code");
    }
}

fn is_dictation_requested_event(data: &serde_json::Map<String, serde_json::Value>) -> bool {
    data.get("event_code").and_then(serde_json::Value::as_str)
        == Some("pipeline.dictation_requested")
}

fn is_safe_dictation_requested_field(key: &str, value: &serde_json::Value) -> bool {
    match key {
        "event_code" => value.as_str() == Some("pipeline.dictation_requested"),
        "recording_id" => value.as_u64().is_some_and(|value| value > 0),
        "slo_contract" => value.as_u64() == Some(1),
        "origin" => value
            .as_str()
            .is_some_and(|value| matches!(value, "hold" | "toggle")),
        "device_selection" => value
            .as_str()
            .is_some_and(|value| matches!(value, "explicit" | "system_default" | "smart_auto")),
        _ => false,
    }
}

fn valid_dictation_requested_shape(data: &serde_json::Map<String, serde_json::Value>) -> bool {
    data.len() == 5
        && data
            .iter()
            .all(|(key, value)| is_safe_dictation_requested_field(key, value))
}

fn sanitize_dictation_requested_event(data: &mut serde_json::Map<String, serde_json::Value>) {
    let has_invalid_known_field = data.iter().any(|(key, value)| {
        matches!(
            key.as_str(),
            "event_code" | "recording_id" | "slo_contract" | "origin" | "device_selection"
        ) && !is_safe_dictation_requested_field(key, value)
    });
    data.retain(|key, value| is_safe_dictation_requested_field(key, value));
    if has_invalid_known_field || !valid_dictation_requested_shape(data) {
        data.retain(|key, _| key == "event_code");
    }
}

fn is_dictation_slo_event(data: &serde_json::Map<String, serde_json::Value>) -> bool {
    matches!(
        data.get("event_code").and_then(serde_json::Value::as_str),
        Some("pipeline.dictation_state_changed" | "pipeline.dictation_presentation")
    )
}

fn is_safe_dictation_state(value: &str) -> bool {
    matches!(
        value,
        "idle" | "starting" | "recording" | "recovering" | "processing"
    )
}

fn valid_dictation_presentation_pair(status_code: &str, action_code: &str) -> bool {
    matches!(
        (status_code, action_code),
        ("microphone_cleanup_in_progress", "wait")
            | ("microphone_initialization_failed", "retry")
            | (
                "microphone_initialization_failed",
                "open_microphone_settings"
            )
            | ("microphone_initialization_failed", "choose_microphone")
            | ("microphone_cleanup_stalled", "restart_app")
            | ("microphone_interrupted", "retry")
            | ("microphone_interrupted", "wait_for_partial_transcription")
    )
}

fn is_safe_dictation_slo_field(event_code: &str, key: &str, value: &serde_json::Value) -> bool {
    match key {
        "event_code" => value.as_str() == Some(event_code),
        "recording_id" => value.as_u64().is_some_and(|value| value > 0),
        "from" | "to" if event_code == "pipeline.dictation_state_changed" => {
            value.as_str().is_some_and(is_safe_dictation_state)
        }
        "status_code" if event_code == "pipeline.dictation_presentation" => {
            value.as_str().is_some_and(|value| {
                matches!(
                    value,
                    "microphone_cleanup_in_progress"
                        | "microphone_initialization_failed"
                        | "microphone_cleanup_stalled"
                        | "microphone_interrupted"
                )
            })
        }
        "action_code" if event_code == "pipeline.dictation_presentation" => {
            value.as_str().is_some_and(|value| {
                matches!(
                    value,
                    "wait"
                        | "retry"
                        | "open_microphone_settings"
                        | "choose_microphone"
                        | "restart_app"
                        | "wait_for_partial_transcription"
                )
            })
        }
        _ => false,
    }
}

fn valid_dictation_slo_shape(data: &serde_json::Map<String, serde_json::Value>) -> bool {
    if data
        .get("recording_id")
        .and_then(serde_json::Value::as_u64)
        .is_none_or(|value| value == 0)
    {
        return false;
    }
    match data.get("event_code").and_then(serde_json::Value::as_str) {
        Some("pipeline.dictation_state_changed") => {
            let from = data.get("from").and_then(serde_json::Value::as_str);
            let to = data.get("to").and_then(serde_json::Value::as_str);
            from.is_some() && to.is_some() && from != to
        }
        Some("pipeline.dictation_presentation") => {
            let status_code = data.get("status_code").and_then(serde_json::Value::as_str);
            let action_code = data.get("action_code").and_then(serde_json::Value::as_str);
            status_code
                .zip(action_code)
                .is_some_and(|(status, action)| valid_dictation_presentation_pair(status, action))
        }
        _ => false,
    }
}

fn sanitize_dictation_slo_event(data: &mut serde_json::Map<String, serde_json::Value>) {
    let Some(event_code) = data
        .get("event_code")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
    else {
        return;
    };
    data.retain(|key, value| is_safe_dictation_slo_field(&event_code, key, value));
    if !valid_dictation_slo_shape(data) {
        data.retain(|key, _| key == "event_code");
    }
}

fn is_dictation_terminal_event(data: &serde_json::Map<String, serde_json::Value>) -> bool {
    data.get("event_code").and_then(serde_json::Value::as_str)
        == Some("pipeline.dictation_terminal")
}

fn is_safe_dictation_terminal_field(key: &str, value: &serde_json::Value) -> bool {
    match key {
        "event_code" => value.as_str() == Some("pipeline.dictation_terminal"),
        "recording_id" => value.as_u64().is_some_and(|value| value > 0),
        "outcome" => value.as_str().is_some_and(is_safe_dictation_outcome),
        "error_code" => value.as_str().is_some_and(is_safe_dictation_error_code),
        "char_count" => value.as_u64().is_some(),
        _ => false,
    }
}

fn sanitize_dictation_terminal_event(data: &mut serde_json::Map<String, serde_json::Value>) {
    let has_invalid_known_field = data.iter().any(|(key, value)| {
        matches!(
            key.as_str(),
            "event_code" | "recording_id" | "outcome" | "error_code" | "char_count"
        ) && !is_safe_dictation_terminal_field(key, value)
    });
    data.retain(|key, value| is_safe_dictation_terminal_field(key, value));
    if has_invalid_known_field || data.len() != 5 {
        data.retain(|key, _| key == "event_code");
    }
}

fn sanitize_paste_safety_event(data: &mut serde_json::Map<String, serde_json::Value>) {
    let phase = data.get("phase_code").and_then(serde_json::Value::as_str);
    let outcome = data.get("outcome_code").and_then(serde_json::Value::as_str);
    let valid = matches!(
        (phase, outcome),
        (
            Some("preparation"),
            Some(
                "ready"
                    | "activation_requested"
                    | "refused"
                    | "refused_clipboard_only"
                    | "timed_out"
                    | "reply_lost"
            )
        ) | (
            Some("authority"),
            Some("cancelled" | "expired" | "already_committed" | "operation_in_flight")
        ) | (Some("post"), Some("key_posted"))
            | (
                Some("confirmation"),
                Some("structurally_confirmed" | "unconfirmed" | "not_observed")
            )
            | (Some("rollback"), Some("restored" | "refused"))
            | (
                Some("focus_proof"),
                Some(
                    "live_match"
                        | "live_mismatch"
                        | "identity_mismatch"
                        | "frontmost_fallback_match"
                        | "frontmost_fallback_mismatch"
                )
            )
    );
    let focus_fields_valid = phase != Some("focus_proof")
        || (matches!(
            data.get("live_code").and_then(serde_json::Value::as_str),
            Some(
                "answered"
                    | "not_attempted"
                    | "setup_failed"
                    | "ax_error"
                    | "wrong_type"
                    | "pid_unavailable"
                    | "slow"
            )
        ) && data
            .get("ax_status")
            .and_then(serde_json::Value::as_i64)
            .is_some_and(|status| (-30_000..=0).contains(&status))
            && data
                .get("live_elapsed_us")
                .and_then(serde_json::Value::as_u64)
                .is_some_and(|elapsed| elapsed <= 10_000_000));
    let route_valid = match data.get("route_code") {
        None => phase != Some("preparation"),
        Some(value) => {
            phase == Some("preparation")
                && value.as_str().is_some_and(|route| {
                    matches!(
                        route,
                        "disabled" | "clipboard_only" | "normal_paste" | "smart_target"
                    )
                })
        }
    };
    let focus_proof = phase == Some("focus_proof");
    data.retain(|key, _| {
        key == "event_code"
            || (valid
                && route_valid
                && focus_fields_valid
                && (matches!(key.as_str(), "phase_code" | "outcome_code" | "route_code")
                    || (focus_proof
                        && matches!(key.as_str(), "live_code" | "ax_status" | "live_elapsed_us"))))
    });
}

fn is_smart_destination_delivery_event(data: &serde_json::Map<String, serde_json::Value>) -> bool {
    data.get("event_code").and_then(serde_json::Value::as_str)
        == Some("pipeline.smart_destination_delivery")
}

fn is_safe_smart_destination_delivery_field(key: &str, value: &serde_json::Value) -> bool {
    let route = |value: &serde_json::Value| {
        value.as_str().is_some_and(|value| {
            matches!(
                value,
                "disabled" | "clipboard_only" | "normal_paste" | "smart_target"
            )
        })
    };
    match key {
        "event_code" => value.as_str() == Some("pipeline.smart_destination_delivery"),
        "recording_id" => value.as_u64().is_some_and(|value| value > 0),
        "accepted_route" | "delivery_route" => route(value),
        "outcome_code" => value.as_str().is_some_and(|value| {
            matches!(
                value,
                "no_text"
                    | "auto_pasted"
                    | "clipboard_only_auto_paste_disabled"
                    | "clipboard_only_accessibility_denied"
                    | "clipboard_only_target_changed"
                    | "clipboard_only_focus_not_editable"
                    | "clipboard_only_clipboard_changed"
                    | "clipboard_only_paste_failed"
                    | "clipboard_only_smart_target_unavailable"
                    | "clipboard_write_failed"
                    | "unconfirmed"
                    | "cancelled"
                    | "expired"
                    | "already_committed"
                    | "operation_in_flight"
            )
        }),
        _ => false,
    }
}

/// Keep only the exact bounded schema in every build; any invalid known field
/// collapses the record to its event code.
fn sanitize_smart_destination_delivery_event(
    data: &mut serde_json::Map<String, serde_json::Value>,
) {
    const REQUIRED_FIELDS: [&str; 5] = [
        "event_code",
        "recording_id",
        "accepted_route",
        "delivery_route",
        "outcome_code",
    ];
    let valid = REQUIRED_FIELDS.iter().all(|key| {
        data.get(*key)
            .is_some_and(|value| is_safe_smart_destination_delivery_field(key, value))
    });
    data.retain(|key, value| is_safe_smart_destination_delivery_field(key, value));
    if !valid {
        data.retain(|key, _| key == "event_code");
    }
}

fn is_delivery_target_verification_event(
    data: &serde_json::Map<String, serde_json::Value>,
) -> bool {
    data.get("event_code").and_then(serde_json::Value::as_str)
        == Some("pipeline.delivery_target_verified")
}

fn is_safe_delivery_target_verification_field(key: &str, value: &serde_json::Value) -> bool {
    match key {
        "event_code" => value.as_str() == Some("pipeline.delivery_target_verified"),
        "recording_id" => value.as_u64().is_some_and(|value| value > 0),
        "outcome_code" => value.as_str().is_some_and(|value| {
            matches!(
                value,
                "verified"
                    | "different_application"
                    | "different_process"
                    | "process_relaunched"
                    | "partial_identity_mismatch"
                    | "lookup_unavailable"
                    | "start_identity_incomplete"
                    | "start_target_is_self"
                    | "stale_owner"
            )
        }),
        "anchor_code" => value
            .as_str()
            .is_some_and(|value| matches!(value, "stop" | "start")),
        "stop_sample_code" => value.as_str().is_some_and(|value| {
            matches!(
                value,
                "complete" | "self" | "incomplete" | "mismatch" | "absent"
            )
        }),
        "source_code" => value
            .as_str()
            .is_some_and(|value| matches!(value, "native" | "none")),
        "retry_count" => value.as_u64().is_some_and(|value| value <= 2),
        "elapsed_ms" => value.as_u64().is_some_and(|value| value <= 1_000),
        "same_application"
        | "same_process"
        | "same_process_instance"
        | "activation_changed"
        | "space_changed"
        | "current_is_self"
        | "ownership_current" => value.is_boolean(),
        "window_relation_code" => value
            .as_str()
            .is_some_and(|value| matches!(value, "unknown" | "same" | "different")),
        _ => false,
    }
}

fn valid_delivery_target_verification_shape(
    data: &serde_json::Map<String, serde_json::Value>,
) -> bool {
    const REQUIRED_FIELDS: [&str; 16] = [
        "event_code",
        "recording_id",
        "anchor_code",
        "stop_sample_code",
        "outcome_code",
        "source_code",
        "retry_count",
        "elapsed_ms",
        "same_application",
        "same_process",
        "same_process_instance",
        "window_relation_code",
        "activation_changed",
        "space_changed",
        "current_is_self",
        "ownership_current",
    ];

    data.len() == REQUIRED_FIELDS.len()
        && REQUIRED_FIELDS.iter().all(|key| {
            data.get(*key)
                .is_some_and(|value| is_safe_delivery_target_verification_field(key, value))
        })
        && valid_delivery_target_verification_evidence(data)
}

fn valid_delivery_target_verification_evidence(
    data: &serde_json::Map<String, serde_json::Value>,
) -> bool {
    let string = |key: &str| data.get(key).and_then(serde_json::Value::as_str);
    let boolean = |key: &str| data.get(key).and_then(serde_json::Value::as_bool);
    let unsigned = |key: &str| data.get(key).and_then(serde_json::Value::as_u64);

    let Some(outcome) = string("outcome_code") else {
        return false;
    };
    let Some(source) = string("source_code") else {
        return false;
    };
    let Some(retry_count) = unsigned("retry_count") else {
        return false;
    };
    let Some(same_application) = boolean("same_application") else {
        return false;
    };
    let Some(same_process) = boolean("same_process") else {
        return false;
    };
    let Some(same_process_instance) = boolean("same_process_instance") else {
        return false;
    };
    let Some(activation_changed) = boolean("activation_changed") else {
        return false;
    };
    let Some(space_changed) = boolean("space_changed") else {
        return false;
    };
    let Some(current_is_self) = boolean("current_is_self") else {
        return false;
    };
    let Some(ownership_current) = boolean("ownership_current") else {
        return false;
    };
    // Only a complete stop sample may anchor delivery at stop.
    if (string("anchor_code") == Some("stop")) != (string("stop_sample_code") == Some("complete")) {
        return false;
    }
    let window_relation = string("window_relation_code");
    let no_identity_comparison = !same_application
        && !same_process
        && !same_process_instance
        && window_relation == Some("unknown")
        && !activation_changed
        && !space_changed;

    match outcome {
        "verified" => {
            source == "native"
                && ownership_current
                && same_application
                && same_process
                && same_process_instance
                && !current_is_self
        }
        "different_application" => {
            source == "native" && ownership_current && !same_application && !same_process_instance
        }
        "different_process" => {
            source == "native"
                && ownership_current
                && same_application
                && !same_process
                && !same_process_instance
        }
        "process_relaunched" => {
            source == "native"
                && ownership_current
                && same_application
                && same_process
                && !same_process_instance
        }
        "partial_identity_mismatch" => {
            source == "native"
                && ownership_current
                && !same_application
                && !same_process_instance
                && window_relation == Some("unknown")
                && !current_is_self
        }
        "lookup_unavailable" => {
            source == "none"
                && ownership_current
                && !current_is_self
                && no_identity_comparison
                && retry_count == 2
        }
        "start_identity_incomplete" => {
            source == "none"
                && ownership_current
                && !current_is_self
                && no_identity_comparison
                && retry_count == 0
        }
        "start_target_is_self" => {
            source == "native"
                && ownership_current
                && current_is_self
                && no_identity_comparison
                && retry_count == 0
        }
        "stale_owner" => {
            source == "none"
                && !ownership_current
                && !current_is_self
                && no_identity_comparison
                && retry_count == 0
        }
        _ => false,
    }
}

fn sanitize_delivery_target_verification_event(
    data: &mut serde_json::Map<String, serde_json::Value>,
) {
    let has_invalid_known_field = data.iter().any(|(key, value)| {
        matches!(
            key.as_str(),
            "event_code"
                | "recording_id"
                | "anchor_code"
                | "stop_sample_code"
                | "outcome_code"
                | "source_code"
                | "retry_count"
                | "elapsed_ms"
                | "same_application"
                | "same_process"
                | "same_process_instance"
                | "window_relation_code"
                | "activation_changed"
                | "space_changed"
                | "current_is_self"
                | "ownership_current"
        ) && !is_safe_delivery_target_verification_field(key, value)
    });
    data.retain(|key, value| is_safe_delivery_target_verification_field(key, value));
    if has_invalid_known_field || !valid_delivery_target_verification_shape(data) {
        data.retain(|key, _| key == "event_code");
    }
}

fn is_homophone_context_event(data: &serde_json::Map<String, serde_json::Value>) -> bool {
    matches!(
        data.get("event_code").and_then(serde_json::Value::as_str),
        Some(
            "homophone_context.backoff_entered"
                | "homophone_context.backoff_exited"
                | "homophone_context.prewarm_requested"
                | "homophone_context.pass"
        )
    )
}

fn sanitize_homophone_context_event(data: &mut serde_json::Map<String, serde_json::Value>) {
    data.retain(|key, value| match key.as_str() {
        "event_code" => value.as_str().is_some_and(|code| {
            matches!(
                code,
                "homophone_context.backoff_entered"
                    | "homophone_context.backoff_exited"
                    | "homophone_context.prewarm_requested"
                    | "homophone_context.pass"
            )
        }),
        "session_id"
        | "consecutive_failures"
        | "cooldown_ms"
        | "flagged"
        | "engine_us"
        | "budget_ms" => value.as_u64().is_some(),
        "scheduled" => value.is_boolean(),
        "outcome" => value.as_str().is_some_and(|outcome| {
            matches!(
                outcome,
                "not_triggered"
                    | "bounded_skip"
                    | "unchanged"
                    | "accepted"
                    | "rejected"
                    | "timeout"
                    | "unavailable"
                    | "failed"
                    | "backoff"
            )
        }),
        _ => false,
    });
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

fn is_launch_event_code(code: &str) -> bool {
    matches!(
        code,
        "system.launch_process_start"
            | "system.launch_native_setup_done"
            | "system.launch_listener_armed"
            | "system.launch_model_ready"
            | "system.launch_frontend_hydrated"
    )
}

fn sanitized_summary(
    stream: &str,
    summary: Option<String>,
    data: &serde_json::Value,
    debug_build: bool,
) -> String {
    if data
        .get("event_code")
        .and_then(serde_json::Value::as_str)
        .is_some_and(is_launch_event_code)
    {
        "Launch milestone".to_string()
    } else if data.get("event_code").and_then(serde_json::Value::as_str)
        == Some("audio.input_resolution_observed")
    {
        // Keep this privacy-sensitive hardware observation constant even in a
        // debug build; useful semantics live only in its strict typed fields.
        "Microphone input resolution observed".to_string()
    } else if data.get("event_code").and_then(serde_json::Value::as_str)
        == Some("audio.device_reresolution_started")
    {
        "Microphone device re-resolution started".to_string()
    } else if data.get("event_code").and_then(serde_json::Value::as_str)
        == Some("audio.permission_prompt_changed")
    {
        "Microphone permission prompt state changed".to_string()
    } else if data.get("event_code").and_then(serde_json::Value::as_str)
        == Some("audio.system_audio_graph_observed")
    {
        // Constant in every build: the interesting semantics are the numeric
        // counts, and the surrounding graph names processes and devices.
        "System audio graph observed".to_string()
    } else if matches!(
        data.get("event_code").and_then(serde_json::Value::as_str),
        Some(
            "pipeline.dictation_requested"
                | "pipeline.dictation_state_changed"
                | "pipeline.dictation_presentation"
        )
    ) {
        "Dictation reliability event".to_string()
    } else if matches!(
        data.get("event_code").and_then(serde_json::Value::as_str),
        Some("pipeline.preview_vad_disagreement")
    ) {
        "Preview and final VAD disagreement".to_string()
    } else if data.get("event_code").and_then(serde_json::Value::as_str)
        == Some("pipeline.delivery_target_verified")
    {
        "Dictation delivery target verification".to_string()
    } else if data
        .as_object()
        .is_some_and(|map| is_main_thread_watch_event(map) || is_dictation_stage_gaps_event(map))
    {
        // Constant in every build; the useful facts are the typed fields.
        data["event_code"]
            .as_str()
            .and_then(canonical_event_code)
            .unwrap_or("Structured event")
            .to_string()
    } else if data.get("event_code").and_then(serde_json::Value::as_str)
        == Some("pipeline.paste_safety")
    {
        "Paste safety decision".to_string()
    } else if data.get("event_code").and_then(serde_json::Value::as_str)
        == Some("pipeline.smart_destination_delivery")
    {
        "Dictation delivery route".to_string()
    } else if matches!(
        data.get("event_code").and_then(serde_json::Value::as_str),
        Some(
            "system.model_install_started"
                | "system.model_install_phase"
                | "system.model_install_terminal"
        )
    ) {
        "Model installation event".to_string()
    } else if stream == "meeting" {
        // Event codes carry the useful lifecycle meaning. Keep the JSONL/UI
        // summary constant so formatted content cannot leak from a call site.
        "Meeting event".to_string()
    } else if data.as_object().is_some_and(is_homophone_context_event) {
        data["event_code"]
            .as_str()
            .unwrap_or("Structured event")
            .to_string()
    } else if !debug_build {
        // Release summaries must never depend on caller-provided text. Retain
        // useful semantics only through the exact event-code allowlist.
        data.get("event_code")
            .and_then(serde_json::Value::as_str)
            .and_then(canonical_event_code)
            .unwrap_or("Structured event")
            .to_string()
    } else {
        key_identity::bounded_summary(stream, summary.unwrap_or_default())
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

pub(crate) fn strip_private_meeting_metadata(data: &mut serde_json::Value) -> bool {
    match data {
        serde_json::Value::Object(object) => {
            let mut changed = false;
            for key in [
                "title",
                "title_source",
                "titleSource",
                "attendees",
                "calendar_event",
                "calendarEvent",
                "calendar_events",
                "calendarEvents",
                "calendar_title",
                "calendarTitle",
                "calendar_attendees",
                "calendarAttendees",
                "event_identifier",
                "eventIdentifier",
                "calendar_identifier",
                "calendarIdentifier",
                "selection_token",
                "selectionToken",
                "calendar_notes",
                "calendarNotes",
                "calendar_location",
                "calendarLocation",
                "calendar_url",
                "calendarUrl",
                "meeting_suggestion",
                "meetingSuggestion",
                "suggestion_token",
                "suggestionToken",
                "occurrence_key",
                "occurrenceKey",
                "token",
            ] {
                changed |= object.remove(key).is_some();
            }
            for value in object.values_mut() {
                changed |= strip_private_meeting_metadata(value);
            }
            changed
        }
        serde_json::Value::Array(items) => {
            let mut changed = false;
            for item in items {
                changed |= strip_private_meeting_metadata(item);
            }
            changed
        }
        _ => false,
    }
}

fn sanitize_event_data(stream: &str, data: &mut serde_json::Value, debug_build: bool) {
    strip_private_meeting_metadata(data);
    let Some(obj) = data.as_object_mut() else {
        return;
    };
    if obj
        .get("event_code")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|code| code.starts_with("updater.") && canonical_event_code(code).is_some())
    {
        obj.retain(|key, value| match key.as_str() {
            "event_code" => value.as_str().and_then(canonical_event_code).is_some(),
            "version" => value.as_str().is_some_and(is_safe_updater_version),
            "selection" => value
                .as_str()
                .is_some_and(|s| matches!(s, "retained" | "fresh_check" | "none")),
            _ => false,
        });
        return;
    }
    if obj
        .get("event_code")
        .and_then(serde_json::Value::as_str)
        .is_some_and(is_launch_event_code)
    {
        obj.retain(|key, value| match key.as_str() {
            "event_code" => value.as_str().and_then(canonical_event_code).is_some(),
            "launch_elapsed_ms" => value.as_u64().is_some(),
            _ => false,
        });
        return;
    }
    if key_identity::sanitize_fields(stream, obj) {
        return;
    }
    if is_model_install_event(obj) {
        // Model installation failures may originate in native libraries and
        // contain local paths or raw error text. Keep only the exact stable
        // lifecycle schema before the same JSONL is eligible for Fleet upload.
        sanitize_model_install_event(obj);
        return;
    }
    if is_audio_input_resolution_event(obj) {
        // This attempt-scoped hardware observation is shipped from the same
        // JSONL as ordinary audio logs. Enforce an exact schema in every build
        // so a future call site cannot append device identity or raw errors.
        sanitize_audio_input_resolution_event(obj);
        return;
    }
    if is_audio_device_reresolution_event(obj) {
        sanitize_audio_device_reresolution_event(obj);
        return;
    }
    if is_audio_graph_observation_event(obj) {
        sanitize_audio_graph_observation_event(obj);
        return;
    }
    if is_audio_permission_prompt_event(obj) {
        sanitize_audio_permission_prompt_event(obj);
        return;
    }
    if is_dictation_requested_event(obj) {
        sanitize_dictation_requested_event(obj);
        return;
    }
    if is_dictation_slo_event(obj) {
        sanitize_dictation_slo_event(obj);
        return;
    }
    let disagreement_owner = match obj.get("event_code").and_then(serde_json::Value::as_str) {
        Some("pipeline.preview_vad_disagreement") => Some("recording_id"),
        _ => None,
    };
    if let Some(owner_field) = disagreement_owner {
        obj.retain(|key, value| match key.as_str() {
            "event_code" => true,
            "sample_count" => value.as_u64().is_some(),
            key if key == owner_field => value.as_u64().is_some_and(|id| id > 0),
            _ => false,
        });
        return;
    }
    if is_dictation_partial_tick_event(obj) {
        sanitize_dictation_partial_tick_event(obj);
        return;
    }
    if is_dictation_preview_presentation_event(obj) {
        sanitize_dictation_preview_presentation_event(obj);
        return;
    }
    if is_main_thread_watch_event(obj) {
        sanitize_main_thread_watch_event(obj);
        return;
    }
    if is_dictation_stage_gaps_event(obj) {
        sanitize_dictation_stage_gaps_event(obj);
        return;
    }
    if is_dictation_terminal_event(obj) {
        // Terminal lifecycle records share the event file with Fleet uploads.
        // Keep the exact content-free schema in debug and release builds.
        sanitize_dictation_terminal_event(obj);
        return;
    }
    if obj.get("event_code").and_then(serde_json::Value::as_str) == Some("pipeline.paste_safety") {
        sanitize_paste_safety_event(obj);
        return;
    }
    if is_smart_destination_delivery_event(obj) {
        sanitize_smart_destination_delivery_event(obj);
        return;
    }
    if is_delivery_target_verification_event(obj) {
        // Delivery identity is privacy-sensitive even in debug builds. Keep
        // only bounded equality facts and stable outcome/source codes so app,
        // process, window, path, content, and native error detail cannot leak.
        sanitize_delivery_target_verification_event(obj);
        return;
    }
    if is_homophone_context_event(obj) {
        sanitize_homophone_context_event(obj);
        return;
    }
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
                    "pass_kind" => matches!(value, "preview" | "final"),
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
                "from" | "to" => matches!(
                    value,
                    "off" | "starting" | "active" | "recovering" | "bypassed"
                ),
                "reason" => matches!(
                    value,
                    "none"
                        | "initialization_failed"
                        | "unsupported_format"
                        | "render_discontinuity"
                        | "processor_failed"
                        | "processing_backlog"
                ),
                "permission" => {
                    matches!(value, "unknown" | "granted" | "denied" | "unsupported")
                }
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

/// `events.jsonl` rotates at runtime once it reaches this size.
pub(crate) const EVENT_LOG_MAX_BYTES: u64 = 5 * 1024 * 1024;
/// Rotated generations kept for `events.jsonl` (`.1` newest, `.2` oldest). Two
/// generations let the shipper recover from a second rotation that lands
/// before it has drained the first.
pub(crate) const EVENT_LOG_GENERATIONS: u8 = 2;
/// The human-readable `app.log` is capped separately; it is never shipped.
const APP_LOG_MAX_BYTES: u64 = 10 * 1024 * 1024;
const APP_LOG_GENERATIONS: u8 = 1;
/// After a failed rotation, wait for this much more data before retrying so a
/// persistent rename failure does not cost two renames per event.
const ROTATION_RETRY_BYTES: u64 = 1024 * 1024;
/// While the current file cannot be created, try at most this often.
const CREATE_RETRY_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

/// `path` with `.N` appended (`events.jsonl` → `events.jsonl.1`).
pub(crate) fn generation_path(path: &Path, generation: u8) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(format!(".{generation}"));
    PathBuf::from(name)
}

/// Rotated generations of `path`, newest first.
pub(crate) fn rotated_paths(path: &Path, generations: u8) -> Vec<PathBuf> {
    (1..=generations)
        .map(|generation| generation_path(path, generation))
        .collect()
}

pub(crate) fn event_jsonl_paths() -> Vec<PathBuf> {
    let Some(current) = jsonl_path() else {
        return Vec::new();
    };
    // Oldest first, so replay sees events in the order they were written.
    let mut paths: Vec<PathBuf> = rotated_paths(&current, EVENT_LOG_GENERATIONS)
        .into_iter()
        .rev()
        .collect();
    paths.push(current);
    paths
}

/// File operations the rotating writer depends on, so tests can inject
/// failures between the steps of a rotation.
pub(crate) trait RotationFs: Send + Sync {
    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()>;
    fn create(&self, path: &Path) -> std::io::Result<std::fs::File>;
}

struct RealFs;

impl RotationFs for RealFs {
    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
        std::fs::rename(from, to)
    }

    fn create(&self, path: &Path) -> std::io::Result<std::fs::File> {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
    }
}

#[cfg(test)]
/// Fails chosen file operations, to cover the gap between the steps of a
/// rotation.
#[derive(Default)]
pub(crate) struct FailingFs {
    pub(crate) fail_rename: std::sync::atomic::AtomicBool,
    pub(crate) fail_create: std::sync::atomic::AtomicBool,
    pub(crate) create_calls: std::sync::atomic::AtomicUsize,
}

#[cfg(test)]
impl RotationFs for FailingFs {
    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
        if self.fail_rename.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(std::io::Error::other("injected rename failure"));
        }
        RealFs.rename(from, to)
    }

    fn create(&self, path: &Path) -> std::io::Result<std::fs::File> {
        self.create_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if self.fail_create.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(std::io::Error::other("injected create failure"));
        }
        RealFs.create(path)
    }
}

/// Shift `path` into `.1`, `.1` into `.2`, … keeping `generations` rotated
/// files. The oldest generation is overwritten. On error `path` itself has not
/// been renamed, so the caller may keep appending to it. A missing `path` (the
/// file was deleted underneath the writer) is not an error.
fn rotate_generations(fs: &dyn RotationFs, path: &Path, generations: u8) -> std::io::Result<()> {
    for generation in (1..generations).rev() {
        let from = generation_path(path, generation);
        if from.exists() {
            fs.rename(&from, &generation_path(path, generation + 1))?;
        }
    }
    match fs.rename(path, &generation_path(path, 1)) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

struct RotatingState {
    fs: Arc<dyn RotationFs>,
    path: PathBuf,
    max_bytes: u64,
    generations: u8,
    /// `None` between a successful rotation and a successful re-create. While
    /// it is `None` nothing is written, so a rotated generation is never
    /// appended to after the shipper may have finished reading it.
    file: Option<std::fs::File>,
    size: u64,
    rotate_at: u64,
    /// Events dropped since the file was last writable.
    dropped: u64,
    /// Earliest time the next create attempt may run after a failed one.
    retry_after: Option<std::time::Instant>,
    create_retry: std::time::Duration,
    /// Formats the one line written after recovery, given the dropped count.
    drop_note: Option<fn(u64) -> String>,
}

impl RotatingState {
    fn rotate(&mut self) {
        match rotate_generations(self.fs.as_ref(), &self.path, self.generations) {
            // The old handle now points at `.1` (or an unlinked file). Drop it;
            // the write that follows creates the replacement.
            Ok(()) => self.file = None,
            // The current file is intact: keep appending to it rather than
            // dropping events, and retry after more data has accumulated.
            Err(_) => {
                self.rotate_at = self.size.saturating_add(ROTATION_RETRY_BYTES);
            }
        }
    }

    /// Discard the handle and start a fresh file. Used after the files were
    /// deleted by Clear logs.
    fn reopen(&mut self) {
        self.file = None;
        self.size = 0;
        self.rotate_at = self.max_bytes;
        self.retry_after = None;
        self.file = self.fs.create(&self.path).ok();
    }

    /// Create the current file after a rotation (or a failed create). At most
    /// one attempt per `create_retry`; writes in between are dropped and
    /// counted rather than buffered. After recovery one note line records how
    /// many were dropped; nothing is written to stderr under the lock.
    fn recover_file(&mut self) -> std::io::Result<()> {
        if self
            .retry_after
            .is_some_and(|until| std::time::Instant::now() < until)
        {
            self.dropped += 1;
            return Err(std::io::Error::other("log file unavailable"));
        }
        match self.fs.create(&self.path) {
            Ok(mut file) => {
                self.size = 0;
                self.rotate_at = self.max_bytes;
                self.retry_after = None;
                if self.dropped > 0 {
                    if let Some(note) = self.drop_note {
                        let line = note(self.dropped);
                        if file.write_all(line.as_bytes()).is_ok() {
                            self.size += line.len() as u64;
                        }
                    }
                }
                self.dropped = 0;
                self.file = Some(file);
                Ok(())
            }
            Err(error) => {
                self.dropped += 1;
                self.retry_after = Some(std::time::Instant::now() + self.create_retry);
                Err(error)
            }
        }
    }

    fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        if self.file.is_some() && self.size >= self.rotate_at {
            self.rotate();
        }
        if self.file.is_none() {
            self.recover_file()?;
        }
        let file = self.file.as_mut().expect("file was just created");
        file.write_all(buf)?;
        self.size += buf.len() as u64;
        Ok(())
    }
}

/// Append-only file that rotates itself at a size cap. Rotation happens
/// between writes, so callers that write whole lines get rotated files that
/// end on a line boundary. Clones share one writer, which lets Clear logs
/// pause and reopen it (see `clear_all_logs`).
#[derive(Clone)]
pub(crate) struct SizeRotatingFile {
    state: Arc<Mutex<RotatingState>>,
}

impl SizeRotatingFile {
    pub(crate) fn open(path: PathBuf, max_bytes: u64, generations: u8) -> std::io::Result<Self> {
        Self::open_with(Arc::new(RealFs), path, max_bytes, generations)
    }

    pub(crate) fn open_with(
        fs: Arc<dyn RotationFs>,
        path: PathBuf,
        max_bytes: u64,
        generations: u8,
    ) -> std::io::Result<Self> {
        let existing = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        if existing >= max_bytes {
            let _ = rotate_generations(fs.as_ref(), &path, generations);
        }
        let file = fs.create(&path)?;
        let size = file.metadata().map(|m| m.len()).unwrap_or(0);
        Ok(Self {
            state: Arc::new(Mutex::new(RotatingState {
                fs,
                path,
                max_bytes,
                generations,
                file: Some(file),
                size,
                rotate_at: max_bytes,
                dropped: 0,
                retry_after: None,
                create_retry: CREATE_RETRY_INTERVAL,
                drop_note: None,
            })),
        })
    }

    #[cfg(test)]
    pub(crate) fn with_create_retry(self, interval: std::time::Duration) -> Self {
        self.lock().create_retry = interval;
        self
    }

    /// Write `note(dropped)` once after the file becomes writable again, when
    /// events were dropped while it was not.
    pub(crate) fn with_drop_note(self, note: fn(u64) -> String) -> Self {
        self.lock().drop_note = Some(note);
        self
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, RotatingState> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }
}

impl Write for SizeRotatingFile {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.write_all(buf)?;
        Ok(buf.len())
    }

    fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        self.lock().write_all(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self.lock().file.as_mut() {
            Some(file) => file.flush(),
            None => Ok(()),
        }
    }
}

/// Writers Clear logs must coordinate with.
static ACTIVE_WRITERS: Mutex<Vec<SizeRotatingFile>> = Mutex::new(Vec::new());

fn register_writer(writer: &SizeRotatingFile) {
    ACTIVE_WRITERS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .push(writer.clone());
}

/// The line written to `events.jsonl` after it became writable again.
fn event_drop_note(dropped: u64) -> String {
    let event = AppEvent {
        timestamp: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        stream: "system".to_string(),
        level: "warn".to_string(),
        summary: "Log events dropped".to_string(),
        data: serde_json::json!({ "dropped_events": dropped }),
    };
    serde_json::to_string(&event).unwrap_or_default() + "\n"
}

/// Delete each writer's live file and rotated generations and reopen it on a
/// fresh file. Without this, a writer keeps appending to an unlinked file until
/// its next size rotation, so output after Clear logs is unreadable and never
/// shipped. Each writer is locked only for a few unlinks and one create, so
/// tracing callers are not stalled behind directory scans. Returns the live
/// paths so the caller does not delete the fresh files.
fn reset_writers(writers: &[SizeRotatingFile]) -> Vec<PathBuf> {
    writers
        .iter()
        .flat_map(|writer| {
            let mut state = writer.lock();
            for path in std::iter::once(state.path.clone())
                .chain(rotated_paths(&state.path, state.generations))
            {
                let _ = std::fs::remove_file(path);
            }
            state.reopen();
            std::iter::once(state.path.clone())
                .chain(rotated_paths(&state.path, state.generations))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The last `n` valid events across `paths` (oldest generation first), in
/// chronological order. Missing or unreadable files are skipped, so a crash
/// between a rotation's rename and re-create still restores the generations
/// that remain. This is the single restore read path.
fn read_recent_events(paths: &[PathBuf], n: usize) -> Vec<AppEvent> {
    let mut newest_first: Vec<AppEvent> = Vec::new();
    for path in paths.iter().rev() {
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        for line in content.lines().rev() {
            // Lines written before an upgrade predate the key-identity boundary.
            if let Ok(event) = serde_json::from_str::<AppEvent>(&sanitize_persisted_line(line)) {
                newest_first.push(event);
                if newest_first.len() >= n {
                    break;
                }
            }
        }
        if newest_first.len() >= n {
            break;
        }
    }
    newest_first.reverse();
    newest_first
}

/// Seed the ring buffer with the last `n` events from every retained JSONL
/// generation.
fn seed_buffer_from_jsonl(buffer: &Arc<Mutex<VecDeque<AppEvent>>>, n: usize) {
    let events = read_recent_events(&event_jsonl_paths(), n);
    let mut buf = buffer.lock().unwrap_or_else(|p| p.into_inner());
    for event in events {
        if buf.len() >= 500 {
            buf.pop_front();
        }
        buf.push_back(event);
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

    // Open JSONL file for appending (AppEvent format). It rotates at startup
    // and again at runtime whenever it reaches EVENT_LOG_MAX_BYTES.
    let jsonl_writer = SizeRotatingFile::open(
        jsonl_path().expect("Could not determine JSONL path"),
        EVENT_LOG_MAX_BYTES,
        EVENT_LOG_GENERATIONS,
    )
    .expect("Could not open JSONL file")
    .with_drop_note(event_drop_note);
    register_writer(&jsonl_writer);

    // Layer 1: Pretty file. Size-capped; the non-blocking worker thread does
    // the rotation, so logging call sites never wait on it.
    let pretty_file = SizeRotatingFile::open(
        log_dir.join(log_file_name),
        APP_LOG_MAX_BYTES,
        APP_LOG_GENERATIONS,
    )
    .expect("Could not open app log file")
    .with_drop_note(|dropped| {
        format!("WARN log file was unavailable; {dropped} log lines were dropped\n")
    });
    register_writer(&pretty_file);
    let (pretty_writer, pretty_guard) = tracing_appender::non_blocking(pretty_file);
    let pretty_layer = tracing_subscriber::fmt::layer()
        .event_format(key_identity::PrettyFormat)
        .with_writer(pretty_writer)
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
        "events.jsonl.2",
        "events.dev.jsonl",
        "events.dev.jsonl.1",
        "events.dev.jsonl.2",
    ];
    // Live writers first: their files are unlinked and reopened under their own
    // locks, so output after a clear is neither lost nor written to an unlinked
    // handle. The slower cleanup below runs with no lock held.
    let writers = ACTIVE_WRITERS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    // Every file a live writer manages (current and rotated generations) is
    // excluded below: logging resumes once the locks drop, and a rotation in
    // the meantime must not be deleted by the unlocked passes.
    let live = reset_writers(&writers);
    for file in files {
        let path = dir.join(file);
        if !live.contains(&path) {
            let _ = std::fs::remove_file(path);
        }
    }
    // Clean up dated rolling files (e.g. app.dev.log.2026-03-02, events.dev.jsonl.2026-03-02)
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if (name.contains(".log.") || name.contains(".jsonl.")) && !live.contains(&entry.path())
            {
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
    fn updater_events_keep_only_bounded_operational_fields() {
        for debug_build in [false, true] {
            let mut data = serde_json::json!({
                "event_code": "updater.download_starting",
                "version": "0.24.0",
                "selection": "fresh_check",
                "message": "private content",
            });
            sanitize_event_data("system", &mut data, debug_build);
            assert_eq!(
                data,
                serde_json::json!({
                    "event_code": "updater.download_starting",
                    "version": "0.24.0",
                    "selection": "fresh_check",
                })
            );
            let mut invalid = serde_json::json!({
                "event_code": "updater.download_starting",
                "version": "1.2.3-private",
                "selection": "private content",
            });
            sanitize_event_data("system", &mut invalid, debug_build);
            assert_eq!(
                invalid,
                serde_json::json!({ "event_code": "updater.download_starting" })
            );
        }
    }

    fn rotation_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("murmur_telemetry_rotation_{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap_or_default()
    }

    #[test]
    fn generation_paths_append_a_numeric_suffix() {
        let log = Path::new("/logs/events.dev.jsonl");
        assert_eq!(
            generation_path(log, 1),
            Path::new("/logs/events.dev.jsonl.1")
        );
        assert_eq!(
            rotated_paths(Path::new("/logs/app.log"), 2),
            vec![
                PathBuf::from("/logs/app.log.1"),
                PathBuf::from("/logs/app.log.2")
            ]
        );
    }

    #[test]
    fn size_rotating_file_stays_below_threshold_without_rotating() {
        let dir = rotation_dir("below");
        let path = dir.join("events.jsonl");
        let mut writer = SizeRotatingFile::open(path.clone(), 20, 2).unwrap();
        writer.write_all(b"aaaaaaaaa\n").unwrap(); // 10 bytes
        writer.write_all(b"bbbbbbbb\n").unwrap(); // 19 bytes, still under 20
        assert_eq!(read(&path), "aaaaaaaaa\nbbbbbbbb\n");
        assert!(!generation_path(&path, 1).exists());
    }

    #[test]
    fn size_rotating_file_rotates_at_runtime_on_the_next_event_after_the_cap() {
        let dir = rotation_dir("runtime");
        let path = dir.join("events.jsonl");
        let mut writer = SizeRotatingFile::open(path.clone(), 20, 2).unwrap();
        writer.write_all(b"aaaaaaaaa\n").unwrap();
        writer.write_all(b"bbbbbbbbb\n").unwrap(); // reaches 20 bytes
        assert!(!generation_path(&path, 1).exists());
        writer.write_all(b"ccc\n").unwrap(); // triggers rotation first
        assert_eq!(read(&generation_path(&path, 1)), "aaaaaaaaa\nbbbbbbbbb\n");
        assert_eq!(read(&path), "ccc\n");
    }

    #[test]
    fn size_rotating_file_keeps_only_the_configured_generations() {
        let dir = rotation_dir("generations");
        let path = dir.join("events.jsonl");
        let mut writer = SizeRotatingFile::open(path.clone(), 4, 2).unwrap();
        for line in ["one\n", "two\n", "three\n", "four\n"] {
            writer.write_all(line.as_bytes()).unwrap();
        }
        // The third rotation pushes "two" into .2, overwriting "one".
        assert_eq!(read(&path), "four\n");
        assert_eq!(read(&generation_path(&path, 1)), "three\n");
        assert_eq!(read(&generation_path(&path, 2)), "two\n");
        assert!(!generation_path(&path, 3).exists());
    }

    #[test]
    fn size_rotating_file_rotates_an_oversized_file_at_startup() {
        let dir = rotation_dir("startup");
        let path = dir.join("app.log");
        std::fs::write(&path, "x".repeat(64)).unwrap();
        let mut writer = SizeRotatingFile::open(path.clone(), 32, 1).unwrap();
        writer.write_all(b"fresh\n").unwrap();
        assert_eq!(read(&path), "fresh\n");
        assert_eq!(read(&generation_path(&path, 1)).len(), 64);
    }

    #[test]
    fn size_rotating_file_resumes_the_size_of_an_existing_file() {
        let dir = rotation_dir("resume");
        let path = dir.join("events.jsonl");
        std::fs::write(&path, "0123456789\n").unwrap(); // 11 bytes, under the cap
        let mut writer = SizeRotatingFile::open(path.clone(), 16, 1).unwrap();
        writer.write_all(b"abcde\n").unwrap(); // 17 bytes
        writer.write_all(b"next\n").unwrap();
        assert_eq!(read(&generation_path(&path, 1)), "0123456789\nabcde\n");
        assert_eq!(read(&path), "next\n");
    }

    #[test]
    fn size_rotating_file_recreates_a_file_deleted_underneath_it() {
        let dir = rotation_dir("deleted");
        let path = dir.join("events.jsonl");
        let mut writer = SizeRotatingFile::open(path.clone(), 8, 2).unwrap();
        writer.write_all(b"aaaaaaaa\n").unwrap();
        std::fs::remove_file(&path).unwrap(); // Clear logs
        writer.write_all(b"after\n").unwrap();
        assert_eq!(read(&path), "after\n");
    }

    #[test]
    fn clearing_below_the_threshold_reopens_the_writer_on_a_fresh_file() {
        let dir = rotation_dir("clear_below");
        let path = dir.join("events.jsonl");
        let mut writer = SizeRotatingFile::open(path.clone(), 1024, 2).unwrap();
        writer.write_all(b"before\n").unwrap();

        reset_writers(std::slice::from_ref(&writer));
        // The file exists, empty, immediately, before any further event.
        assert_eq!(read(&path), "");

        writer.write_all(b"after\n").unwrap();
        assert_eq!(read(&path), "after\n");
    }

    #[test]
    fn clearing_resets_every_registered_writer_and_the_size_counter() {
        let dir = rotation_dir("clear_all");
        let events = SizeRotatingFile::open(dir.join("events.jsonl"), 16, 2).unwrap();
        let app = SizeRotatingFile::open(dir.join("app.log"), 16, 1).unwrap();
        let (mut events_writer, mut app_writer) = (events.clone(), app.clone());
        events_writer.write_all(b"aaaaaaa\n").unwrap();
        app_writer.write_all(b"bbbbbbb\n").unwrap();

        std::fs::write(dir.join("events.jsonl.1"), "old\n").unwrap();
        let live = reset_writers(&[events, app]);
        // Every managed path is protected from the unlocked cleanup passes.
        assert_eq!(
            live,
            [
                dir.join("events.jsonl"),
                dir.join("events.jsonl.1"),
                dir.join("events.jsonl.2"),
                dir.join("app.log"),
                dir.join("app.log.1"),
            ]
        );
        // Rotated generations go with the live files.
        assert!(!dir.join("events.jsonl.1").exists());

        // The cleared bytes must not count toward the next rotation.
        events_writer.write_all(b"ccccccc\n").unwrap();
        events_writer.write_all(b"ddddddd\n").unwrap();
        app_writer.write_all(b"eeeeeee\n").unwrap();
        assert_eq!(read(&dir.join("events.jsonl")), "ccccccc\nddddddd\n");
        assert!(!dir.join("events.jsonl.1").exists());
        assert_eq!(read(&dir.join("app.log")), "eeeeeee\n");
    }

    #[test]
    fn failed_recreate_never_appends_to_the_rotated_generation() {
        use std::sync::atomic::Ordering::SeqCst;
        let dir = rotation_dir("recreate_fails");
        let path = dir.join("events.jsonl");
        let fs = Arc::new(FailingFs::default());
        let mut writer = SizeRotatingFile::open_with(fs.clone(), path.clone(), 16, 2)
            .unwrap()
            .with_create_retry(std::time::Duration::ZERO);
        writer.write_all(b"aaaaaaa\n").unwrap();
        writer.write_all(b"bbbbbbb\n").unwrap();

        // The rename of current → .1 succeeds, then re-creating current fails.
        fs.fail_create.store(true, SeqCst);
        assert!(writer.write_all(b"ccccccc\n").is_err());
        assert!(writer.write_all(b"ddddddd\n").is_err());
        assert_eq!(read(&generation_path(&path, 1)), "aaaaaaa\nbbbbbbb\n");
        assert!(!path.exists());

        // Once creation works again the very next write lands in current.
        fs.fail_create.store(false, SeqCst);
        writer.write_all(b"eeeeeee\n").unwrap();
        assert_eq!(read(&path), "eeeeeee\n");
        assert_eq!(read(&generation_path(&path, 1)), "aaaaaaa\nbbbbbbb\n");
    }

    #[test]
    fn failed_rename_keeps_appending_to_current_and_retries_later() {
        use std::sync::atomic::Ordering::SeqCst;
        let dir = rotation_dir("rename_fails");
        let path = dir.join("events.jsonl");
        let fs = Arc::new(FailingFs::default());
        let mut writer = SizeRotatingFile::open_with(fs.clone(), path.clone(), 16, 2).unwrap();
        writer.write_all(b"aaaaaaa\n").unwrap();
        writer.write_all(b"bbbbbbb\n").unwrap();

        fs.fail_rename.store(true, SeqCst);
        writer.write_all(b"ccccccc\n").unwrap();
        assert_eq!(read(&path), "aaaaaaa\nbbbbbbb\nccccccc\n");
        assert!(!generation_path(&path, 1).exists());

        // The retry waits for ROTATION_RETRY_BYTES more data.
        fs.fail_rename.store(false, SeqCst);
        let filler = format!("{}\n", "x".repeat(ROTATION_RETRY_BYTES as usize));
        writer.write_all(filler.as_bytes()).unwrap();
        assert!(!generation_path(&path, 1).exists());
        writer.write_all(b"after\n").unwrap();
        assert_eq!(read(&path), "after\n");
        assert!(read(&generation_path(&path, 1)).starts_with("aaaaaaa\nbbbbbbb\nccccccc\n"));
    }

    #[test]
    fn create_failures_back_off_count_drops_and_leave_one_note_on_recovery() {
        use std::sync::atomic::Ordering::SeqCst;
        let dir = rotation_dir("backoff");
        let path = dir.join("events.jsonl");
        let fs = Arc::new(FailingFs::default());
        let mut writer = SizeRotatingFile::open_with(fs.clone(), path.clone(), 16, 2)
            .unwrap()
            .with_create_retry(std::time::Duration::from_secs(3600))
            .with_drop_note(|dropped| format!("dropped {dropped}\n"));
        writer.write_all(b"aaaaaaa\n").unwrap();
        writer.write_all(b"bbbbbbb\n").unwrap();
        let opened = fs.create_calls.load(SeqCst);

        // The first write after the rotation tries to create the file; the
        // rest fall inside the backoff window and do no file work at all.
        fs.fail_create.store(true, SeqCst);
        for _ in 0..5 {
            assert!(writer.write_all(b"lost\n").is_err());
        }
        assert_eq!(fs.create_calls.load(SeqCst), opened + 1);
        assert!(!path.exists());

        // After the window passes and creation works, the file starts with one
        // note carrying the count, then the new event. Nothing is replayed.
        fs.fail_create.store(false, SeqCst);
        writer.lock().retry_after = Some(std::time::Instant::now());
        writer.write_all(b"back\n").unwrap();
        writer.write_all(b"again\n").unwrap();
        assert_eq!(read(&path), "dropped 5\nback\nagain\n");
    }

    #[test]
    fn event_drop_note_is_a_valid_content_free_event() {
        let line = event_drop_note(7);
        assert!(line.ends_with('\n'));
        let event: AppEvent = serde_json::from_str(line.trim_end()).unwrap();
        assert_eq!(event.stream, "system");
        assert_eq!(event.level, "warn");
        assert_eq!(event.data["dropped_events"], 7);
    }

    fn event_line(n: usize) -> String {
        let event = AppEvent {
            timestamp: format!("2026-01-01T00:00:00.{n:03}Z"),
            stream: "system".to_string(),
            level: "info".to_string(),
            summary: format!("event {n}"),
            data: serde_json::json!({}),
        };
        serde_json::to_string(&event).unwrap() + "\n"
    }

    fn summaries(events: &[AppEvent]) -> Vec<String> {
        events.iter().map(|e| e.summary.clone()).collect()
    }

    #[test]
    fn restart_right_after_rotation_restores_events_from_older_generations() {
        let dir = rotation_dir("restore");
        let path = dir.join("events.jsonl");
        let paths = vec![
            generation_path(&path, 2),
            generation_path(&path, 1),
            path.clone(),
        ];
        // Events 1..=3 rotated away to .2, 4..=7 to .1, and only 8..=9 landed
        // in the fresh file before the restart. Line "bad" is skipped.
        let join =
            |range: std::ops::RangeInclusive<usize>| range.map(event_line).collect::<String>();
        std::fs::write(&paths[0], join(1..=3)).unwrap();
        std::fs::write(&paths[1], join(4..=7) + "bad\n").unwrap();
        std::fs::write(&paths[2], join(8..=9)).unwrap();

        let restored = read_recent_events(&paths, 5);
        assert_eq!(
            summaries(&restored),
            ["event 5", "event 6", "event 7", "event 8", "event 9"]
        );
        assert_eq!(read_recent_events(&paths, 100).len(), 9);
    }

    #[test]
    fn restore_sanitizes_key_identity_from_every_generation() {
        let dir = rotation_dir("restore_key_identity");
        let path = dir.join("events.jsonl");
        let paths = vec![
            generation_path(&path, 2),
            generation_path(&path, 1),
            path.clone(),
        ];
        // Lines as v0.51.6–v0.52.2 wrote them, one per generation.
        let legacy = |n: usize, key: &str| {
            format!(
                "{{\"timestamp\":\"t{n}\",\"stream\":\"keyboard\",\"level\":\"info\",\"summary\":\"Escape pressed\",\"data\":{{\"reason\":\"combo_cancelled\",\"event_type\":\"KeyPress({key})\",\"key\":\"Some({key})\"}}}}\n"
            )
        };
        std::fs::write(&paths[0], legacy(1, "KeyI")).unwrap();
        std::fs::write(&paths[1], legacy(2, "Num4") + &event_line(3)).unwrap();
        std::fs::write(&paths[2], legacy(4, "SemiColon")).unwrap();

        let restored = read_recent_events(&paths, 500);
        let timestamps: Vec<&str> = restored.iter().map(|e| e.timestamp.as_str()).collect();
        assert_eq!(timestamps, ["t1", "t2", "2026-01-01T00:00:00.003Z", "t4"]);
        let encoded = serde_json::to_string(&restored).unwrap();
        for leak in ["KeyI", "Num4", "SemiColon", "KeyPress(", "Escape"] {
            assert!(!encoded.contains(leak), "{leak} restored: {encoded}");
        }
        assert_eq!(restored[0].data["reason"], "combo_cancelled");
        assert_eq!(restored[2].summary, "event 3");
    }

    #[test]
    fn restore_survives_a_crash_between_rename_and_recreate() {
        let dir = rotation_dir("restore_crash");
        let path = dir.join("events.jsonl");
        let paths = vec![
            generation_path(&path, 2),
            generation_path(&path, 1),
            path.clone(),
        ];
        std::fs::write(&paths[1], (1..=4).map(event_line).collect::<String>()).unwrap();
        // The current file was renamed away and never recreated.
        let restored = read_recent_events(&paths, 500);
        assert_eq!(
            summaries(&restored),
            ["event 1", "event 2", "event 3", "event 4"]
        );
    }

    #[test]
    fn event_jsonl_paths_list_oldest_generation_first() {
        let paths = event_jsonl_paths();
        if let Some(current) = jsonl_path() {
            assert_eq!(
                paths,
                vec![
                    generation_path(&current, 2),
                    generation_path(&current, 1),
                    current
                ]
            );
        }
    }

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
    fn input_resolution_schema_is_exact_and_content_free_in_every_build() {
        for debug_build in [true, false] {
            let mut data = serde_json::json!({
                "event_code": "audio.input_resolution_observed",
                "capture_id": 41,
                "owner": 7,
                "owner_kind": "dictation",
                "backend": "auhal",
                "resolution_pass": 2,
                "backend_attempt": 1,
                "microphone_mode": "pinned",
                "input_enumeration_ok": true,
                "requested_present": false,
                "requested_present_known": true,
                "input_device_count": 3,
                "input_device_count_capped": false,
                "default_input_available": true,
                "device_id": "SENTINEL_PRIVATE_UID",
                "device_name": "SENTINEL_PRIVATE_MICROPHONE",
                "default_input_id": "SENTINEL_PRIVATE_DEFAULT_UID",
                "uid": "SENTINEL_PRIVATE_UID",
                "raw_error": "SENTINEL /Users/private CoreAudio error",
                "arbitrary_string": "SENTINEL_TRANSCRIPT"
            });

            sanitize_event_data("audio", &mut data, debug_build);

            assert_eq!(data["event_code"], "audio.input_resolution_observed");
            assert_eq!(data["microphone_mode"], "pinned");
            assert_eq!(data["requested_present"], false);
            assert_eq!(data["requested_present_known"], true);
            assert_eq!(data["input_device_count"], 3);
            assert_eq!(data["default_input_available"], true);
            assert_eq!(data.as_object().unwrap().len(), 14);
            let encoded = serde_json::to_string(&data).unwrap();
            assert!(!encoded.contains("SENTINEL"));
            assert!(!encoded.contains("Users"));
            assert!(!encoded.contains("device_id"));
            assert!(!encoded.contains("device_name"));
            assert!(!encoded.contains("default_input_id"));
            assert!(!encoded.contains("raw_error"));

            let summary = sanitized_summary(
                "audio",
                Some("SENTINEL_PRIVATE_MICROPHONE /Users/private".to_string()),
                &data,
                debug_build,
            );
            assert_eq!(summary, "Microphone input resolution observed");
        }
    }

    #[test]
    fn input_resolution_schema_rejects_invalid_or_contradictory_evidence() {
        for mut data in [
            serde_json::json!({
                "event_code": "audio.input_resolution_observed",
                "capture_id": 41,
                "owner": 7,
                "owner_kind": "dictation",
                "backend": "auhal",
                "resolution_pass": 1,
                "backend_attempt": 1,
                "microphone_mode": "pinned",
                "input_enumeration_ok": true,
                "requested_present": false,
                "requested_present_known": false,
                "input_device_count": 3,
                "input_device_count_capped": false,
                "default_input_available": true
            }),
            serde_json::json!({
                "event_code": "audio.input_resolution_observed",
                "capture_id": 41,
                "owner": 7,
                "owner_kind": "dictation",
                "backend": "cpal",
                "resolution_pass": 1,
                "backend_attempt": 2,
                "microphone_mode": "system_default",
                "input_enumeration_ok": true,
                "requested_present": true,
                "requested_present_known": false,
                "input_device_count": 257,
                "input_device_count_capped": true,
                "default_input_available": false
            }),
            serde_json::json!({
                "event_code": "audio.input_resolution_observed",
                "capture_id": 41,
                "owner": 7,
                "owner_kind": "dictation",
                "backend": "SENTINEL_PRIVATE_BACKEND",
                "resolution_pass": 1,
                "backend_attempt": 1,
                "microphone_mode": "pinned",
                "input_enumeration_ok": false,
                "requested_present": false,
                "requested_present_known": false,
                "input_device_count": 1,
                "input_device_count_capped": false,
                "default_input_available": true
            }),
            serde_json::json!({
                "event_code": "audio.input_resolution_observed",
                "capture_id": 41,
                "owner": 7,
                "owner_kind": "dictation",
                "backend": "auhal",
                "microphone_mode": "pinned",
                "input_enumeration_ok": true,
                "requested_present": false,
                "requested_present_known": true,
                "input_device_count": 3,
                "input_device_count_capped": false,
                "default_input_available": true
            }),
        ] {
            sanitize_event_data("audio", &mut data, true);
            assert_eq!(data["event_code"], "audio.input_resolution_observed");
            assert_eq!(data.as_object().unwrap().len(), 1);
            assert!(!serde_json::to_string(&data).unwrap().contains("SENTINEL"));
        }

        let mut unavailable = serde_json::json!({
            "event_code": "audio.input_resolution_observed",
            "capture_id": 9,
            "owner": 3,
            "owner_kind": "preview",
            "backend": "cpal",
            "resolution_pass": 3,
            "backend_attempt": 2,
            "microphone_mode": "pinned",
            "input_enumeration_ok": false,
            "requested_present": false,
            "requested_present_known": false,
            "input_device_count": 0,
            "input_device_count_capped": false,
            "default_input_available": true
        });
        sanitize_event_data("audio", &mut unavailable, true);
        assert_eq!(unavailable.as_object().unwrap().len(), 14);
    }

    #[test]
    fn device_reresolution_schema_is_exact_and_content_free_in_every_build() {
        for debug_build in [true, false] {
            let mut data = serde_json::json!({
                "event_code": "audio.device_reresolution_started",
                "owner": 12,
                "owner_kind": "dictation",
                "completed_pass": 1,
                "next_pass": 2,
                "retry_delay_ms": 500,
                "error_kind": "device_unavailable",
                "device_id": "SENTINEL_PRIVATE_UID",
                "device_name": "SENTINEL_PRIVATE_MICROPHONE",
                "default_input_id": "SENTINEL_PRIVATE_DEFAULT_UID",
                "raw_error": "SENTINEL /Users/private CoreAudio error",
                "transcript": "SENTINEL_PRIVATE_TRANSCRIPT"
            });

            sanitize_event_data("audio", &mut data, debug_build);

            assert_eq!(data["event_code"], "audio.device_reresolution_started");
            assert_eq!(data["owner"], 12);
            assert_eq!(data["owner_kind"], "dictation");
            assert_eq!(data["completed_pass"], 1);
            assert_eq!(data["next_pass"], 2);
            assert_eq!(data["retry_delay_ms"], 500);
            assert_eq!(data["error_kind"], "device_unavailable");
            assert_eq!(data.as_object().unwrap().len(), 7);
            let encoded = serde_json::to_string(&data).unwrap();
            assert!(!encoded.contains("SENTINEL"));
            assert!(!encoded.contains("Users"));
            assert!(!encoded.contains("device_id"));
            assert!(!encoded.contains("device_name"));
            assert!(!encoded.contains("default_input_id"));
            assert!(!encoded.contains("raw_error"));
            assert!(!encoded.contains("transcript"));

            let summary = sanitized_summary(
                "audio",
                Some("SENTINEL_PRIVATE_MICROPHONE /Users/private".to_string()),
                &data,
                debug_build,
            );
            assert_eq!(summary, "Microphone device re-resolution started");
        }
    }

    #[test]
    fn microphone_benchmark_owner_keeps_only_content_free_audio_evidence() {
        for debug_build in [true, false] {
            let mut input_resolution = serde_json::json!({
                "event_code": "audio.input_resolution_observed",
                "capture_id": 44,
                "owner": 9,
                "owner_kind": "microphone_benchmark",
                "backend": "auhal",
                "resolution_pass": 1,
                "backend_attempt": 1,
                "microphone_mode": "pinned",
                "input_enumeration_ok": true,
                "requested_present": true,
                "requested_present_known": true,
                "input_device_count": 2,
                "input_device_count_capped": false,
                "default_input_available": true,
                "device_id": "SENTINEL_PRIVATE_UID",
                "device_name": "SENTINEL_PRIVATE_MICROPHONE",
                "raw_error": "SENTINEL /Users/private CoreAudio error",
                "transcript": "SENTINEL_PRIVATE_TRANSCRIPT"
            });
            sanitize_event_data("audio", &mut input_resolution, debug_build);
            assert_eq!(input_resolution["owner_kind"], "microphone_benchmark");
            assert_eq!(input_resolution.as_object().unwrap().len(), 14);

            let mut reresolution = serde_json::json!({
                "event_code": "audio.device_reresolution_started",
                "owner": 9,
                "owner_kind": "microphone_benchmark",
                "completed_pass": 2,
                "next_pass": 3,
                "retry_delay_ms": 500,
                "error_kind": "device_unavailable",
                "device_id": "SENTINEL_PRIVATE_UID",
                "device_name": "SENTINEL_PRIVATE_MICROPHONE",
                "raw_error": "SENTINEL /Users/private CoreAudio error",
                "transcript": "SENTINEL_PRIVATE_TRANSCRIPT"
            });
            sanitize_event_data("audio", &mut reresolution, debug_build);
            assert_eq!(reresolution["owner_kind"], "microphone_benchmark");
            assert_eq!(reresolution.as_object().unwrap().len(), 7);

            for event in [&input_resolution, &reresolution] {
                let encoded = serde_json::to_string(event).unwrap();
                assert!(!encoded.contains("SENTINEL"));
                assert!(!encoded.contains("Users"));
                assert!(!encoded.contains("device_id"));
                assert!(!encoded.contains("device_name"));
                assert!(!encoded.contains("raw_error"));
                assert!(!encoded.contains("transcript"));
            }

            let mut unrecognized_owner = serde_json::json!({
                "event_code": "audio.device_reresolution_started",
                "owner": 9,
                "owner_kind": "microphone_benchmark:SENTINEL_PRIVATE_UID",
                "completed_pass": 2,
                "next_pass": 3,
                "retry_delay_ms": 500,
                "error_kind": "device_unavailable"
            });
            sanitize_event_data("audio", &mut unrecognized_owner, debug_build);
            assert_eq!(unrecognized_owner.as_object().unwrap().len(), 1);
            assert_eq!(
                unrecognized_owner["event_code"],
                "audio.device_reresolution_started"
            );
            assert!(!serde_json::to_string(&unrecognized_owner)
                .unwrap()
                .contains("SENTINEL"));
        }
    }

    #[test]
    fn audio_graph_observation_schema_is_exact_and_content_free_in_every_build() {
        for debug_build in [true, false] {
            let mut data = serde_json::json!({
                "event_code": "audio.system_audio_graph_observed",
                "capture_id": 42,
                "running_audio_process_count": 3,
                "tap_count": 1,
                "device_count": 5,
                "devices_running_count": 2,
                "query_timed_out": false,
                "probe_unavailable": false,
                "elapsed_ms": 18,
                "pid": 311,
                "process_name": "SENTINEL_PRIVATE_PROCESS",
                "tap_uid": "SENTINEL_PRIVATE_TAP_UID",
                "device_name": "SENTINEL_PRIVATE_MICROPHONE",
                "device_id": "SENTINEL_PRIVATE_UID",
                "report": "SENTINEL /Users/private audio graph"
            });

            sanitize_event_data("audio", &mut data, debug_build);

            let mut keys: Vec<&str> = data
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            keys.sort_unstable();
            assert_eq!(
                keys,
                vec![
                    "capture_id",
                    "device_count",
                    "devices_running_count",
                    "elapsed_ms",
                    "event_code",
                    "probe_unavailable",
                    "query_timed_out",
                    "running_audio_process_count",
                    "tap_count",
                ]
            );
            let encoded = serde_json::to_string(&data).unwrap();
            assert!(!encoded.contains("SENTINEL"));
            assert!(!encoded.contains("Users"));
            assert!(!encoded.contains("pid"));
            assert!(!encoded.contains("process_name"));
            assert!(!encoded.contains("tap_uid"));
            assert!(!encoded.contains("device_name"));
            assert!(!encoded.contains("device_id"));

            let summary = sanitized_summary(
                "audio",
                Some("SENTINEL_PRIVATE_MICROPHONE /Users/private".to_string()),
                &data,
                debug_build,
            );
            assert_eq!(summary, "System audio graph observed");
        }
    }

    #[test]
    fn audio_graph_observation_schema_rejects_invalid_or_contradictory_counts() {
        let base = serde_json::json!({
            "event_code": "audio.system_audio_graph_observed",
            "capture_id": 7,
            "running_audio_process_count": 1,
            "tap_count": 0,
            "device_count": 4,
            "devices_running_count": 1,
            "query_timed_out": false,
            "probe_unavailable": false,
            "elapsed_ms": 9
        });
        let collapses = |label: &str, key: &str, value: serde_json::Value| {
            let mut data = base.clone();
            data[key] = value;
            sanitize_event_data("audio", &mut data, true);
            assert_eq!(
                data.as_object().unwrap().len(),
                1,
                "{label} must collapse to the event code alone"
            );
            assert_eq!(data["event_code"], "audio.system_audio_graph_observed");
        };
        collapses(
            "more running devices than devices",
            "devices_running_count",
            serde_json::json!(9),
        );
        collapses(
            "a timed-out query that still counted objects",
            "query_timed_out",
            serde_json::json!(true),
        );
        collapses("a zero capture id", "capture_id", serde_json::json!(0));
        collapses(
            "a stringly-typed count",
            "tap_count",
            serde_json::json!("1"),
        );
        collapses(
            "an out-of-range elapsed time",
            "elapsed_ms",
            serde_json::json!(60_001),
        );
        collapses(
            "an unstarted probe that still counted objects",
            "probe_unavailable",
            serde_json::json!(true),
        );
        collapses(
            "a missing probe_unavailable flag",
            "probe_unavailable",
            serde_json::Value::Null,
        );
    }

    #[test]
    fn audio_graph_observation_distinguishes_a_timeout_from_an_unstarted_probe() {
        // A wedge we actually observed.
        let mut timed_out = serde_json::json!({
            "event_code": "audio.system_audio_graph_observed",
            "capture_id": 7,
            "running_audio_process_count": 0,
            "tap_count": 0,
            "device_count": 0,
            "devices_running_count": 0,
            "query_timed_out": true,
            "probe_unavailable": false,
            "elapsed_ms": 2_003
        });
        sanitize_event_data("audio", &mut timed_out, false);
        assert_eq!(timed_out.as_object().unwrap().len(), 9);
        assert_eq!(timed_out["query_timed_out"], true);
        assert_eq!(timed_out["probe_unavailable"], false);
        assert_eq!(timed_out["elapsed_ms"], 2_003);

        // A probe that never ran must not be reported as an observed wedge.
        let mut unavailable = serde_json::json!({
            "event_code": "audio.system_audio_graph_observed",
            "capture_id": 7,
            "running_audio_process_count": 0,
            "tap_count": 0,
            "device_count": 0,
            "devices_running_count": 0,
            "query_timed_out": false,
            "probe_unavailable": true,
            "elapsed_ms": 0
        });
        sanitize_event_data("audio", &mut unavailable, false);
        assert_eq!(unavailable.as_object().unwrap().len(), 9);
        assert_eq!(unavailable["query_timed_out"], false);
        assert_eq!(unavailable["probe_unavailable"], true);

        // Claiming both at once is incoherent evidence.
        let mut both = serde_json::json!({
            "event_code": "audio.system_audio_graph_observed",
            "capture_id": 7,
            "running_audio_process_count": 0,
            "tap_count": 0,
            "device_count": 0,
            "devices_running_count": 0,
            "query_timed_out": true,
            "probe_unavailable": true,
            "elapsed_ms": 5
        });
        sanitize_event_data("audio", &mut both, false);
        assert_eq!(both.as_object().unwrap().len(), 1);
    }

    #[test]
    fn device_reresolution_schema_rejects_invalid_or_contradictory_evidence() {
        for mut data in [
            serde_json::json!({
                "event_code": "audio.device_reresolution_started",
                "owner": 12,
                "owner_kind": "dictation",
                "completed_pass": 1,
                "next_pass": 3,
                "retry_delay_ms": 500,
                "error_kind": "device_unavailable"
            }),
            serde_json::json!({
                "event_code": "audio.device_reresolution_started",
                "owner": 12,
                "owner_kind": "preview",
                "completed_pass": 2,
                "next_pass": 3,
                "retry_delay_ms": 501,
                "error_kind": "device_unavailable"
            }),
            serde_json::json!({
                "event_code": "audio.device_reresolution_started",
                "owner": 12,
                "owner_kind": "preview",
                "completed_pass": 1,
                "next_pass": 2,
                "retry_delay_ms": 500,
                "error_kind": "SENTINEL_PRIVATE_ERROR"
            }),
            serde_json::json!({
                "event_code": "audio.device_reresolution_started",
                "owner": 0,
                "owner_kind": "transform",
                "completed_pass": 1,
                "next_pass": 2,
                "retry_delay_ms": 500,
                "error_kind": "device_unavailable"
            }),
        ] {
            sanitize_event_data("audio", &mut data, true);
            assert_eq!(data["event_code"], "audio.device_reresolution_started");
            assert_eq!(data.as_object().unwrap().len(), 1);
            assert!(!serde_json::to_string(&data).unwrap().contains("SENTINEL"));
        }

        let mut final_retry = serde_json::json!({
            "event_code": "audio.device_reresolution_started",
            "owner": 4,
            "owner_kind": "corpus",
            "completed_pass": 2,
            "next_pass": 3,
            "retry_delay_ms": 500,
            "error_kind": "device_unavailable"
        });
        sanitize_event_data("audio", &mut final_retry, true);
        assert_eq!(final_retry.as_object().unwrap().len(), 7);
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
    fn speculative_capture_codes_survive_release_summaries() {
        for code in [
            "audio.speculative_capture_refused",
            "audio.speculative_capture_flowing",
            "audio.speculative_capture_adopted",
            "audio.speculative_capture_discarded",
            "audio.speculative_capture_closed",
            "keyboard.fn_capture_skipped",
            "keyboard.globe_companion_ignored",
        ] {
            assert_eq!(canonical_event_code(code), Some(code));
            let data = serde_json::json!({"event_code": code, "capture_id": 4});
            assert_eq!(
                sanitized_summary("audio", Some("PRIVATE_SENTINEL".into()), &data, false),
                code
            );
        }
    }

    #[test]
    fn release_pipeline_preserves_content_free_recording_timings() {
        for code in [
            "audio.recording_status_timing",
            "audio.recording_context_ready",
        ] {
            let mut data = serde_json::json!({
                "event_code": code,
                "recording_id": 9,
                "context_ready": true,
                "ready_to_status_emit_ms": 2,
                "context_resolution_ms": 17,
                "error": "PRIVATE_SENTINEL",
                "transcript": "PRIVATE_SENTINEL"
            });

            sanitize_event_data("pipeline", &mut data, false);

            assert_eq!(data["event_code"], code);
            assert_eq!(data["recording_id"], 9);
            assert_eq!(data["context_ready"], true);
            assert_eq!(data["ready_to_status_emit_ms"], 2);
            assert_eq!(data["context_resolution_ms"], 17);
            assert!(!serde_json::to_string(&data)
                .unwrap()
                .contains("PRIVATE_SENTINEL"));
            assert_eq!(
                sanitized_summary("pipeline", Some("PRIVATE_SENTINEL".into()), &data, false),
                code
            );
        }
    }

    #[test]
    fn release_pipeline_keeps_only_allowlisted_dictation_lifecycle_strings() {
        let mut data = serde_json::json!({
            "recording_id": 9,
            "char_count": 42,
            "event_code": "pipeline.dictation_terminal",
            "outcome": "runtime_interruption",
            "error_code": "stream_invalidated",
            "model": "PRIVATE_MODEL",
            "error": "/Users/private/project"
        });

        sanitize_event_data("pipeline", &mut data, false);

        assert_eq!(data["recording_id"], 9);
        assert_eq!(data["char_count"], 42);
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
    fn dictation_terminal_never_retains_private_capture_content() {
        for debug_build in [true, false] {
            let mut data = serde_json::json!({
                "event_code": "pipeline.dictation_terminal",
                "recording_id": 9,
                "outcome": "success",
                "error_code": "none",
                "char_count": 24,
                "raw_text": "SENTINEL PRIVATE RAW",
                "final_text": "SENTINEL PRIVATE FINAL",
                "capture": {
                    "rawText": "SENTINEL PRIVATE NESTED RAW",
                    "finalText": "SENTINEL PRIVATE NESTED FINAL"
                }
            });

            sanitize_event_data("pipeline", &mut data, debug_build);

            let encoded = serde_json::to_string(&data).unwrap();
            assert_eq!(data["event_code"], "pipeline.dictation_terminal");
            assert_eq!(data["recording_id"], 9);
            assert_eq!(data["outcome"], "success");
            assert_eq!(data["error_code"], "none");
            assert_eq!(data["char_count"], 24);
            assert!(!encoded.contains("SENTINEL"));
            assert!(data.get("raw_text").is_none());
            assert!(data.get("final_text").is_none());
            assert!(data.get("capture").is_none());
        }
    }

    #[test]
    fn content_free_dictation_terminal_bytes_are_unchanged() {
        let mut data = serde_json::json!({
            "event_code": "pipeline.dictation_terminal",
            "recording_id": 41,
            "outcome": "success",
            "error_code": "none",
            "char_count": 12
        });
        let expected = serde_json::to_vec(&data).unwrap();

        sanitize_event_data("pipeline", &mut data, false);

        assert_eq!(serde_json::to_vec(&data).unwrap(), expected);
    }

    #[test]
    fn coreml_pass_kind_and_preview_skip_outcomes_survive_release_sanitization() {
        let mut data =
            serde_json::json!({"pass_kind": "preview", "recording_id": 41, "text": "PRIVATE"});
        sanitize_event_data("pipeline", &mut data, false);
        assert_eq!(
            data,
            serde_json::json!({"pass_kind": "preview", "recording_id": 41})
        );
    }

    #[test]
    fn preview_vad_disagreement_schema_is_exact_in_every_build() {
        for debug_build in [true, false] {
            {
                let (code, owner) = ("pipeline.preview_vad_disagreement", "recording_id");
                assert_eq!(canonical_event_code(code), Some(code));
                let mut data = serde_json::json!({
                    "event_code": code, owner: 41, "sample_count": 62_050,
                    "text": "SENTINEL_TRANSCRIPT", "other_id": 42, "nested": {"text": "PRIVATE"}
                });
                sanitize_event_data("pipeline", &mut data, debug_build);
                assert_eq!(
                    data,
                    serde_json::json!({"event_code": code, owner: 41, "sample_count": 62_050})
                );
            }
        }
    }

    #[test]
    fn dictation_partial_tick_schema_is_exact_and_content_free_in_every_build() {
        for debug_build in [true, false] {
            for outcome in [
                "emitted",
                "emit_failed",
                "stopped",
                "model_unavailable",
                "backend_unavailable",
                "decode_error",
                "worker_failed",
            ] {
                let mut data = serde_json::json!({
                    "event_code": "pipeline.dictation_partial_tick",
                    "recording_id": 9,
                    "outcome": outcome,
                    "sample_count": 320_000,
                    "text": "SENTINEL_TRANSCRIPT",
                    "path": "/Users/private/project"
                });

                sanitize_event_data("pipeline", &mut data, debug_build);

                assert_eq!(data["event_code"], "pipeline.dictation_partial_tick");
                assert_eq!(data["recording_id"], 9);
                assert_eq!(data["outcome"], outcome);
                assert_eq!(data["sample_count"], 320_000);
                assert_eq!(data.as_object().unwrap().len(), 4);
                assert!(!serde_json::to_string(&data).unwrap().contains("SENTINEL"));
            }
        }
    }

    #[test]
    fn dictation_preview_presentation_schema_is_exact_and_content_free_in_every_build() {
        for debug_build in [true, false] {
            for outcome in ["shown", "show_failed", "hidden", "hide_failed"] {
                let mut data = serde_json::json!({
                    "event_code": "pipeline.dictation_preview_presentation",
                    "recording_id": 9,
                    "outcome": outcome,
                    "text": "SENTINEL_TRANSCRIPT",
                    "error": "/Users/private/project"
                });

                sanitize_event_data("pipeline", &mut data, debug_build);

                assert_eq!(
                    data["event_code"],
                    "pipeline.dictation_preview_presentation"
                );
                assert_eq!(data["recording_id"], 9);
                assert_eq!(data["outcome"], outcome);
                assert_eq!(data.as_object().unwrap().len(), 3);
                let mut hide = serde_json::json!({
                    "event_code": "pipeline.dictation_preview_presentation",
                    "recording_id": 9,
                    "outcome": outcome,
                    "trigger": "after_paste",
                });
                sanitize_event_data("pipeline", &mut hide, debug_build);
                assert_eq!(hide["trigger"], "after_paste");
                let mut free_trigger = serde_json::json!({
                    "event_code": "pipeline.dictation_preview_presentation",
                    "recording_id": 9,
                    "trigger": "SENTINEL",
                });
                sanitize_event_data("pipeline", &mut free_trigger, debug_build);
                assert!(free_trigger.get("trigger").is_none());
                assert!(!serde_json::to_string(&data).unwrap().contains("SENTINEL"));
                assert!(!serde_json::to_string(&data).unwrap().contains("private"));
            }
        }
    }

    #[test]
    fn main_thread_watch_schema_is_exact_and_content_free_in_every_build() {
        for debug_build in [true, false] {
            let mut span = serde_json::json!({
                "event_code": "system.main_thread_paste_span",
                "recording_id": 4,
                "kind": "command",
                "label": "show_dictation_preview",
                "start_us": 10,
                "start_unix_ms": 20,
                "duration_us": 30,
                "overlap_us": 25,
                "queue_us": 0,
                "on_main": true,
                "longest_phase": "",
                "longest_phase_us": 0,
                "segments": 0,
                "title": "SENTINEL_WINDOW",
                "suppressed": 3
            });
            sanitize_event_data("system", &mut span, debug_build);
            assert_eq!(span["label"], "show_dictation_preview");
            assert_eq!(span["overlap_us"], 25);
            assert_eq!(span["longest_phase"], "");
            assert!(span.get("title").is_none());
            assert!(span.get("suppressed").is_none());
            assert_eq!(span.as_object().unwrap().len(), 13);

            let mut unsafe_label = serde_json::json!({
                "event_code": "system.main_thread_span",
                "kind": "clipboard",
                "label": "Copied secret text",
                "duration_us": 12_000
            });
            sanitize_event_data("system", &mut unsafe_label, debug_build);
            assert!(unsafe_label.get("label").is_none());
            assert!(unsafe_label.get("kind").is_none());
            assert_eq!(unsafe_label["duration_us"], 12_000);

            let mut wait = serde_json::json!({
                "event_code": "pipeline.main_thread_paste_wait",
                "recording_id": 4,
                "outcome": "started",
                "inject_queue_us": 80_000,
                "slow": true,
                "longest_label": "preview.hide",
                "longest_kind": "job",
                "event_loop_drain_us": 70_000,
                "preview_hide": "after_paste",
                "text": "SENTINEL_TRANSCRIPT"
            });
            sanitize_event_data("pipeline", &mut wait, debug_build);
            assert_eq!(wait["inject_queue_us"], 80_000);
            assert_eq!(wait["longest_label"], "preview.hide");
            assert_eq!(wait["event_loop_drain_us"], 70_000);
            assert!(!serde_json::to_string(&wait).unwrap().contains("SENTINEL"));
            assert_eq!(wait["preview_hide"], "after_paste");
            assert_eq!(wait.as_object().unwrap().len(), 9);
            assert_eq!(
                sanitized_summary("pipeline", Some("free text".into()), &wait, debug_build),
                "pipeline.main_thread_paste_wait"
            );
        }
    }

    #[test]
    fn main_thread_stack_event_keeps_symbol_names_and_numbers_only_in_every_build() {
        for debug_build in [true, false] {
            let frames = "libsystem_kernel.dylib!mach_msg2_trap;AppKit!-[NSView layout];ui!_ZN2ui4main17h0123456789abcdefE;?!?";
            let mut stack = serde_json::json!({
                "event_code": "system.main_thread_stack",
                "sample": 1,
                "busy_us": 61_000,
                "start_us": 5_000_000,
                "start_unix_ms": 1_700_000_000_000u64,
                "frame_count": 4,
                "frames": frames,
                "held_us": 9,
                "sampling_stopped": false,
                "suppressed": 2,
                "transcript": "SENTINEL_TRANSCRIPT",
                "thread_name": "SENTINEL_THREAD",
            });
            sanitize_event_data("system", &mut stack, debug_build);
            assert_eq!(stack["frames"], frames);
            assert_eq!(stack["held_us"], 9);
            assert_eq!(stack["sampling_stopped"], false);
            assert_eq!(stack.as_object().unwrap().len(), 10);
            assert!(!serde_json::to_string(&stack).unwrap().contains("SENTINEL"));
            assert_eq!(
                sanitized_summary(
                    "system",
                    Some("free text SENTINEL".into()),
                    &stack,
                    debug_build
                ),
                "system.main_thread_stack"
            );

            // Anything that could carry content or a path drops the field.
            for bad in [
                "ui!f; /Users/george/Documents/private.txt",
                "ui!f\"quoted secret\"",
                "ui!f=secret",
                "ui!f\nSENTINEL",
                "SENTINEL_NO_BANG",
                "ui!two SENTINEL words",
                "Some Image!f",
                "",
            ] {
                let mut data = serde_json::json!({
                    "event_code": "system.main_thread_stack",
                    "sample": 2,
                    "frame_count": 1,
                    "frames": bad,
                });
                sanitize_event_data("system", &mut data, debug_build);
                assert!(data.get("frames").is_none(), "{bad:?} survived");
                assert_eq!(data["sample"], 2);
            }
            let mut too_long = serde_json::json!({
                "event_code": "system.main_thread_stack",
                "frames": "a".repeat(crate::main_thread_watch::stack::MAX_FRAMES_LEN + 1),
                "frame_count": crate::main_thread_watch::stack::MAX_FRAMES + 1,
                "sample": 3,
                "sampling_stopped": "yes",
                "held_us": "9",
            });
            sanitize_event_data("system", &mut too_long, debug_build);
            assert_eq!(too_long.as_object().unwrap().len(), 1);
        }
    }

    #[test]
    fn main_thread_rates_and_watch_started_schemas_are_numbers_and_booleans_only() {
        for debug_build in [true, false] {
            let mut rates = serde_json::json!({
                "event_code": "system.main_thread_rates",
                "window_us": 1_000_123,
                "start_unix_ms": 1_700_000_000_000u64,
                "audio_level_emits": 33,
                "overlay_audio_level_emits": 17,
                "ipc_invokes": 60,
                "ipc_main_invokes": 7,
                "ipc_main_us": 1_800,
                "command": "SENTINEL_COMMAND",
                "audio_level_emits_text": "33",
            });
            sanitize_event_data("system", &mut rates, debug_build);
            assert_eq!(rates.as_object().unwrap().len(), 8);
            assert_eq!(rates["audio_level_emits"], 33);
            assert_eq!(rates["overlay_audio_level_emits"], 17);
            assert!(!serde_json::to_string(&rates).unwrap().contains("SENTINEL"));
            let mut stringly = serde_json::json!({
                "event_code": "system.main_thread_rates",
                "ipc_invokes": "SENTINEL",
                "audio_level_emits": -1,
            });
            sanitize_event_data("system", &mut stringly, debug_build);
            assert_eq!(stringly.as_object().unwrap().len(), 1);
            assert_eq!(
                sanitized_summary("system", Some("rates SENTINEL".into()), &rates, debug_build),
                "system.main_thread_rates"
            );

            let mut started = serde_json::json!({
                "event_code": "system.main_thread_watch_started",
                "epoch_unix_ms": 1,
                "observers_installed": true,
                "reporter_started": true,
                "log_span_min_us": 10_000,
                "slow_paste_us": 20_000,
                "stack_sampling": true,
                "stack_first_sample_us": 50_000,
                "path": "/Users/SENTINEL",
            });
            sanitize_event_data("system", &mut started, debug_build);
            assert_eq!(started.as_object().unwrap().len(), 8);
            assert_eq!(started["stack_sampling"], true);
            assert_eq!(started["stack_first_sample_us"], 50_000);
        }
    }

    #[test]
    fn labelled_apply_spans_keep_their_static_labels_in_every_build() {
        for debug_build in [true, false] {
            for label in [
                "applied.window.set_size",
                "overlay.detect_notch",
                "overlay.dock_prefs",
                "preview.follow_overlay",
            ] {
                let mut span = serde_json::json!({
                    "event_code": "system.main_thread_span",
                    "kind": "job",
                    "label": label,
                    "start_us": 1,
                    "start_unix_ms": 2,
                    "duration_us": 0,
                    "queue_us": 41_000,
                    "on_main": true,
                    "longest_phase": "",
                    "longest_phase_us": 0,
                    "segments": 0,
                });
                sanitize_event_data("system", &mut span, debug_build);
                assert_eq!(span["label"], label);
                assert_eq!(span["queue_us"], 41_000);
            }
        }
    }

    #[test]
    fn dictation_stage_gaps_schema_is_numbers_and_one_recording_id_in_every_build() {
        for debug_build in [true, false] {
            let mut gaps = serde_json::json!({
                "event_code": "pipeline.dictation_stage_gaps",
                "recording_id": 12,
                "total_us": 160_000,
                "unattributed_us": 700,
                "capture_finalization_us": 22_000,
                "handoff_us": 40,
                "pipeline_prologue_us": 90,
                "vad_us": 9_000,
                "vad_blocking_queue_us": 80,
                "pre_backend_us": 30,
                "backend_acquire_us": 400,
                "backend_model_queue_us": 0,
                "backend_model_load_us": 0,
                "backend_decode_us": 75_000,
                "backend_release_us": 20,
                "transform_setup_us": 15,
                "transform_us": 1_000,
                "post_transform_us": 300,
                "delivery_us": 5_000,
                "pipeline_return_us": 12,
                "idle_status_emit_us": 100,
                "tail_us": 50,
                "text": "SENTINEL_TRANSCRIPT",
                "model": "SENTINEL_MODEL",
                "outcome": "success",
            });
            sanitize_event_data("pipeline", &mut gaps, debug_build);
            assert_eq!(gaps.as_object().unwrap().len(), 22);
            assert_eq!(gaps["unattributed_us"], 700);
            assert!(!serde_json::to_string(&gaps).unwrap().contains("SENTINEL"));
            assert_eq!(
                sanitized_summary("pipeline", Some("gaps SENTINEL".into()), &gaps, debug_build),
                "pipeline.dictation_stage_gaps"
            );

            let mut invalid = serde_json::json!({
                "event_code": "pipeline.dictation_stage_gaps",
                "recording_id": 0,
                "total_us": "SENTINEL",
                "tail_us": -4,
                "vad_us": 1,
            });
            sanitize_event_data("pipeline", &mut invalid, debug_build);
            assert_eq!(invalid.as_object().unwrap().len(), 2);
            assert_eq!(invalid["vad_us"], 1);
        }
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
    fn permission_prompt_schema_is_exact_and_content_free_in_every_build() {
        for debug_build in [true, false] {
            for (state, prompt_pending_ms, expected_len) in
                [("pending", None, 5), ("resolved", Some(42_000), 6)]
            {
                let mut data = serde_json::json!({
                    "event_code": "audio.permission_prompt_changed",
                    "recording_id": 17,
                    "owner": 17,
                    "owner_kind": "dictation",
                    "state": state,
                    "device_name": "SENTINEL_PRIVATE_MICROPHONE",
                    "device_id": "SENTINEL_PRIVATE_UID",
                    "error": "/Users/private/CoreAudio",
                    "transcript": "SENTINEL_PRIVATE_TRANSCRIPT"
                });
                if let Some(prompt_pending_ms) = prompt_pending_ms {
                    data["prompt_pending_ms"] = prompt_pending_ms.into();
                }

                sanitize_event_data("audio", &mut data, debug_build);

                assert_eq!(data["event_code"], "audio.permission_prompt_changed");
                assert_eq!(data["recording_id"], 17);
                assert_eq!(data["owner"], 17);
                assert_eq!(data["owner_kind"], "dictation");
                assert_eq!(data["state"], state);
                assert_eq!(data.as_object().unwrap().len(), expected_len);
                let encoded = serde_json::to_string(&data).unwrap();
                assert!(!encoded.contains("SENTINEL"));
                assert!(!encoded.contains("Users"));
                assert_eq!(
                    sanitized_summary(
                        "audio",
                        Some("SENTINEL /Users/private".to_string()),
                        &data,
                        debug_build,
                    ),
                    "Microphone permission prompt state changed"
                );
            }
        }
    }

    #[test]
    fn permission_prompt_schema_rejects_cross_field_contradictions() {
        for mut data in [
            serde_json::json!({
                "event_code": "audio.permission_prompt_changed",
                "recording_id": 17,
                "owner": 18,
                "owner_kind": "dictation",
                "state": "pending"
            }),
            serde_json::json!({
                "event_code": "audio.permission_prompt_changed",
                "recording_id": 17,
                "owner": 17,
                "owner_kind": "transform",
                "state": "pending"
            }),
            serde_json::json!({
                "event_code": "audio.permission_prompt_changed",
                "recording_id": 17,
                "owner": 17,
                "owner_kind": "dictation",
                "state": "pending",
                "prompt_pending_ms": 1
            }),
            serde_json::json!({
                "event_code": "audio.permission_prompt_changed",
                "recording_id": 17,
                "owner": 17,
                "owner_kind": "dictation",
                "state": "resolved"
            }),
            serde_json::json!({
                "event_code": "audio.permission_prompt_changed",
                "recording_id": 17,
                "owner": 17,
                "owner_kind": "dictation",
                "state": "pending",
                "prompt_pending_ms": 300_001
            }),
        ] {
            sanitize_event_data("audio", &mut data, true);
            assert_eq!(data["event_code"], "audio.permission_prompt_changed");
            assert_eq!(data.as_object().unwrap().len(), 1);
        }
    }

    #[test]
    fn dictation_slo_schemas_are_exact_and_content_free_in_every_build() {
        for debug_build in [true, false] {
            for mut data in [
                serde_json::json!({
                    "event_code": "pipeline.dictation_requested",
                    "recording_id": 31,
                    "slo_contract": 1,
                    "origin": "hold",
                    "device_selection": "explicit",
                    "content": "SENTINEL_PRIVATE_TRANSCRIPT",
                    "device_id": "/Users/private/device",
                    "nested": {"raw_error": "SENTINEL"}
                }),
                serde_json::json!({
                    "event_code": "pipeline.dictation_state_changed",
                    "recording_id": 31,
                    "from": "recording",
                    "to": "processing",
                    "error": "/Users/private/error",
                    "content": "SENTINEL_PRIVATE_TRANSCRIPT"
                }),
                serde_json::json!({
                    "event_code": "pipeline.dictation_presentation",
                    "recording_id": 31,
                    "status_code": "microphone_initialization_failed",
                    "action_code": "choose_microphone",
                    "device_id": "/Users/private/device",
                    "message": "SENTINEL_PRIVATE_TRANSCRIPT"
                }),
                serde_json::json!({
                    "event_code": "pipeline.dictation_presentation",
                    "recording_id": 31,
                    "status_code": "microphone_interrupted",
                    "action_code": "wait_for_partial_transcription",
                    "message": "SENTINEL_PRIVATE_TRANSCRIPT",
                    "path": "/Users/private/project"
                }),
            ] {
                sanitize_event_data("pipeline", &mut data, debug_build);
                assert_eq!(data["recording_id"], 31);
                assert_eq!(
                    data.as_object().unwrap().len(),
                    if data["event_code"] == "pipeline.dictation_requested" {
                        5
                    } else {
                        4
                    }
                );
                let encoded = serde_json::to_string(&data).unwrap();
                assert!(!encoded.contains("SENTINEL"));
                assert!(!encoded.contains("Users"));
                assert_eq!(
                    sanitized_summary(
                        "pipeline",
                        Some("SENTINEL /Users/private".to_string()),
                        &data,
                        debug_build,
                    ),
                    "Dictation reliability event"
                );
            }
        }
    }

    #[test]
    fn dictation_slo_schemas_reject_invalid_states_and_presentations() {
        for mut data in [
            serde_json::json!({
                "event_code": "pipeline.dictation_requested",
                "recording_id": 31,
                "slo_contract": 2,
                "origin": "hold",
                "device_selection": "explicit"
            }),
            serde_json::json!({
                "event_code": "pipeline.dictation_requested",
                "recording_id": 31,
                "slo_contract": 1,
                "origin": "SENTINEL_PRIVATE_ORIGIN",
                "device_selection": "system_default"
            }),
            serde_json::json!({
                "event_code": "pipeline.dictation_requested",
                "recording_id": 31,
                "slo_contract": 1,
                "origin": "toggle",
                "device_selection": "SENTINEL_PRIVATE_DEVICE"
            }),
            serde_json::json!({
                "event_code": "pipeline.dictation_state_changed",
                "recording_id": 31,
                "from": "processing",
                "to": "processing"
            }),
            serde_json::json!({
                "event_code": "pipeline.dictation_state_changed",
                "recording_id": 0,
                "from": "SENTINEL_PRIVATE_STATE",
                "to": "idle"
            }),
            serde_json::json!({
                "event_code": "pipeline.dictation_presentation",
                "recording_id": 31,
                "status_code": "microphone_cleanup_stalled",
                "action_code": "retry"
            }),
            serde_json::json!({
                "event_code": "pipeline.dictation_presentation",
                "recording_id": 31,
                "status_code": "SENTINEL_PRIVATE_STATUS",
                "action_code": "restart_app"
            }),
        ] {
            sanitize_event_data("pipeline", &mut data, true);
            assert!(matches!(
                data["event_code"].as_str(),
                Some(
                    "pipeline.dictation_requested"
                        | "pipeline.dictation_state_changed"
                        | "pipeline.dictation_presentation"
                )
            ));
            assert_eq!(data.as_object().unwrap().len(), 1);
            assert!(!serde_json::to_string(&data).unwrap().contains("SENTINEL"));
        }
    }

    #[test]
    fn paste_safety_schema_is_bounded_and_content_free_in_every_build() {
        for debug_build in [true, false] {
            for (phase, outcome) in [
                ("preparation", "ready"),
                ("preparation", "activation_requested"),
                ("preparation", "refused"),
                ("preparation", "refused_clipboard_only"),
                ("preparation", "timed_out"),
                ("preparation", "reply_lost"),
                ("authority", "cancelled"),
                ("authority", "expired"),
                ("authority", "already_committed"),
                ("authority", "operation_in_flight"),
                ("post", "key_posted"),
                ("confirmation", "structurally_confirmed"),
                ("confirmation", "unconfirmed"),
                ("confirmation", "not_observed"),
                ("rollback", "restored"),
                ("rollback", "refused"),
            ] {
                let mut data = serde_json::json!({"event_code":"pipeline.paste_safety",
                    "phase_code":phase, "outcome_code":outcome,
                    "window_title":"synthetic-private-value", "transcript":"synthetic-private-value"});
                if phase == "preparation" {
                    data["route_code"] = "smart_target".into();
                }
                sanitize_event_data("pipeline", &mut data, debug_build);
                assert_eq!(
                    data.as_object().unwrap().len(),
                    if phase == "preparation" { 4 } else { 3 }
                );
                assert!(!serde_json::to_string(&data)
                    .unwrap()
                    .contains("synthetic-private-value"));
                assert_eq!(
                    sanitized_summary(
                        "pipeline",
                        Some("synthetic-private-value".into()),
                        &data,
                        debug_build
                    ),
                    "Paste safety decision"
                );
                data["outcome_code"] = "arbitrary-native-error".into();
                sanitize_event_data("pipeline", &mut data, debug_build);
                assert_eq!(data.as_object().unwrap().len(), 1);
            }
        }
    }

    #[test]
    fn paste_focus_proof_schema_keeps_only_bounded_diagnostics() {
        for debug_build in [true, false] {
            let mut data = serde_json::json!({"event_code":"pipeline.paste_safety",
                "phase_code":"focus_proof", "outcome_code":"frontmost_fallback_match",
                "live_code":"ax_error", "ax_status":-25204, "live_elapsed_us":20_150,
                "bundle_id":"synthetic-private-value"});
            sanitize_event_data("pipeline", &mut data, debug_build);
            assert_eq!(data.as_object().unwrap().len(), 6);
            assert!(!serde_json::to_string(&data)
                .unwrap()
                .contains("synthetic-private-value"));

            for (field, value) in [
                ("live_code", serde_json::json!("free text")),
                ("ax_status", serde_json::json!(12)),
                ("live_elapsed_us", serde_json::json!(10_000_001_u64)),
            ] {
                let mut data = serde_json::json!({"event_code":"pipeline.paste_safety",
                    "phase_code":"focus_proof", "outcome_code":"live_match",
                    "live_code":"answered", "ax_status":0, "live_elapsed_us":40});
                data[field] = value;
                sanitize_event_data("pipeline", &mut data, debug_build);
                assert_eq!(data.as_object().unwrap().len(), 1);
            }

            let mut data = serde_json::json!({"event_code":"pipeline.paste_safety",
                "phase_code":"focus_proof", "outcome_code":"identity_mismatch",
                "live_code":"not_attempted", "ax_status":0, "live_elapsed_us":0});
            sanitize_event_data("pipeline", &mut data, debug_build);
            assert_eq!(data.as_object().unwrap().len(), 6);

            let mut data = serde_json::json!({"event_code":"pipeline.paste_safety",
                "phase_code":"post", "outcome_code":"key_posted", "live_code":"answered"});
            sanitize_event_data("pipeline", &mut data, debug_build);
            assert_eq!(data.as_object().unwrap().len(), 3);
        }
    }

    #[test]
    fn smart_destination_delivery_schema_is_exact_and_content_free_in_every_build() {
        for debug_build in [true, false] {
            let mut data = serde_json::json!({
                "event_code": "pipeline.smart_destination_delivery",
                "recording_id": 12,
                "accepted_route": "normal_paste",
                "delivery_route": "normal_paste",
                "outcome_code": "auto_pasted",
                "bundle_id": "com.private.SENTINEL",
                "transcript": "SENTINEL_PRIVATE_TRANSCRIPT",
                "nested": {"content": "SENTINEL_PRIVATE_CONTENT"}
            });
            sanitize_event_data("pipeline", &mut data, debug_build);
            assert_eq!(
                data,
                serde_json::json!({
                    "event_code": "pipeline.smart_destination_delivery",
                    "recording_id": 12,
                    "accepted_route": "normal_paste",
                    "delivery_route": "normal_paste",
                    "outcome_code": "auto_pasted",
                }),
                "debug_build={debug_build}",
            );

            let mut invalid = serde_json::json!({
                "event_code": "pipeline.smart_destination_delivery",
                "recording_id": 12,
                "accepted_route": "SENTINEL_PRIVATE_ROUTE",
                "delivery_route": "normal_paste",
                "outcome_code": "auto_pasted",
            });
            sanitize_event_data("pipeline", &mut invalid, debug_build);
            assert_eq!(
                invalid,
                serde_json::json!({"event_code": "pipeline.smart_destination_delivery"}),
            );
        }
    }

    #[test]
    fn smart_destination_delivery_outcome_codes_cover_every_injection_outcome() {
        use crate::injector::{ClipboardOnlyReason, InjectionOutcome};
        let outcomes = [
            InjectionOutcome::NoText,
            InjectionOutcome::Stopped(crate::injector::authority::AttemptStop::Cancelled),
            InjectionOutcome::Stopped(crate::injector::authority::AttemptStop::Expired),
            InjectionOutcome::Stopped(crate::injector::authority::AttemptStop::AlreadyCommitted),
            InjectionOutcome::Stopped(crate::injector::authority::AttemptStop::OperationInFlight),
            InjectionOutcome::AutoPasted,
            InjectionOutcome::ClipboardOnly(ClipboardOnlyReason::AutoPasteDisabled),
            InjectionOutcome::ClipboardOnly(ClipboardOnlyReason::AccessibilityDenied),
            InjectionOutcome::ClipboardOnly(ClipboardOnlyReason::TargetChanged),
            InjectionOutcome::ClipboardOnly(ClipboardOnlyReason::FocusNotEditable),
            InjectionOutcome::ClipboardOnly(ClipboardOnlyReason::ClipboardChanged),
            InjectionOutcome::ClipboardOnly(ClipboardOnlyReason::PasteFailed),
            InjectionOutcome::ClipboardOnly(ClipboardOnlyReason::SmartTargetUnavailable),
        ];
        for outcome in outcomes {
            assert!(
                is_safe_smart_destination_delivery_field(
                    "outcome_code",
                    &serde_json::json!(outcome.code())
                ),
                "{outcome:?}",
            );
        }
        for route in ["disabled", "clipboard_only", "normal_paste", "smart_target"] {
            assert!(is_safe_smart_destination_delivery_field(
                "delivery_route",
                &serde_json::json!(route)
            ));
        }
    }

    #[test]
    fn delivery_target_verification_schema_is_exact_and_content_free_in_every_build() {
        for debug_build in [true, false] {
            let mut data = serde_json::json!({
                "event_code": "pipeline.delivery_target_verified",
                "recording_id": 574,
                "anchor_code": "stop",
                "stop_sample_code": "complete",
                "outcome_code": "different_process",
                "source_code": "native",
                "retry_count": 2,
                "elapsed_ms": 1_000,
                "same_application": true,
                "same_process": false,
                "same_process_instance": false,
                "window_relation_code": "different",
                "activation_changed": true,
                "space_changed": false,
                "current_is_self": false,
                "ownership_current": true,
                "app_name": "SENTINEL_PRIVATE_APPLICATION",
                "bundle_id": "com.private.SENTINEL",
                "process_id": 99_999,
                "window_title": "SENTINEL_PRIVATE_WINDOW",
                "transcript": "SENTINEL_PRIVATE_TRANSCRIPT",
                "clipboard": "SENTINEL_PRIVATE_CLIPBOARD",
                "path": "/Users/private/SENTINEL",
                "raw_error": "SENTINEL_PRIVATE_NATIVE_ERROR",
                "nested": {"content": "SENTINEL_PRIVATE_CONTENT"}
            });

            sanitize_event_data("pipeline", &mut data, debug_build);

            assert_eq!(data["event_code"], "pipeline.delivery_target_verified");
            assert_eq!(data["recording_id"], 574);
            assert_eq!(data["anchor_code"], "stop");
            assert_eq!(data["stop_sample_code"], "complete");
            assert_eq!(data["outcome_code"], "different_process");
            assert_eq!(data["source_code"], "native");
            assert_eq!(data["retry_count"], 2);
            assert_eq!(data["elapsed_ms"], 1_000);
            assert_eq!(data["same_application"], true);
            assert_eq!(data["same_process"], false);
            assert_eq!(data["same_process_instance"], false);
            assert_eq!(data["window_relation_code"], "different");
            assert_eq!(data["activation_changed"], true);
            assert_eq!(data["space_changed"], false);
            assert_eq!(data["current_is_self"], false);
            assert_eq!(data["ownership_current"], true);
            assert_eq!(data.as_object().unwrap().len(), 16);

            let encoded = serde_json::to_string(&data).unwrap();
            for sentinel in [
                "SENTINEL",
                "com.private",
                "/Users/private",
                "app_name",
                "bundle_id",
                "process_id",
                "window_title",
                "transcript",
                "clipboard",
                "path",
                "raw_error",
                "nested",
            ] {
                assert!(
                    !encoded.contains(sentinel),
                    "retained private field {sentinel}"
                );
            }

            assert_eq!(
                sanitized_summary(
                    "pipeline",
                    Some("SENTINEL_PRIVATE_APPLICATION /Users/private".to_string()),
                    &data,
                    debug_build,
                ),
                "Dictation delivery target verification"
            );
        }
    }

    #[test]
    fn delivery_target_verification_schema_accepts_only_bounded_vocabularies() {
        let base = serde_json::json!({
            "event_code": "pipeline.delivery_target_verified",
            "recording_id": 1,
            "anchor_code": "stop",
            "stop_sample_code": "complete",
            "outcome_code": "verified",
            "source_code": "native",
            "retry_count": 0,
            "elapsed_ms": 0,
            "same_application": true,
            "same_process": true,
            "same_process_instance": true,
            "window_relation_code": "same",
            "activation_changed": false,
            "space_changed": false,
            "current_is_self": false,
            "ownership_current": true
        });

        for (
            outcome,
            source,
            retry_count,
            same_application,
            same_process,
            same_instance,
            current_is_self,
            ownership_current,
        ) in [
            ("verified", "native", 0, true, true, true, false, true),
            (
                "different_application",
                "native",
                0,
                false,
                false,
                false,
                false,
                true,
            ),
            (
                "different_process",
                "native",
                0,
                true,
                false,
                false,
                false,
                true,
            ),
            (
                "process_relaunched",
                "native",
                0,
                true,
                true,
                false,
                false,
                true,
            ),
            (
                "partial_identity_mismatch",
                "native",
                0,
                false,
                false,
                false,
                false,
                true,
            ),
            (
                "partial_identity_mismatch",
                "native",
                0,
                false,
                true,
                false,
                false,
                true,
            ),
            (
                "partial_identity_mismatch",
                "native",
                1,
                false,
                true,
                false,
                false,
                true,
            ),
            (
                "lookup_unavailable",
                "none",
                2,
                false,
                false,
                false,
                false,
                true,
            ),
            (
                "start_identity_incomplete",
                "none",
                0,
                false,
                false,
                false,
                false,
                true,
            ),
            (
                "start_target_is_self",
                "native",
                0,
                false,
                false,
                false,
                true,
                true,
            ),
            ("stale_owner", "none", 0, false, false, false, false, false),
        ] {
            let mut data = base.clone();
            data["outcome_code"] = outcome.into();
            data["source_code"] = source.into();
            data["retry_count"] = retry_count.into();
            data["same_application"] = same_application.into();
            data["same_process"] = same_process.into();
            data["same_process_instance"] = same_instance.into();
            data["current_is_self"] = current_is_self.into();
            data["ownership_current"] = ownership_current.into();
            if source == "none"
                || matches!(
                    outcome,
                    "start_target_is_self" | "partial_identity_mismatch"
                )
            {
                data["window_relation_code"] = "unknown".into();
            }
            sanitize_event_data("pipeline", &mut data, true);
            assert_eq!(data.as_object().unwrap().len(), 16, "outcome {outcome}");
        }

        for relation in ["unknown", "same", "different"] {
            let mut data = base.clone();
            data["window_relation_code"] = relation.into();
            sanitize_event_data("pipeline", &mut data, true);
            assert_eq!(data.as_object().unwrap().len(), 16, "relation {relation}");
        }

        for (anchor, stop_sample) in [
            ("stop", "complete"),
            ("start", "self"),
            ("start", "incomplete"),
            ("start", "mismatch"),
            ("start", "absent"),
        ] {
            let mut data = base.clone();
            data["anchor_code"] = anchor.into();
            data["stop_sample_code"] = stop_sample.into();
            sanitize_event_data("pipeline", &mut data, true);
            assert_eq!(
                data.as_object().unwrap().len(),
                16,
                "anchor {anchor} stop sample {stop_sample}"
            );
        }

        // Only a complete stop sample may anchor delivery at stop.
        for (anchor, stop_sample) in [("stop", "self"), ("stop", "absent"), ("start", "complete")] {
            let mut data = base.clone();
            data["anchor_code"] = anchor.into();
            data["stop_sample_code"] = stop_sample.into();
            sanitize_event_data("pipeline", &mut data, true);
            assert_eq!(
                data.as_object().unwrap().len(),
                1,
                "anchor {anchor} stop sample {stop_sample}"
            );
        }
    }

    #[test]
    fn delivery_target_verification_schema_collapses_malformed_evidence() {
        let valid = serde_json::json!({
            "event_code": "pipeline.delivery_target_verified",
            "recording_id": 1,
            "anchor_code": "start",
            "stop_sample_code": "absent",
            "outcome_code": "lookup_unavailable",
            "source_code": "none",
            "retry_count": 2,
            "elapsed_ms": 1_000,
            "same_application": false,
            "same_process": false,
            "same_process_instance": false,
            "window_relation_code": "unknown",
            "activation_changed": false,
            "space_changed": false,
            "current_is_self": false,
            "ownership_current": true
        });

        let mut malformed = Vec::new();
        for (key, value) in [
            ("recording_id", serde_json::json!(0)),
            ("anchor_code", serde_json::json!("SENTINEL_PRIVATE_ANCHOR")),
            (
                "stop_sample_code",
                serde_json::json!("SENTINEL_PRIVATE_STOP_SAMPLE"),
            ),
            (
                "outcome_code",
                serde_json::json!("SENTINEL_PRIVATE_OUTCOME"),
            ),
            ("source_code", serde_json::json!("osascript:/Users/private")),
            ("retry_count", serde_json::json!(3)),
            ("elapsed_ms", serde_json::json!(1_001)),
            ("same_application", serde_json::json!("true")),
            ("same_process", serde_json::Value::Null),
            ("same_process_instance", serde_json::json!(1)),
            (
                "window_relation_code",
                serde_json::json!("SENTINEL_PRIVATE_WINDOW"),
            ),
            ("activation_changed", serde_json::json!([])),
            ("space_changed", serde_json::json!({})),
            ("current_is_self", serde_json::json!("false")),
            ("ownership_current", serde_json::json!(0)),
        ] {
            let mut data = valid.clone();
            data[key] = value;
            malformed.push(data);
        }
        let mut missing_required = valid;
        missing_required
            .as_object_mut()
            .unwrap()
            .remove("window_relation_code");
        malformed.push(missing_required);

        for mut data in malformed {
            sanitize_event_data("pipeline", &mut data, true);
            assert_eq!(data["event_code"], "pipeline.delivery_target_verified");
            assert_eq!(data.as_object().unwrap().len(), 1);
            let encoded = serde_json::to_string(&data).unwrap();
            assert!(!encoded.contains("SENTINEL"));
            assert!(!encoded.contains("Users"));
        }
    }

    #[test]
    fn delivery_target_verification_schema_rejects_misleading_cross_fields() {
        let event = |outcome,
                     source,
                     retry_count,
                     same_application,
                     same_process,
                     same_process_instance,
                     current_is_self,
                     ownership_current| {
            serde_json::json!({
                "event_code": "pipeline.delivery_target_verified",
                "recording_id": 9,
                "anchor_code": "stop",
                "stop_sample_code": "complete",
                "outcome_code": outcome,
                "source_code": source,
                "retry_count": retry_count,
                "elapsed_ms": 10,
                "same_application": same_application,
                "same_process": same_process,
                "same_process_instance": same_process_instance,
                "window_relation_code": if source == "native" && outcome != "start_target_is_self" {
                    "same"
                } else {
                    "unknown"
                },
                "activation_changed": false,
                "space_changed": false,
                "current_is_self": current_is_self,
                "ownership_current": ownership_current
            })
        };

        for mut data in [
            event("verified", "none", 0, true, true, true, false, true),
            event("verified", "native", 0, true, true, true, true, true),
            event("verified", "native", 0, true, true, true, false, false),
            event("verified", "native", 0, true, true, false, false, true),
            event(
                "different_process",
                "native",
                0,
                true,
                true,
                false,
                false,
                true,
            ),
            event(
                "different_process",
                "native",
                0,
                true,
                false,
                true,
                false,
                true,
            ),
            event(
                "process_relaunched",
                "native",
                0,
                true,
                false,
                false,
                false,
                true,
            ),
            event(
                "partial_identity_mismatch",
                "native",
                0,
                false,
                true,
                false,
                false,
                true,
            ),
            event(
                "lookup_unavailable",
                "none",
                0,
                false,
                false,
                false,
                false,
                true,
            ),
            event(
                "start_identity_incomplete",
                "native",
                0,
                false,
                false,
                false,
                false,
                true,
            ),
            event(
                "start_target_is_self",
                "native",
                0,
                false,
                false,
                false,
                false,
                true,
            ),
            event("stale_owner", "none", 0, false, false, false, false, true),
        ] {
            sanitize_event_data("pipeline", &mut data, true);
            assert_eq!(data["event_code"], "pipeline.delivery_target_verified");
            assert_eq!(data.as_object().unwrap().len(), 1);
        }
    }

    #[test]
    fn permission_probe_result_survives_sanitizing_and_stays_content_free() {
        // #638: the probe was invisible in app.log. Its terminal result must
        // survive the meeting sanitizer while carrying no device or user text.
        let mut data = serde_json::json!({
            "event_code": "meeting.permission_probe_finished",
            "tcc_authorized": true,
            "permission": "granted",
            "capture_ready": true,
            "audio_flowing": false,
            "needs_relaunch": false,
            "device_name": "SENTINEL_PRIVATE_DEVICE",
            "session_id": "private-session-id"
        });
        sanitize_event_data("meeting", &mut data, true);
        let encoded = serde_json::to_string(&data).unwrap();
        assert_eq!(data["event_code"], "meeting.permission_probe_finished");
        assert_eq!(data["permission"], "granted");
        assert_eq!(data["tcc_authorized"], true);
        assert_eq!(data["capture_ready"], true);
        assert_eq!(data["audio_flowing"], false);
        assert_eq!(data["needs_relaunch"], false);
        assert!(!encoded.contains("SENTINEL"));
        assert!(!encoded.contains("private-session"));

        for code in [
            "meeting.permission_probe_started",
            "meeting.permission_probe_failed",
        ] {
            let mut data = serde_json::json!({ "event_code": code });
            sanitize_event_data("meeting", &mut data, true);
            assert_eq!(data["event_code"], code);
        }

        // An unlisted permission value must not ride through.
        let mut spoofed = serde_json::json!({
            "event_code": "meeting.permission_probe_finished",
            "permission": "SENTINEL_PRIVATE_STATE"
        });
        sanitize_event_data("meeting", &mut spoofed, true);
        assert!(spoofed.get("permission").is_none());
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
    fn every_event_stream_strips_private_meeting_metadata_recursively() {
        const TITLE_SENTINEL: &str = "SENTINEL_PRIVATE_MEETING_TITLE";
        const ATTENDEE_SENTINEL: &str = "SENTINEL_PRIVATE_ATTENDEE";
        for stream in [
            "audio",
            "keyboard",
            "meeting",
            "pipeline",
            "query",
            "system",
            "transform",
        ] {
            for debug_build in [true, false] {
                let mut data = serde_json::json!({
                    "event_code": "meeting.capture_started",
                    "title": TITLE_SENTINEL,
                    "titleSource": "manual",
                    "attendees": [ATTENDEE_SENTINEL],
                    "nested": {
                        "title_source": "calendar",
                        "attendees": [ATTENDEE_SENTINEL]
                    }
                });
                sanitize_event_data(stream, &mut data, debug_build);
                let encoded = serde_json::to_string(&data).unwrap();
                assert!(!encoded.contains(TITLE_SENTINEL), "stream={stream}");
                assert!(!encoded.contains(ATTENDEE_SENTINEL), "stream={stream}");
                assert!(!encoded.contains("titleSource"), "stream={stream}");
                assert!(!encoded.contains("title_source"), "stream={stream}");
                assert!(!encoded.contains("attendees"), "stream={stream}");
            }
        }
    }

    #[test]
    fn calendar_fields_never_enter_any_event_stream() {
        for stream in [
            "audio",
            "keyboard",
            "meeting",
            "pipeline",
            "query",
            "system",
            "transform",
        ] {
            for debug_build in [true, false] {
                let mut data = serde_json::json!({
                    "calendarEvent": {"notes": "SECRET_CALENDAR_NOTE"},
                    "nested": [{
                        "eventIdentifier": "SECRET_EVENT_ID",
                        "calendar_identifier": "SECRET_CALENDAR_ID",
                        "selectionToken": "SECRET_SELECTION",
                        "calendarTitle": "SECRET_TITLE",
                        "calendarAttendees": ["SECRET_PERSON"],
                        "calendar_notes": "SECRET_NOTES",
                        "calendarLocation": "SECRET_ROOM",
                        "calendarUrl": "https://SECRET_CALL.invalid/"
                    }]
                });
                sanitize_event_data(stream, &mut data, debug_build);
                assert!(
                    !serde_json::to_string(&data).unwrap().contains("SECRET"),
                    "stream={stream}"
                );
            }
        }
    }

    #[test]
    fn suggestions_tokens_occurrences_and_payloads_never_enter_logs() {
        for stream in [
            "audio",
            "keyboard",
            "meeting",
            "pipeline",
            "query",
            "system",
            "transform",
        ] {
            for debug_build in [true, false] {
                let mut data = serde_json::json!({
                    "meetingSuggestion": {"title": "SECRET_EVENT"},
                    "suggestion_token": "SECRET_TOKEN", "occurrenceKey": "SECRET_OCCURRENCE",
                    "nested": [{"token": "SECRET_TOKEN", "title": "SECRET_TITLE", "attendees": ["SECRET_PERSON"]}]
                });
                sanitize_event_data(stream, &mut data, debug_build);
                assert!(!serde_json::to_string(&data).unwrap().contains("SECRET"));
            }
        }
    }

    #[test]
    fn echo_cancellation_transition_keeps_only_content_free_recovery_evidence() {
        for debug_build in [true, false] {
            let mut data = serde_json::json!({
                "event_code": "meeting.echo_cancellation_state_changed",
                "generation": 12,
                "from": "active",
                "to": "recovering",
                "reason": "render_discontinuity",
                "recovery_episode": 4,
                "recovery_attempt": 1,
                "recovery_max_attempts": 3,
                "transcript": "SENTINEL_PRIVATE_TRANSCRIPT",
                "device_name": "SENTINEL_PRIVATE_DEVICE",
                "session_id": "SENTINEL_PRIVATE_SESSION",
                "audio_path": "/Users/private/SENTINEL.wav"
            });
            sanitize_event_data("meeting", &mut data, debug_build);
            let encoded = serde_json::to_string(&data).unwrap();
            assert_eq!(
                data["event_code"],
                "meeting.echo_cancellation_state_changed"
            );
            assert_eq!(data["from"], "active");
            assert_eq!(data["to"], "recovering");
            assert_eq!(data["reason"], "render_discontinuity");
            assert_eq!(data["recovery_episode"], 4);
            assert_eq!(data["recovery_attempt"], 1);
            assert_eq!(data["recovery_max_attempts"], 3);
            assert!(!encoded.contains("SENTINEL"));
            assert!(!encoded.contains("/Users/private"));
        }
    }

    #[test]
    fn homophone_events_keep_only_content_free_fields_in_all_builds() {
        for (debug_build, outcome) in [true, false].into_iter().flat_map(|debug| {
            ["unavailable", "not_triggered", "backoff"].map(|outcome| (debug, outcome))
        }) {
            for code in [
                "homophone_context.pass",
                "homophone_context.backoff_entered",
                "homophone_context.backoff_exited",
                "homophone_context.prewarm_requested",
            ] {
                let mut data = serde_json::json!({
                    "event_code": code, "session_id": 7, "flagged": 1,
                    "engine_us": 151_000, "budget_ms": 150, "outcome": outcome,
                    "consecutive_failures": 3, "cooldown_ms": 300_000, "scheduled": true,
                    "prompt": "SENTINEL PRIVATE", "answer": {"nested": "SENTINEL"},
                    "transcript": "SENTINEL PRIVATE", "error": "/Users/private/path"
                });
                sanitize_event_data("pipeline", &mut data, debug_build);
                assert_eq!(data["event_code"], code);
                assert_eq!(data["outcome"], outcome);
                assert_eq!(data["engine_us"], 151_000);
                assert_eq!(
                    sanitized_summary(
                        "pipeline",
                        Some("SENTINEL PRIVATE".into()),
                        &data,
                        debug_build
                    ),
                    code
                );
                let encoded = data.to_string();
                assert!(!encoded.contains("SENTINEL"));
                assert!(!encoded.contains("/Users/private"));
                data["outcome"] = serde_json::json!("SENTINEL PRIVATE");
                data["engine_us"] = serde_json::json!({"nested": "SENTINEL"});
                sanitize_event_data("pipeline", &mut data, debug_build);
                assert!(data.get("outcome").is_none());
                assert!(data.get("engine_us").is_none());
            }
        }
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

    #[test]
    fn release_dictation_completion_keeps_aggregator_contract() {
        let mut data = serde_json::json!({
            "event_code": "pipeline.dictation_completed",
            "recording_id": 41,
            "char_count": 18,
            "total_ms": 220,
            "model": "PRIVATE_MODEL"
        });

        sanitize_event_data("pipeline", &mut data, false);
        let summary = sanitized_summary(
            "pipeline",
            Some("dictation processing metrics".to_string()),
            &data,
            false,
        );

        assert_eq!(summary, "pipeline.dictation_completed");
        assert_eq!(data["event_code"], "pipeline.dictation_completed");
        assert_eq!(data["recording_id"], 41);
        assert_eq!(data["char_count"], 18);
        assert_eq!(data["total_ms"], 220);
        assert!(data.get("model").is_none());
    }

    #[test]
    fn release_partial_interruption_keeps_neutral_metrics_contract() {
        let mut data = serde_json::json!({
            "event_code": "pipeline.dictation_metrics",
            "recording_id": 41,
            "char_count": 18,
            "total_ms": 220,
            "model": "PRIVATE_MODEL"
        });

        sanitize_event_data("pipeline", &mut data, false);
        let summary = sanitized_summary(
            "pipeline",
            Some("dictation processing metrics".to_string()),
            &data,
            false,
        );

        assert_eq!(summary, "pipeline.dictation_metrics");
        assert_eq!(data["event_code"], "pipeline.dictation_metrics");
        assert_eq!(data["recording_id"], 41);
        assert_eq!(data["char_count"], 18);
        assert_eq!(data["total_ms"], 220);
        assert!(data.get("model").is_none());
    }

    #[test]
    fn model_install_terminal_schema_is_content_free_in_every_build() {
        let mut data = serde_json::json!({
            "event_code": "system.model_install_terminal",
            "attempt_id": 17,
            "install_kind": "coreml",
            "outcome_code": "installer_timeout",
            "repaired_cache": true,
            "repeated_repair": true,
            "termination_confirmed": true,
            "raw_error": "SENTINEL /Users/private/FluidAudio"
        });

        sanitize_event_data("system", &mut data, true);

        assert_eq!(data["attempt_id"], 17);
        assert_eq!(data["outcome_code"], "installer_timeout");
        let encoded = serde_json::to_string(&data).unwrap();
        assert!(!encoded.contains("SENTINEL"));
        assert!(!encoded.contains("/Users/private"));
        assert_eq!(
            sanitized_summary(
                "system",
                Some("SENTINEL native error".to_string()),
                &data,
                true,
            ),
            "Model installation event"
        );
    }

    #[test]
    fn model_install_schema_rejects_contradictory_repair_or_termination_evidence() {
        for mut data in [
            serde_json::json!({
                "event_code": "system.model_install_phase",
                "attempt_id": 2,
                "install_kind": "coreml",
                "phase": "initializing",
                "repeated_repair": true,
            }),
            serde_json::json!({
                "event_code": "system.model_install_terminal",
                "attempt_id": 3,
                "install_kind": "coreml",
                "outcome_code": "success",
                "repaired_cache": false,
                "repeated_repair": true,
                "termination_confirmed": false,
            }),
        ] {
            sanitize_event_data("system", &mut data, false);
            assert_eq!(
                data,
                serde_json::json!({ "event_code": data["event_code"].clone() })
            );
        }
    }
    #[test]
    fn launch_milestones_keep_only_elapsed_time_and_fixed_codes() {
        for debug in [false, true] {
            for code in [
                "system.launch_process_start",
                "system.launch_native_setup_done",
                "system.launch_listener_armed",
                "system.launch_model_ready",
                "system.launch_frontend_hydrated",
            ] {
                let mut data = serde_json::json!({"event_code":code,"launch_elapsed_ms":125,
                    "transcript":"PRIVATE_SENTINEL", "model":"PRIVATE_SENTINEL", "recording_id":22});
                sanitize_event_data("system", &mut data, debug);
                assert_eq!(
                    data,
                    serde_json::json!({"event_code":code,"launch_elapsed_ms":125})
                );
                assert_eq!(
                    sanitized_summary("system", Some("PRIVATE_SENTINEL".into()), &data, debug),
                    "Launch milestone"
                );
            }
        }
    }
}
