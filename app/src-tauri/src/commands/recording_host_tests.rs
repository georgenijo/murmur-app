//! Seam-contract tests for the Stage 0a `DictationHost` (plan §6, C9.1 C5).
//!
//! A `FakeHost` stands in for `TauriHost`, logs every port call in the order
//! the command reaches it, and returns scripted results. Each scenario asserts
//! the exact log, which is the order the production code reaches those ports
//! (read from the command bodies, not from this fake). A scenario is
//! single-task: the order is the program order of the awaits, and the one
//! scenario that hops threads (the kept-preview start) is ordered by awaiting
//! the `spawn_blocking` join. A scripted hook runs inside a port at the point
//! production reaches it, so a test can change shared state "between" two ports
//! with no timing dependence.
//!
//! `every_port_is_reached_in_two_scenarios` fails when a port is added to the
//! trait but only one scenario (or none) reaches it.

use super::dictation_host::{
    HostEmitError, HostFuture, KeyboardPort, PerformancePort, SmartDestinationPort, SupervisorPort,
    TargetSnapshotPort,
};
use super::tests::test_state;
use super::*;
use std::sync::Mutex as StdMutex;

type Hook = Box<dyn Fn(&State) + Send>;

#[derive(Default)]
struct Script {
    app_disabled: bool,
    start_ack: Option<AudioStartError>,
    stop_result: Option<Result<Vec<f32>, String>>,
    cancel_result: Option<Result<bool, String>>,
    interrupted: Option<audio_lifecycle::InterruptedCapture>,
    pipeline_error: Option<String>,
    on_emit_smart_destination: Option<Hook>,
    on_supervisor_start: Option<Hook>,
}

struct Shared {
    log: StdMutex<Vec<String>>,
    state: Arc<State>,
    script: StdMutex<Script>,
}

impl Shared {
    fn push(&self, entry: impl Into<String>) {
        self.log
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(entry.into());
    }

    fn script(&self) -> std::sync::MutexGuard<'_, Script> {
        self.script
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

struct FakeHost {
    shared: Arc<Shared>,
}

impl FakeHost {
    fn new(state: Arc<State>) -> Self {
        Self {
            shared: Arc::new(Shared {
                log: StdMutex::new(Vec::new()),
                state,
                script: StdMutex::new(Script::default()),
            }),
        }
    }

    fn script(&self) -> std::sync::MutexGuard<'_, Script> {
        self.shared.script()
    }

    fn log(&self) -> Vec<String> {
        self.shared
            .log
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

impl DictationHost for FakeHost {
    fn emit_json(&self, event: &str, payload: serde_json::Value) -> Result<(), HostEmitError> {
        self.shared.push(format!("emit {event} {payload}"));
        Ok(())
    }

    fn supervisor(&self) -> Arc<dyn SupervisorPort> {
        Arc::clone(&self.shared) as Arc<dyn SupervisorPort>
    }

    fn keyboard(&self) -> &dyn KeyboardPort {
        &*self.shared
    }

    fn ax(&self) -> &dyn TargetSnapshotPort {
        &*self.shared
    }

    fn smart_destination(&self) -> &dyn SmartDestinationPort {
        &*self.shared
    }

    fn performance(&self) -> &dyn PerformancePort {
        &*self.shared
    }

    fn microphone_transition<'a>(
        &'a self,
        state: &'a State,
    ) -> HostFuture<'a, Result<tokio::sync::MutexGuard<'a, ()>, String>> {
        self.shared.push("mic_transition");
        Box::pin(async move { Ok(state.app_state.recording_transition.lock().await) })
    }

    fn refresh_expired_ide_context(&self, _app_state: &AppState, _bundle_id: Option<&str>) {
        self.shared.push("refresh_expired_ide_context");
    }

    fn emit_smart_destination(
        &self,
        _app_state: &AppState,
        recording_id: u64,
        _context: &DictationContextSnapshot,
        _accepted_target: &crate::frontmost::DeliveryTargetSnapshot,
    ) {
        self.shared
            .push(format!("emit_smart_destination {recording_id}"));
        if let Some(hook) = self.shared.script().on_emit_smart_destination.as_ref() {
            hook(&self.shared.state);
        }
    }

    fn publish_recording_ready_if_audio_ready(
        &self,
        recording_id: u64,
        _context_ready_at: std::time::Instant,
    ) -> bool {
        self.shared
            .push(format!("publish_recording_ready {recording_id}"));
        false
    }

    fn spawn_model_preparation(&self, _model_name: String, recording_id: u64) {
        self.shared
            .push(format!("spawn_model_preparation {recording_id}"));
    }

    fn claim_preview_hide(
        &self,
        recording_id: u64,
    ) -> Option<crate::commands::dictation_preview::PreviewHideOwner> {
        self.shared
            .push(format!("claim_preview_hide {recording_id}"));
        None
    }

    fn run_transcription_pipeline<'a>(
        &'a self,
        samples: &'a [f32],
        _app_state: &'a AppState,
        recording_id: u64,
        _performance_guard: &'a mut NativeDictationPerformanceGuard,
        _context: Arc<DictationContextSnapshot>,
        stop: PipelineStopContext,
    ) -> HostFuture<'a, Result<PipelineResult, String>> {
        self.shared
            .push(format!("pipeline {recording_id} samples={}", samples.len()));
        let error = self
            .shared
            .script()
            .pipeline_error
            .take()
            .expect("the scenario scripts the pipeline result");
        Box::pin(async move {
            drop(stop);
            Err(error)
        })
    }
}

