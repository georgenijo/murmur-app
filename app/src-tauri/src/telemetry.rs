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
        "audio.system_audio_graph_observed" => Some("audio.system_audio_graph_observed"),
        "audio.device_reresolution_started" => Some("audio.device_reresolution_started"),
        "audio.input_resolution_observed" => Some("audio.input_resolution_observed"),
        "audio.capture_started" => Some("audio.capture_started"),
        "audio.fallback_started" => Some("audio.fallback_started"),
        "audio.capture_ready" => Some("audio.capture_ready"),
        "audio.capture_failed" => Some("audio.capture_failed"),
        "audio.lifecycle_failed" => Some("audio.lifecycle_failed"),
        "audio.permission_prompt_changed" => Some("audio.permission_prompt_changed"),
        "pipeline.dictation_requested" => Some("pipeline.dictation_requested"),
        "pipeline.dictation_state_changed" => Some("pipeline.dictation_state_changed"),
        "pipeline.dictation_presentation" => Some("pipeline.dictation_presentation"),
        "pipeline.dictation_stop_handoff" => Some("pipeline.dictation_stop_handoff"),
        "pipeline.dictation_terminal" => Some("pipeline.dictation_terminal"),
        "pipeline.dictation_completed" => Some("pipeline.dictation_completed"),
        "pipeline.dictation_failed" => Some("pipeline.dictation_failed"),
        "pipeline.dictation_partial_tick" => Some("pipeline.dictation_partial_tick"),
        "pipeline.dictation_preview_presentation" => {
            Some("pipeline.dictation_preview_presentation")
        }
        "pipeline.delivery_target_verified" => Some("pipeline.delivery_target_verified"),
        "performance.store_operation_failed" => Some("performance.store_operation_failed"),
        "system.startup_baseline" => Some("system.startup_baseline"),
        "system.model_install_started" => Some("system.model_install_started"),
        "system.model_install_phase" => Some("system.model_install_phase"),
        "system.model_install_terminal" => Some("system.model_install_terminal"),
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
            )
        }),
        _ => false,
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
        _ => false,
    });
}

fn is_safe_audio_owner_kind(value: &str) -> bool {
    matches!(
        value,
        "dictation" | "transform" | "query" | "preview" | "microphone_benchmark" | "corpus"
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
    const REQUIRED_FIELDS: [&str; 15] = [
        "event_code",
        "recording_id",
        "anchor_code",
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
    if data.get("event_code").and_then(serde_json::Value::as_str)
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
    } else if data.get("event_code").and_then(serde_json::Value::as_str)
        == Some("pipeline.delivery_target_verified")
    {
        "Dictation delivery target verification".to_string()
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
    if is_dictation_partial_tick_event(obj) {
        sanitize_dictation_partial_tick_event(obj);
        return;
    }
    if is_dictation_preview_presentation_event(obj) {
        sanitize_dictation_preview_presentation_event(obj);
        return;
    }
    if is_dictation_terminal_event(obj) {
        // Terminal lifecycle records share the event file with Fleet uploads.
        // Keep the exact content-free schema in debug and release builds.
        sanitize_dictation_terminal_event(obj);
        return;
    }
    if is_delivery_target_verification_event(obj) {
        // Delivery identity is privacy-sensitive even in debug builds. Keep
        // only bounded equality facts and stable outcome/source codes so app,
        // process, window, path, content, and native error detail cannot leak.
        sanitize_delivery_target_verification_event(obj);
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
                "owner_kind": "query",
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
    fn dictation_partial_tick_schema_is_exact_and_content_free_in_every_build() {
        for debug_build in [true, false] {
            for outcome in ["emitted", "emit_failed"] {
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
                assert!(!serde_json::to_string(&data).unwrap().contains("SENTINEL"));
                assert!(!serde_json::to_string(&data).unwrap().contains("private"));
            }
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
    fn delivery_target_verification_schema_is_exact_and_content_free_in_every_build() {
        for debug_build in [true, false] {
            let mut data = serde_json::json!({
                "event_code": "pipeline.delivery_target_verified",
                "recording_id": 574,
                "anchor_code": "stop",
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
            assert_eq!(data.as_object().unwrap().len(), 15);

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
            assert_eq!(data.as_object().unwrap().len(), 15, "outcome {outcome}");
        }

        for relation in ["unknown", "same", "different"] {
            let mut data = base.clone();
            data["window_relation_code"] = relation.into();
            sanitize_event_data("pipeline", &mut data, true);
            assert_eq!(data.as_object().unwrap().len(), 15, "relation {relation}");
        }

        for anchor in ["stop", "start"] {
            let mut data = base.clone();
            data["anchor_code"] = anchor.into();
            sanitize_event_data("pipeline", &mut data, true);
            assert_eq!(data.as_object().unwrap().len(), 15, "anchor {anchor}");
        }
    }

    #[test]
    fn delivery_target_verification_schema_collapses_malformed_evidence() {
        let valid = serde_json::json!({
            "event_code": "pipeline.delivery_target_verified",
            "recording_id": 1,
            "anchor_code": "start",
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
}
