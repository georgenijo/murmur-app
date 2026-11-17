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