impl SupervisorPort for Shared {
    fn start_dictation_recording(
        &self,
        _device_id: Option<String>,
        recording_id: u64,
        origin: &str,
        _gesture: audio_lifecycle::gesture::StartGesture,
    ) -> Result<(), AudioStartError> {
        let status = self.state.app_state.dictation.lock_or_recover().status;
        self.push(format!(
            "supervisor.start {recording_id} {origin} status={status:?}"
        ));
        self.scripted_start_ack()
    }

    fn start_dictation_recording_after_preview(
        &self,
        _device_id: Option<String>,
        recording_id: u64,
        origin: &str,
        _gesture: audio_lifecycle::gesture::StartGesture,
    ) -> Result<(), AudioStartError> {
        self.push(format!(
            "supervisor.start_after_preview {recording_id} {origin}"
        ));
        self.scripted_start_ack()
    }

    fn stop_dictation_recording(
        &self,
        recording_id: u64,
        trigger: audio_lifecycle::DictationStopTrigger,
        _released_at: Option<std::time::Instant>,
    ) -> Result<Vec<f32>, String> {
        self.push(format!("supervisor.stop {recording_id} {trigger:?}"));
        self.script()
            .stop_result
            .take()
            .expect("the scenario scripts the stop result")
    }

    fn cancel_dictation_capture(
        &self,
        recording_id: u64,
        reason: AudioCancelReason,
    ) -> Result<bool, String> {
        self.push(format!("supervisor.cancel {recording_id} {reason:?}"));
        self.script()
            .cancel_result
            .take()
            .expect("the scenario scripts the cancel result")
    }

    fn take_interrupted_dictation(
        &self,
        recording_id: u64,
    ) -> Option<audio_lifecycle::InterruptedCapture> {
        self.push(format!("supervisor.take_interrupted {recording_id}"));
        self.script().interrupted.take()
    }
}

impl Shared {
    fn scripted_start_ack(&self) -> Result<(), AudioStartError> {
        if let Some(hook) = self.script().on_supervisor_start.as_ref() {
            hook(&self.state);
        }
        match self.script().start_ack.take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl KeyboardPort for Shared {
    fn is_app_disabled(&self) -> bool {
        self.push("keyboard.is_app_disabled");
        self.script().app_disabled
    }

    fn listener_generation(&self) -> u64 {
        self.push("keyboard.listener_generation");
        0
    }

    fn set_processing(&self, processing: bool) {
        self.push(format!("keyboard.set_processing {processing}"));
    }

    fn hotkey_release_take_for(
        &self,
        recording_id: u64,
        _stop_entered_at: std::time::Instant,
    ) -> Option<crate::hotkey_release::HotkeyRelease> {
        self.push(format!("keyboard.hotkey_release_take_for {recording_id}"));
        None
    }

    fn stop_gesture_take_for(
        &self,
        recording_id: u64,
        _entered: std::time::Instant,
    ) -> Option<crate::stop_gesture::StopStamp> {
        self.push(format!("keyboard.stop_gesture_take_for {recording_id}"));
        None
    }
}

impl TargetSnapshotPort for Shared {
    fn capture_delivery_target_snapshot(&self) -> crate::frontmost::DeliveryTargetSnapshot {
        self.push("ax.capture_delivery_target_snapshot");
        crate::frontmost::DeliveryTargetSnapshot::Incomplete
    }
}

impl SmartDestinationPort for Shared {
    fn capture<'a>(
        &'a self,
        _expected: &'a crate::frontmost::DeliveryTargetSnapshot,
        enabled: bool,
    ) -> HostFuture<'a, crate::smart_destination::Snapshot> {
        self.push(format!("smart_destination.capture enabled={enabled}"));
        let snapshot = if enabled {
            crate::smart_destination::Snapshot::Unavailable
        } else {
            crate::smart_destination::Snapshot::Disabled
        };
        Box::pin(async move { snapshot })
    }
}

impl PerformancePort for Shared {
    fn register_production(
        &self,
        recording_id: u64,
        selection: crate::performance_metrics::production::MicrophoneSelectionV1,
        _kind: Option<crate::performance_metrics::production::MicrophoneKindV1>,
    ) {
        self.push(format!(
            "performance.register_production {recording_id} {selection:?}"
        ));
    }

    fn production_context(&self, recording_id: u64, _context: &DictationContextSnapshot) {
        self.push(format!("performance.production_context {recording_id}"));
    }

    fn production_sample_count(&self, recording_id: u64, count: usize) {
        self.push(format!(
            "performance.production_sample_count {recording_id} {count}"
        ));
    }

    fn begin_register(&self, recording_id: u64) {
        self.push(format!("performance.begin_register {recording_id}"));
    }

    fn begin_abort(&self, recording_id: u64) {
        self.push(format!("performance.begin_abort {recording_id}"));
    }

    fn spawn_begin(
        &self,
        recording_id: u64,
        _runtimes: Vec<RuntimeIdentityV1>,
        _started_at_ms: i64,
    ) {
        self.push(format!("performance.spawn_begin {recording_id}"));
    }

    fn complete(
        &self,
        recording_id: u64,
        outcome: RunOutcomeV1,
        _stages: Vec<StageTimingV1>,
        _input: Option<ContentFreeInputSummaryV1>,
        _runtimes: Option<Vec<RuntimeIdentityV1>>,
    ) {
        self.push(format!(
            "performance.complete {recording_id} {}",
            serde_json::to_string(&outcome).expect("outcome serializes")
        ));
    }
}

fn fixture(status: DictationStatus, recording_id: u64) -> (Arc<State>, FakeHost) {
    let app_state = AppState::default();
    app_state.recording_id.store(recording_id, Ordering::SeqCst);
    app_state.dictation.lock_or_recover().status = status;
    let state = Arc::new(test_state(app_state));
    let host = FakeHost::new(Arc::clone(&state));
    (state, host)
}

fn install_context(state: &State, recording_id: u64) {
    let destination = select_recording_destination(
        &crate::frontmost::DeliveryTargetSnapshot::Incomplete,
        crate::smart_destination::Snapshot::Disabled,
    );
    state.app_state.set_active_context(
        recording_id,
        resolve_recording_context(&state.app_state, destination),
    );
}

fn enable_smart_destination(state: &State) {
    let mut dictation = state.app_state.dictation.lock_or_recover();
    dictation.smart_destination_enabled = true;
    dictation.auto_paste = true;
    dictation.save_audio = false;
    dictation.save_transcript = false;
}

fn status(state: &State) -> DictationStatus {
    state.app_state.dictation.lock_or_recover().status
}

fn run<T>(future: impl std::future::Future<Output = T>) -> T {
    tauri::async_runtime::block_on(future)
}

fn start(host: &FakeHost, state: &State) -> Result<serde_json::Value, String> {
    run(start_native_recording_inner(
        host, state, None, None, None, None,
    ))
}

fn stop(
    host: &FakeHost,
    state: &State,
    expected: Option<u64>,
) -> Result<serde_json::Value, String> {
    run(stop_native_recording_inner(
        host,
        state,
        expected,
        audio_lifecycle::DictationStopTrigger::Other,
    ))
}

fn cancel(host: &FakeHost, state: &State, expected: Option<u64>) -> Result<(), String> {
    run(cancel_native_recording_inner(host, state, expected))
}

fn strings(entries: &[&str]) -> Vec<String> {
    entries.iter().map(|entry| entry.to_string()).collect()
}

const STATUS_EVENT: &str = "recording-status-changed";

// ------------------------------------------------------------------ start

/// The start reaches the supervisor (CS) with Starting committed, gets the
/// scripted ack, and finishes the context install: every start-side port, in
/// the order of `start_native_recording_inner`.
fn scenario_start_acknowledged() -> (Vec<String>, serde_json::Value, DictationStatus) {
    let (state, host) = fixture(DictationStatus::Idle, 0);
    let reply = start(&host, &state).expect("start replies");
    (host.log(), reply, status(&state))
}

#[test]
fn start_reaches_the_supervisor_then_installs_context_in_production_order() {
    let (log, reply, status) = scenario_start_acknowledged();
    assert_eq!(
        log,
        strings(&[
            "mic_transition",
            "keyboard.is_app_disabled",
            "ax.capture_delivery_target_snapshot",
            "emit dictation-generation-started {\"recordingId\":1}",
            &format!("emit {STATUS_EVENT} \"starting\""),
            "performance.register_production 1 SystemDefault",
            "performance.begin_register 1",
            "supervisor.start 1 toggle status=Starting",
            "smart_destination.capture enabled=false",
            "refresh_expired_ide_context",
            "performance.production_context 1",
            "publish_recording_ready 1",
            "spawn_model_preparation 1",
            "performance.spawn_begin 1",
        ])
    );
    assert_eq!(
        reply,
        serde_json::json!({
            "type": "recording_starting", "state": "starting", "recordingId": 1
        })
    );
    assert_eq!(status, DictationStatus::Starting);
}

#[test]
fn start_refused_by_the_supervisor_aborts_the_begin_and_restores_idle() {
    let (state, host) = fixture(DictationStatus::Idle, 0);
    host.script().start_ack = Some(AudioStartError::AlreadyStarting);
    let reply = start(&host, &state).expect("start replies");
    assert_eq!(
        host.log(),
        strings(&[
            "mic_transition",
            "keyboard.is_app_disabled",
            "ax.capture_delivery_target_snapshot",
            "emit dictation-generation-started {\"recordingId\":1}",
            &format!("emit {STATUS_EVENT} \"starting\""),
            "performance.register_production 1 SystemDefault",
            "performance.begin_register 1",
            "supervisor.start 1 toggle status=Starting",
            "performance.begin_abort 1",
            &format!("emit {STATUS_EVENT} \"idle\""),
        ])
    );
    assert_eq!(
        reply,
        serde_json::json!({ "type": "already_starting", "state": "idle" })
    );
    assert_eq!(status(&state), DictationStatus::Idle);
}

#[test]
fn start_while_the_app_is_disabled_stops_after_the_transition_and_keyboard_read() {
    let (state, host) = fixture(DictationStatus::Idle, 0);
    host.script().app_disabled = true;
    let reply = start(&host, &state).expect("start replies");
    assert_eq!(
        host.log(),
        strings(&["mic_transition", "keyboard.is_app_disabled"])
    );
    assert_eq!(
        reply,
        serde_json::json!({ "type": "app_disabled", "state": "idle" })
    );
}

/// A kept Settings mic test sends the start through the supervisor's
/// after-preview entry on a blocking thread. The join is the causal barrier:
/// everything after it is ordered behind the supervisor call.
#[test]
fn start_with_a_kept_preview_uses_the_after_preview_supervisor_entry() {
    let (state, host) = fixture(DictationStatus::Idle, 0);
    state
        .app_state
        .microphone_preview
        .claim(50)
        .expect("a mic test is kept");
    host.script().start_ack = Some(AudioStartError::AudioRecovering);
    let reply = start(&host, &state).expect("start replies");
    assert_eq!(
        host.log(),
        strings(&[
            "mic_transition",
            "keyboard.is_app_disabled",
            "ax.capture_delivery_target_snapshot",
            "emit dictation-generation-started {\"recordingId\":1}",
            &format!("emit {STATUS_EVENT} \"starting\""),
            "performance.register_production 1 SystemDefault",
            "performance.begin_register 1",
            "supervisor.start_after_preview 1 toggle",
            "performance.begin_abort 1",
            &format!("emit {STATUS_EVENT} \"idle\""),
        ])
    );
    assert_eq!(
        reply,
        serde_json::json!({ "type": "audio_recovering", "state": "idle" })
    );
}

/// With Smart Destination on, the capture is asked for with `enabled=true`
/// and the selection is emitted before the context is recorded.
#[test]
fn start_with_smart_destination_captures_and_emits_the_selection() {
    let (state, host) = fixture(DictationStatus::Idle, 0);
    enable_smart_destination(&state);
    start(&host, &state).expect("start replies");
    let log = host.log();
    let tail: Vec<&str> = log
        .iter()
        .skip_while(|entry| !entry.starts_with("supervisor.start "))
        .map(String::as_str)
        .collect();
    assert_eq!(
        tail,
        vec![
            "supervisor.start 1 toggle status=Starting",
            "smart_destination.capture enabled=true",
            "refresh_expired_ide_context",
            "emit_smart_destination 1",
            "performance.production_context 1",
            "publish_recording_ready 1",
            "spawn_model_preparation 1",
            "performance.spawn_begin 1",
        ]
    );
}

/// The supervisor's own `Idle` can land while the context resolves. The start
/// then flushes the reserved diagnostics instead of publishing Recording, so
/// the model is never prepared and no ready is published.
#[test]
fn start_whose_capture_ended_during_context_flushes_diagnostics_only() {
    let (state, host) = fixture(DictationStatus::Idle, 0);
    enable_smart_destination(&state);
    host.script().on_emit_smart_destination = Some(Box::new(|state| {
        state.app_state.dictation.lock_or_recover().status = DictationStatus::Idle;
    }));
    let reply = start(&host, &state).expect("start replies");
    let log = host.log();
    let tail: Vec<&str> = log
        .iter()
        .skip_while(|entry| !entry.starts_with("emit_smart_destination"))
        .map(String::as_str)
        .collect();
    assert_eq!(
        tail,
        vec![
            "emit_smart_destination 1",
            "performance.production_context 1",
            "performance.spawn_begin 1",
        ]
    );
    assert_eq!(
        reply,
        serde_json::json!({ "type": "audio_recovering", "state": "idle" })
    );
}

/// A newer generation can take over while the context resolves: the start
/// aborts its begin and publishes nothing further.
#[test]
fn start_superseded_during_context_aborts_the_begin() {
    let (state, host) = fixture(DictationStatus::Idle, 0);
    host.script().on_supervisor_start = Some(Box::new(|state| {
        state.app_state.recording_id.store(9, Ordering::SeqCst);
    }));
    let reply = start(&host, &state).expect("start replies");
    let log = host.log();
    let tail: Vec<&str> = log
        .iter()
        .skip_while(|entry| !entry.starts_with("supervisor.start "))
        .map(String::as_str)
        .collect();
    assert_eq!(
        tail,
        vec![
            "supervisor.start 1 toggle status=Starting",
            "smart_destination.capture enabled=false",
            "refresh_expired_ide_context",
            "performance.production_context 1",
            "performance.begin_abort 1",
        ]
    );
    assert_eq!(
        reply,
        serde_json::json!({ "type": "audio_recovering", "state": "starting" })
    );
}

/// A stamped hold gesture reads the keyboard listener generation while the
/// start is accepted, before the delivery target is sampled.
#[test]
fn start_from_a_stamped_hold_reads_the_listener_generation_before_the_ax_sample() {
    let (state, host) = fixture(DictationStatus::Idle, 0);
    let token = audio_lifecycle::gesture::emit(audio_lifecycle::gesture::Candidate::new(), true);
    let reply = run(start_native_recording_inner(
        &host,
        &state,
        None,
        None,
        Some("hold".to_string()),
        Some(token),
    ))
    .expect("start replies");
    audio_lifecycle::gesture::clear_pending();
    let log = host.log();
    assert_eq!(
        &log[..4],
        &strings(&[
            "mic_transition",
            "keyboard.is_app_disabled",
            "keyboard.listener_generation",
            "ax.capture_delivery_target_snapshot",
        ])[..]
    );
    assert!(log.contains(&"supervisor.start 1 hold status=Starting".to_string()));
    assert_eq!(reply["type"], "recording_starting");
}

// ------------------------------------------------------------------- stop

fn scenario_stop_missing_context() -> Vec<String> {
    let (state, host) = fixture(DictationStatus::Recording, 7);
    let error = stop(&host, &state, None).expect_err("no context installed");
    assert_eq!(error, "Missing dictation context for recording 7");
    assert_eq!(status(&state), DictationStatus::Idle);
    host.log()
}

#[test]
fn stop_without_a_context_claims_the_hide_then_unwinds_in_order() {
    assert_eq!(
        scenario_stop_missing_context(),
        strings(&[
            "claim_preview_hide 7",
            "ax.capture_delivery_target_snapshot",
            "keyboard.hotkey_release_take_for 7",
            "keyboard.stop_gesture_take_for 7",
            "keyboard.set_processing false",
            &format!("emit {STATUS_EVENT} \"idle\""),
            "performance.complete 7 {\"status\":\"failed\",\"stage\":\"captureFinalization\",\"errorCode\":\"internalEarlyExit\"}",
        ])
    );
}

fn scenario_stop_empty_audio() -> Vec<String> {
    let (state, host) = fixture(DictationStatus::Recording, 7);
    install_context(&state, 7);
    host.script().stop_result = Some(Ok(Vec::new()));
    let reply = stop(&host, &state, None).expect("stop replies");
    assert_eq!(
        reply,
        serde_json::json!({ "type": "transcription", "text": "", "state": "idle" })
    );
    host.log()
}

#[test]
fn stop_with_no_audio_stops_the_supervisor_then_returns_to_idle() {
    assert_eq!(
        scenario_stop_empty_audio(),
        strings(&[
            "claim_preview_hide 7",
            "ax.capture_delivery_target_snapshot",
            "keyboard.hotkey_release_take_for 7",
            "keyboard.stop_gesture_take_for 7",
            "keyboard.set_processing true",
            &format!("emit {STATUS_EVENT} \"processing\""),
            "supervisor.stop 7 Other",
            "performance.production_sample_count 7 0",
            &format!("emit {STATUS_EVENT} \"idle\""),
        ])
    );
}

fn scenario_stop_pipeline_error() -> Vec<String> {
    let (state, host) = fixture(DictationStatus::Recording, 7);
    install_context(&state, 7);
    {
        let mut script = host.script();
        script.stop_result = Some(Ok(vec![0.0; 5_000]));
        script.pipeline_error = Some("scripted pipeline failure".to_string());
    }
    let error = stop(&host, &state, None).expect_err("the pipeline fails");
    assert_eq!(error, "scripted pipeline failure");
    host.log()
}

#[test]
fn stop_hands_enough_audio_to_the_pipeline_and_resets_after_it_returns() {
    assert_eq!(
        scenario_stop_pipeline_error(),
        strings(&[
            "claim_preview_hide 7",
            "ax.capture_delivery_target_snapshot",
            "keyboard.hotkey_release_take_for 7",
            "keyboard.stop_gesture_take_for 7",
            "keyboard.set_processing true",
            &format!("emit {STATUS_EVENT} \"processing\""),
            "supervisor.stop 7 Other",
            "performance.production_sample_count 7 5000",
            "pipeline 7 samples=5000",
            "keyboard.set_processing false",
            &format!("emit {STATUS_EVENT} \"idle\""),
        ])
    );
}

fn scenario_stop_stale_expectation() -> Vec<String> {
    let (state, host) = fixture(DictationStatus::Recording, 7);
    let reply = stop(&host, &state, Some(6)).expect("stop replies");
    assert_eq!(
        reply,
        serde_json::json!({ "type": "recording_superseded", "state": "idle" })
    );
    assert_eq!(status(&state), DictationStatus::Recording);
    host.log()
}

#[test]
fn stop_for_a_stale_generation_drops_its_interrupted_capture_and_nothing_else() {
    assert_eq!(
        scenario_stop_stale_expectation(),
        strings(&["supervisor.take_interrupted 6"])
    );
}

fn scenario_stop_recovering_with_interrupted_capture() -> Vec<String> {
    let (state, host) = fixture(DictationStatus::Recovering, 7);
    install_context(&state, 7);
    {
        let mut script = host.script();
        script.interrupted = Some(audio_lifecycle::InterruptedCapture {
            samples: vec![0.0; 6_000],
            reason: "device_changed".to_string(),
            duration_ms: 375,
        });
        script.pipeline_error = Some("scripted pipeline failure".to_string());
    }
    stop(&host, &state, None).expect_err("the pipeline fails");
    host.log()
}

#[test]
fn stop_of_an_interrupted_capture_skips_the_supervisor_stop() {
    assert_eq!(
        scenario_stop_recovering_with_interrupted_capture(),
        strings(&[
            "supervisor.take_interrupted 7",
            "claim_preview_hide 7",
            "ax.capture_delivery_target_snapshot",
            "keyboard.hotkey_release_take_for 7",
            "keyboard.stop_gesture_take_for 7",
            "keyboard.set_processing true",
            &format!("emit {STATUS_EVENT} \"processing\""),
            "performance.production_sample_count 7 6000",
            "pipeline 7 samples=6000",
            "keyboard.set_processing false",
            &format!("emit {STATUS_EVENT} \"idle\""),
        ])
    );
}

fn scenario_stop_during_starting() -> Vec<String> {
    let (state, host) = fixture(DictationStatus::Starting, 7);
    host.script().cancel_result = Some(Ok(true));
    let reply = stop(&host, &state, None).expect("stop replies");
    assert_eq!(
        reply,
        serde_json::json!({ "type": "recording_cancelled", "state": "recovering" })
    );
    host.log()
}

#[test]
fn stop_during_starting_cancels_the_exact_owner_after_claiming_the_release() {
    assert_eq!(
        scenario_stop_during_starting(),
        strings(&[
            "keyboard.hotkey_release_take_for 7",
            "keyboard.stop_gesture_take_for 7",
            "supervisor.cancel 7 User",
        ])
    );
}

// ----------------------------------------------------------------- cancel

fn scenario_cancel_starting() -> Vec<String> {
    let (state, host) = fixture(DictationStatus::Starting, 7);
    host.script().cancel_result = Some(Ok(true));
    cancel(&host, &state, None).expect("cancel succeeds");
    host.log()
}

#[test]
fn cancel_while_starting_cancels_the_capture_and_nothing_else() {
    assert_eq!(
        scenario_cancel_starting(),
        strings(&["supervisor.cancel 7 User"])
    );
}

fn scenario_cancel_failure_reports_a_terminal() -> Vec<String> {
    let (state, host) = fixture(DictationStatus::Recording, 7);
    host.script().cancel_result = Some(Err("supervisor gone".to_string()));
    let error = cancel(&host, &state, None).expect_err("cancel fails");
    assert_eq!(error, "supervisor gone");
    host.log()
}

#[test]
fn cancel_that_the_supervisor_refuses_returns_the_error() {
    assert_eq!(
        scenario_cancel_failure_reports_a_terminal(),
        strings(&["supervisor.cancel 7 User"])
    );
}

fn scenario_cancel_processing() -> Vec<String> {
    let (state, host) = fixture(DictationStatus::Processing, 7);
    cancel(&host, &state, None).expect("cancel succeeds");
    assert_eq!(status(&state), DictationStatus::Idle);
    assert!(state.app_state.is_cancelled(7));
    host.log()
}

#[test]
fn cancel_while_processing_resets_the_ui_then_completes_the_ledger() {
    assert_eq!(
        scenario_cancel_processing(),
        strings(&[
            "keyboard.set_processing false",
            &format!("emit {STATUS_EVENT} \"idle\""),
            "emit recording-cancelled {\"recordingId\":7}",
            "performance.complete 7 {\"status\":\"cancelled\",\"stage\":\"inferenceDecode\"}",
        ])
    );
}

#[test]
fn cancel_for_another_generation_or_while_idle_reaches_no_port() {
    let (state, host) = fixture(DictationStatus::Recording, 7);
    cancel(&host, &state, Some(6)).expect("a stale cancel is a no-op");
    let (idle_state, idle_host) = fixture(DictationStatus::Idle, 7);
    cancel(&idle_host, &idle_state, None).expect("cancel while idle is a no-op");
    assert_eq!(host.log(), Vec::<String>::new());
    assert_eq!(idle_host.log(), Vec::<String>::new());
}

// -------------------------------------------------------- port coverage

/// Every port method is reached by at least two scenarios, so a regression in
/// one scenario cannot silently drop the only check on a port.
#[test]
fn every_port_is_reached_in_two_scenarios() {
    let start_ok = scenario_start_acknowledged().0;
    let start_refused = {
        let (state, host) = fixture(DictationStatus::Idle, 0);
        host.script().start_ack = Some(AudioStartError::AlreadyStarting);
        start(&host, &state).expect("start replies");
        host.log()
    };
    let start_smart = {
        let (state, host) = fixture(DictationStatus::Idle, 0);
        enable_smart_destination(&state);
        start(&host, &state).expect("start replies");
        host.log()
    };
    let start_flushed = {
        let (state, host) = fixture(DictationStatus::Idle, 0);
        enable_smart_destination(&state);
        host.script().on_emit_smart_destination = Some(Box::new(|state| {
            state.app_state.dictation.lock_or_recover().status = DictationStatus::Idle;
        }));
        start(&host, &state).expect("start replies");
        host.log()
    };
    let start_hold = {
        let (state, host) = fixture(DictationStatus::Idle, 0);
        let token =
            audio_lifecycle::gesture::emit(audio_lifecycle::gesture::Candidate::new(), true);
        run(start_native_recording_inner(
            &host,
            &state,
            None,
            None,
            Some("hold".to_string()),
            Some(token),
        ))
        .expect("start replies");
        audio_lifecycle::gesture::clear_pending();
        host.log()
    };
    let start_hold_refused = {
        let (state, host) = fixture(DictationStatus::Idle, 0);
        let token =
            audio_lifecycle::gesture::emit(audio_lifecycle::gesture::Candidate::new(), true);
        host.script().start_ack = Some(AudioStartError::AlreadyStarting);
        run(start_native_recording_inner(
            &host,
            &state,
            None,
            None,
            Some("hold".to_string()),
            Some(token),
        ))
        .expect("start replies");
        audio_lifecycle::gesture::clear_pending();
        host.log()
    };
    let start_kept_preview = {
        let (state, host) = fixture(DictationStatus::Idle, 0);
        state
            .app_state
            .microphone_preview
            .claim(50)
            .expect("a mic test is kept");
        host.script().start_ack = Some(AudioStartError::AudioRecovering);
        start(&host, &state).expect("start replies");
        host.log()
    };
    let start_kept_preview_again = {
        let (state, host) = fixture(DictationStatus::Idle, 0);
        state
            .app_state
            .microphone_preview
            .claim(51)
            .expect("a mic test is kept");
        host.script().start_ack = Some(AudioStartError::AlreadyStarting);
        start(&host, &state).expect("start replies");
        host.log()
    };
    let scenarios: Vec<Vec<String>> = vec![
        start_ok,
        start_refused,
        start_smart,
        start_flushed,
        start_hold,
        start_hold_refused,
        start_kept_preview,
        start_kept_preview_again,
        scenario_stop_missing_context(),
        scenario_stop_empty_audio(),
        scenario_stop_pipeline_error(),
        scenario_stop_stale_expectation(),
        scenario_stop_recovering_with_interrupted_capture(),
        scenario_stop_during_starting(),
        scenario_cancel_starting(),
        scenario_cancel_failure_reports_a_terminal(),
        scenario_cancel_processing(),
    ];
    let ports = [
        "emit ",
        "mic_transition",
        "keyboard.is_app_disabled",
        "keyboard.listener_generation",
        "keyboard.set_processing",
        "keyboard.hotkey_release_take_for",
        "keyboard.stop_gesture_take_for",
        "ax.capture_delivery_target_snapshot",
        "smart_destination.capture",
        "performance.register_production",
        "performance.production_context",
        "performance.production_sample_count",
        "performance.begin_register",
        "performance.begin_abort",
        "performance.spawn_begin",
        "performance.complete",
        "supervisor.start ",
        "supervisor.start_after_preview",
        "supervisor.stop ",
        "supervisor.cancel",
        "supervisor.take_interrupted",
        "refresh_expired_ide_context",
        "emit_smart_destination",
        "publish_recording_ready",
        "spawn_model_preparation",
        "claim_preview_hide",
        "pipeline ",
    ];
    for port in ports {
        let reached = scenarios
            .iter()
            .filter(|log| log.iter().any(|entry| entry.starts_with(port)))
            .count();
        assert!(
            reached >= 2,
            "port `{port}` is reached by {reached} scenario(s); two are required"
        );
    }
}
