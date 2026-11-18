//! Seam-contract tests for the Stage 0a `DictationHost` (plan §6, C9.1 C5).
//!
//! A `FakeHost` stands in for `TauriHost`, logs every port call in the order
//! the command reaches it, and returns scripted results. Each scenario asserts
//! its complete log, which is the order the production code reaches those ports
//! (read from the command bodies, not from this fake; every `expected_*`
//! function cites the `recording.rs` lines it was read from). A scenario is
//! single-task: the order is the program order of the awaits, and the one
//! scenario that hops threads (the kept-preview start) is ordered by awaiting
//! the `spawn_blocking` join. A scripted hook runs inside a port at the point
//! production reaches it, so a test can change shared state "between" two ports
//! with no timing dependence.
//!
//! Every log entry is recorded under a [`Call`] kind, and the coverage is
//! enumerated from `Call::ALL`, whose members are tied to the trait
//! declarations by an exhaustive `match` (a new `Call` variant must be named)
//! and by `call_kinds_match_the_trait_declarations` (a trait method with no
//! `Call` fails). `every_port_is_reached_in_two_scenarios` then asserts the
//! full ordered trace of every scenario and that each `Call` is reached by two.

use super::dictation_host::{
    HostEmitError, HostFuture, KeyboardPort, PerformancePort, SmartDestinationPort, SupervisorPort,
    TargetSnapshotPort,
};
use super::tests::test_state;
use super::*;
use std::sync::Mutex as StdMutex;

type Hook = Box<dyn Fn(&State) + Send>;

/// One variant per port method the commands can call (the accessors
/// `supervisor`, `keyboard`, `ax`, `smart_destination` and `performance` only
/// hand out a port and are not calls). `emit` on `dyn DictationHost` reaches
/// the trait's `emit_json`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Call {
    EmitJson,
    MicrophoneTransition,
    ResolveSmartAuto,
    RefreshExpiredIdeContext,
    EmitSmartDestination,
    PublishRecordingReady,
    SpawnModelPreparation,
    ClaimPreviewHide,
    RunTranscriptionPipeline,
    SupervisorStart,
    SupervisorStartAfterPreview,
    SupervisorStop,
    SupervisorCancel,
    SupervisorTakeInterrupted,
    SupervisorIsDictationCleanupPending,
    KeyboardIsAppDisabled,
    KeyboardListenerGeneration,
    KeyboardSetProcessing,
    KeyboardHotkeyReleaseTakeFor,
    KeyboardStopGestureTakeFor,
    AxCaptureDeliveryTargetSnapshot,
    SmartDestinationCapture,
    PerformanceRegisterProduction,
    PerformanceProductionContext,
    PerformanceProductionSampleCount,
    PerformanceBeginRegister,
    PerformanceBeginAbort,
    PerformanceSpawnBegin,
    PerformanceComplete,
}

impl Call {
    /// Adding a variant without listing it here fails to compile, and a
    /// variant missing from this list fails
    /// `call_kinds_match_the_trait_declarations`.
    const ALL: [Call; 29] = [
        Call::EmitJson,
        Call::MicrophoneTransition,
        Call::ResolveSmartAuto,
        Call::RefreshExpiredIdeContext,
        Call::EmitSmartDestination,
        Call::PublishRecordingReady,
        Call::SpawnModelPreparation,
        Call::ClaimPreviewHide,
        Call::RunTranscriptionPipeline,
        Call::SupervisorStart,
        Call::SupervisorStartAfterPreview,
        Call::SupervisorStop,
        Call::SupervisorCancel,
        Call::SupervisorTakeInterrupted,
        Call::SupervisorIsDictationCleanupPending,
        Call::KeyboardIsAppDisabled,
        Call::KeyboardListenerGeneration,
        Call::KeyboardSetProcessing,
        Call::KeyboardHotkeyReleaseTakeFor,
        Call::KeyboardStopGestureTakeFor,
        Call::AxCaptureDeliveryTargetSnapshot,
        Call::SmartDestinationCapture,
        Call::PerformanceRegisterProduction,
        Call::PerformanceProductionContext,
        Call::PerformanceProductionSampleCount,
        Call::PerformanceBeginRegister,
        Call::PerformanceBeginAbort,
        Call::PerformanceSpawnBegin,
        Call::PerformanceComplete,
    ];

    /// `(trait, method)` the variant stands for, and the log label. No
    /// wildcard arm: the match is the exhaustiveness check.
    fn spec(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Call::EmitJson => ("DictationHost", "emit_json", "emit"),
            Call::MicrophoneTransition => {
                ("DictationHost", "microphone_transition", "mic_transition")
            }
            Call::ResolveSmartAuto => ("DictationHost", "resolve_smart_auto", "resolve_smart_auto"),
            Call::RefreshExpiredIdeContext => (
                "DictationHost",
                "refresh_expired_ide_context",
                "refresh_expired_ide_context",
            ),
            Call::EmitSmartDestination => (
                "DictationHost",
                "emit_smart_destination",
                "emit_smart_destination",
            ),
            Call::PublishRecordingReady => (
                "DictationHost",
                "publish_recording_ready_if_audio_ready",
                "publish_recording_ready",
            ),
            Call::SpawnModelPreparation => (
                "DictationHost",
                "spawn_model_preparation",
                "spawn_model_preparation",
            ),
            Call::ClaimPreviewHide => ("DictationHost", "claim_preview_hide", "claim_preview_hide"),
            Call::RunTranscriptionPipeline => {
                ("DictationHost", "run_transcription_pipeline", "pipeline")
            }
            Call::SupervisorStart => (
                "SupervisorPort",
                "start_dictation_recording",
                "supervisor.start",
            ),
            Call::SupervisorStartAfterPreview => (
                "SupervisorPort",
                "start_dictation_recording_after_preview",
                "supervisor.start_after_preview",
            ),
            Call::SupervisorStop => (
                "SupervisorPort",
                "stop_dictation_recording",
                "supervisor.stop",
            ),
            Call::SupervisorCancel => (
                "SupervisorPort",
                "cancel_dictation_capture",
                "supervisor.cancel",
            ),
            Call::SupervisorTakeInterrupted => (
                "SupervisorPort",
                "take_interrupted_dictation",
                "supervisor.take_interrupted",
            ),
            Call::SupervisorIsDictationCleanupPending => (
                "SupervisorPort",
                "is_dictation_cleanup_pending",
                "supervisor.is_dictation_cleanup_pending",
            ),
            Call::KeyboardIsAppDisabled => (
                "KeyboardPort",
                "is_app_disabled",
                "keyboard.is_app_disabled",
            ),
            Call::KeyboardListenerGeneration => (
                "KeyboardPort",
                "listener_generation",
                "keyboard.listener_generation",
            ),
            Call::KeyboardSetProcessing => {
                ("KeyboardPort", "set_processing", "keyboard.set_processing")
            }
            Call::KeyboardHotkeyReleaseTakeFor => (
                "KeyboardPort",
                "hotkey_release_take_for",
                "keyboard.hotkey_release_take_for",
            ),
            Call::KeyboardStopGestureTakeFor => (
                "KeyboardPort",
                "stop_gesture_take_for",
                "keyboard.stop_gesture_take_for",
            ),
            Call::AxCaptureDeliveryTargetSnapshot => (
                "TargetSnapshotPort",
                "capture_delivery_target_snapshot",
                "ax.capture_delivery_target_snapshot",
            ),
            Call::SmartDestinationCapture => (
                "SmartDestinationPort",
                "capture",
                "smart_destination.capture",
            ),
            Call::PerformanceRegisterProduction => (
                "PerformancePort",
                "register_production",
                "performance.register_production",
            ),
            Call::PerformanceProductionContext => (
                "PerformancePort",
                "production_context",
                "performance.production_context",
            ),
            Call::PerformanceProductionSampleCount => (
                "PerformancePort",
                "production_sample_count",
                "performance.production_sample_count",
            ),
            Call::PerformanceBeginRegister => (
                "PerformancePort",
                "begin_register",
                "performance.begin_register",
            ),
            Call::PerformanceBeginAbort => {
                ("PerformancePort", "begin_abort", "performance.begin_abort")
            }
            Call::PerformanceSpawnBegin => {
                ("PerformancePort", "spawn_begin", "performance.spawn_begin")
            }
            Call::PerformanceComplete => ("PerformancePort", "complete", "performance.complete"),
        }
    }

    fn label(self) -> &'static str {
        self.spec().2
    }
}

/// Everything a scenario's host recorded: the rendered log and, in the same
/// order, the kind of every entry.
struct Recorded {
    log: Vec<String>,
    kinds: Vec<Call>,
}

#[derive(Default)]
struct Script {
    app_disabled: bool,
    start_ack: Option<AudioStartError>,
    stop_result: Option<Result<Vec<f32>, String>>,
    cancel_result: Option<Result<bool, String>>,
    interrupted: Option<audio_lifecycle::InterruptedCapture>,
    /// What `is_dictation_cleanup_pending` answers (the owner is still being
    /// cleaned up).
    cleanup_pending: bool,
    /// What the Smart Auto resolve answers (`None`: not scripted, an error).
    smart_auto: Option<Result<crate::microphone_auto::SmartAutoSelection, String>>,
    pipeline_error: Option<String>,
    on_emit_smart_destination: Option<Hook>,
    on_supervisor_start: Option<Hook>,
    /// Runs inside the supervisor stop, after the stop was admitted.
    on_supervisor_stop: Option<Hook>,
}

struct Shared {
    log: StdMutex<Vec<(Call, String)>>,
    state: Arc<State>,
    script: StdMutex<Script>,
}

impl Shared {
    /// The entry is the call's label, then its arguments, so a call cannot be
    /// logged under another call's kind.
    fn push(&self, call: Call, detail: impl AsRef<str>) {
        let detail = detail.as_ref();
        let entry = if detail.is_empty() {
            call.label().to_string()
        } else {
            format!("{} {detail}", call.label())
        };
        self.log
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push((call, entry));
    }

    fn script(&self) -> std::sync::MutexGuard<'_, Script> {
        self.script
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

pub(super) struct FakeHost {
    shared: Arc<Shared>,
}

impl FakeHost {
    pub(super) fn new(state: Arc<State>) -> Self {
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

    fn recorded(&self) -> Recorded {
        let entries = self
            .shared
            .log
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        Recorded {
            kinds: entries.iter().map(|(call, _)| *call).collect(),
            log: entries.into_iter().map(|(_, entry)| entry).collect(),
        }
    }

    fn log(&self) -> Vec<String> {
        self.recorded().log
    }
}

impl DictationHost for FakeHost {
    fn emit_json(&self, event: &str, payload: serde_json::Value) -> Result<(), HostEmitError> {
        self.shared
            .push(Call::EmitJson, format!("{event} {payload}"));
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
        self.shared.push(Call::MicrophoneTransition, "");
        Box::pin(async move { Ok(state.app_state.recording_transition.lock().await) })
    }

    fn resolve_smart_auto(
        &self,
        _request: &SmartAutoRequest,
    ) -> Result<crate::microphone_auto::SmartAutoSelection, String> {
        self.shared.push(Call::ResolveSmartAuto, "");
        self.script()
            .smart_auto
            .clone()
            .unwrap_or_else(|| Err("smart auto is not scripted".to_string()))
    }

    fn refresh_expired_ide_context(&self, _app_state: &AppState, _bundle_id: Option<&str>) {
        self.shared.push(Call::RefreshExpiredIdeContext, "");
    }

    fn emit_smart_destination(
        &self,
        _app_state: &AppState,
        recording_id: u64,
        _context: &DictationContextSnapshot,
        _accepted_target: &crate::frontmost::DeliveryTargetSnapshot,
    ) {
        self.shared
            .push(Call::EmitSmartDestination, format!("{recording_id}"));
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
            .push(Call::PublishRecordingReady, format!("{recording_id}"));
        false
    }

    fn spawn_model_preparation(&self, _model_name: String, recording_id: u64) {
        self.shared
            .push(Call::SpawnModelPreparation, format!("{recording_id}"));
    }

    fn claim_preview_hide(
        &self,
        recording_id: u64,
    ) -> Option<crate::commands::dictation_preview::PreviewHideOwner> {
        self.shared
            .push(Call::ClaimPreviewHide, format!("{recording_id}"));
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
        self.shared.push(
            Call::RunTranscriptionPipeline,
            format!("{recording_id} samples={}", samples.len()),
        );
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
        self.push(
            Call::SupervisorStart,
            format!("{recording_id} {origin} status={status:?}"),
        );
        self.scripted_start_ack()
    }

    fn start_dictation_recording_after_preview(
        &self,
        _device_id: Option<String>,
        recording_id: u64,
        origin: &str,
        _gesture: audio_lifecycle::gesture::StartGesture,
    ) -> Result<(), AudioStartError> {
        self.push(
            Call::SupervisorStartAfterPreview,
            format!("{recording_id} {origin}"),
        );
        self.scripted_start_ack()
    }

    fn stop_dictation_recording(
        &self,
        recording_id: u64,
        trigger: audio_lifecycle::DictationStopTrigger,
        _released_at: Option<std::time::Instant>,
    ) -> Result<Vec<f32>, String> {
        self.push(Call::SupervisorStop, format!("{recording_id} {trigger:?}"));
        if let Some(hook) = self.script().on_supervisor_stop.as_ref() {
            hook(&self.state);
        }
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
        self.push(Call::SupervisorCancel, format!("{recording_id} {reason:?}"));
        self.script()
            .cancel_result
            .take()
            .expect("the scenario scripts the cancel result")
    }

    fn take_interrupted_dictation(
        &self,
        recording_id: u64,
    ) -> Option<audio_lifecycle::InterruptedCapture> {
        self.push(Call::SupervisorTakeInterrupted, format!("{recording_id}"));
        self.script().interrupted.take()
    }

    fn is_dictation_cleanup_pending(&self, recording_id: u64) -> bool {
        self.push(
            Call::SupervisorIsDictationCleanupPending,
            format!("{recording_id}"),
        );
        self.script().cleanup_pending
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
        self.push(Call::KeyboardIsAppDisabled, "");
        self.script().app_disabled
    }

    fn listener_generation(&self) -> u64 {
        self.push(Call::KeyboardListenerGeneration, "");
        0
    }

    fn set_processing(&self, processing: bool) {
        self.push(Call::KeyboardSetProcessing, format!("{processing}"));
    }

    fn hotkey_release_take_for(
        &self,
        recording_id: u64,
        _stop_entered_at: std::time::Instant,
    ) -> Option<crate::hotkey_release::HotkeyRelease> {
        self.push(
            Call::KeyboardHotkeyReleaseTakeFor,
            format!("{recording_id}"),
        );
        None
    }

    fn stop_gesture_take_for(
        &self,
        recording_id: u64,
        _entered: std::time::Instant,
    ) -> Option<crate::stop_gesture::StopStamp> {
        self.push(Call::KeyboardStopGestureTakeFor, format!("{recording_id}"));
        None
    }
}

impl TargetSnapshotPort for Shared {
    fn capture_delivery_target_snapshot(&self) -> crate::frontmost::DeliveryTargetSnapshot {
        self.push(Call::AxCaptureDeliveryTargetSnapshot, "");
        crate::frontmost::DeliveryTargetSnapshot::Incomplete
    }
}

impl SmartDestinationPort for Shared {
    fn capture<'a>(
        &'a self,
        _expected: &'a crate::frontmost::DeliveryTargetSnapshot,
        enabled: bool,
    ) -> HostFuture<'a, crate::smart_destination::Snapshot> {
        self.push(Call::SmartDestinationCapture, format!("enabled={enabled}"));
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
        self.push(
            Call::PerformanceRegisterProduction,
            format!("{recording_id} {selection:?}"),
        );
    }

    fn production_context(&self, recording_id: u64, _context: &DictationContextSnapshot) {
        self.push(
            Call::PerformanceProductionContext,
            format!("{recording_id}"),
        );
    }

    fn production_sample_count(&self, recording_id: u64, count: usize) {
        self.push(
            Call::PerformanceProductionSampleCount,
            format!("{recording_id} {count}"),
        );
    }

    fn begin_register(&self, recording_id: u64) {
        self.push(Call::PerformanceBeginRegister, format!("{recording_id}"));
    }

    fn begin_abort(&self, recording_id: u64) {
        self.push(Call::PerformanceBeginAbort, format!("{recording_id}"));
    }

    fn spawn_begin(
        &self,
        recording_id: u64,
        _runtimes: Vec<RuntimeIdentityV1>,
        _started_at_ms: i64,
    ) {
        self.push(Call::PerformanceSpawnBegin, format!("{recording_id}"));
    }

    fn complete(
        &self,
        recording_id: u64,
        outcome: RunOutcomeV1,
        _stages: Vec<StageTimingV1>,
        _input: Option<ContentFreeInputSummaryV1>,
        _runtimes: Option<Vec<RuntimeIdentityV1>>,
    ) {
        self.push(
            Call::PerformanceComplete,
            format!(
                "{recording_id} {}",
                serde_json::to_string(&outcome).expect("outcome serializes")
            ),
        );
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

fn start_smart_auto(host: &FakeHost, state: &State) -> Result<serde_json::Value, String> {
    run(start_native_recording_inner(
        host,
        state,
        None,
        Some(SmartAutoRequest {
            approved_device_ids: vec!["mic-7".to_string()],
            preferred_device_ids: Vec::new(),
            allow_continuity: false,
            require_recent_signal: false,
        }),
        None,
        None,
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

fn status_event(status: &str) -> String {
    format!("emit {STATUS_EVENT} \"{status}\"")
}

/// A run of one command against a fresh `FakeHost`, and the complete port-call
/// trace production must produce for it. The run asserts the reply and final
/// status; `check` and the coverage test compare the trace.
struct Scenario {
    name: &'static str,
    run: fn() -> Recorded,
    expected: fn() -> Vec<String>,
}

fn check(scenario: &Scenario) {
    let recorded = (scenario.run)();
    assert_eq!(
        recorded.log,
        (scenario.expected)(),
        "scenario `{}` left production order",
        scenario.name
    );
    assert_eq!(recorded.kinds.len(), recorded.log.len());
}

// Line numbers below are in `recording.rs` (`start_native_recording_inner`
// ~5486, `stop_native_recording_inner` ~5898, `cancel_native_recording_inner`
// ~6394): the program order of the port calls in those bodies.

// ------------------------------------------------------------------ start

/// Start up to and including the begin reservation, as the accepted start
/// reaches it, in program order: transition (5560), app-disabled read (5564),
/// the cleanup-pending read of the current id under the dictation lock (5604),
/// the listener generation only for a stamped hold (5650), the AX sample
/// (5674), the generation-started and `starting` emits (5696 and the status
/// emit after it), the production reservation (5705) and begin (5716).
fn start_through_begin_register(hold: bool) -> Vec<String> {
    let mut trace = strings(&[
        "mic_transition",
        "keyboard.is_app_disabled",
        "supervisor.is_dictation_cleanup_pending 0",
    ]);
    if hold {
        trace.push("keyboard.listener_generation".to_string());
    }
    trace.extend(strings(&[
        "ax.capture_delivery_target_snapshot",
        "emit dictation-generation-started {\"recordingId\":1}",
    ]));
    trace.push(status_event("starting"));
    trace.extend(strings(&[
        "performance.register_production 1 SystemDefault",
        "performance.begin_register 1",
    ]));
    trace
}

/// The supervisor entry (5663-5677) takes the scripted ack.
fn scenario_start_acknowledged() -> Recorded {
    let (state, host) = fixture(DictationStatus::Idle, 0);
    let reply = start(&host, &state).expect("start replies");
    assert_eq!(
        reply,
        serde_json::json!({
            "type": "recording_starting", "state": "starting", "recordingId": 1
        })
    );
    assert_eq!(status(&state), DictationStatus::Starting);
    host.recorded()
}

/// After the supervisor (5675): the smart capture (5715), the IDE refresh
/// (5739), no destination emit while Smart Destination is off (5750), the
/// context (5753), then ready (5846), model preparation (5847) and the
/// diagnostics begin (5852).
fn expected_start_acknowledged() -> Vec<String> {
    let mut trace = start_through_begin_register(false);
    trace.extend(strings(&[
        "supervisor.start 1 toggle status=Starting",
        "smart_destination.capture enabled=false",
        "refresh_expired_ide_context",
        "performance.production_context 1",
        "publish_recording_ready 1",
        "spawn_model_preparation 1",
        "performance.spawn_begin 1",
    ]));
    trace
}

/// Smart Auto resolves after the retiring-capture read and before any allocation
/// (R101.13): the resolve is the fourth port call, the reservation names Smart Auto.
fn scenario_start_smart_auto_resolved() -> Recorded {
    let (state, host) = fixture(DictationStatus::Idle, 0);
    host.script().smart_auto = Some(Ok(crate::microphone_auto::SmartAutoSelection {
        device_id: "mic-7".to_string(),
        reason: crate::microphone_auto::SmartAutoReason::CurrentVerified,
    }));
    let reply = start_smart_auto(&host, &state).expect("start replies");
    assert_eq!(
        reply,
        serde_json::json!({
            "type": "recording_starting", "state": "starting", "recordingId": 1
        })
    );
    assert_eq!(status(&state), DictationStatus::Starting);
    host.recorded()
}

fn expected_start_smart_auto_resolved() -> Vec<String> {
    let mut trace = strings(&[
        "mic_transition",
        "keyboard.is_app_disabled",
        "supervisor.is_dictation_cleanup_pending 0",
        "resolve_smart_auto",
        "ax.capture_delivery_target_snapshot",
        "emit dictation-generation-started {\"recordingId\":1}",
    ]);
    trace.push(status_event("starting"));
    trace.extend(strings(&[
        "performance.register_production 1 SmartAuto",
        "performance.begin_register 1",
        "supervisor.start 1 toggle status=Starting",
        "smart_destination.capture enabled=false",
        "refresh_expired_ide_context",
        "performance.production_context 1",
        "publish_recording_ready 1",
        "spawn_model_preparation 1",
        "performance.spawn_begin 1",
    ]));
    trace
}

/// A failed Smart Auto resolve refuses with its own text (R101.14) before any id
/// is burned, any claim is accepted or any status is written.
fn scenario_start_smart_auto_refused() -> Recorded {
    let (state, host) = fixture(DictationStatus::Idle, 3);
    host.script().smart_auto = Some(Err("no verified microphone".to_string()));
    let refusal = start_smart_auto(&host, &state).expect_err("start refuses");
    assert_eq!(refusal, "no verified microphone");
    assert_eq!(status(&state), DictationStatus::Idle);
    assert_eq!(state.app_state.recording_id.load(Ordering::SeqCst), 3);
    host.recorded()
}

fn expected_start_smart_auto_refused() -> Vec<String> {
    strings(&[
        "mic_transition",
        "keyboard.is_app_disabled",
        "supervisor.is_dictation_cleanup_pending 3",
        "resolve_smart_auto",
    ])
}

fn scenario_start_refused() -> Recorded {
    let (state, host) = fixture(DictationStatus::Idle, 0);
    host.script().start_ack = Some(AudioStartError::AlreadyStarting);
    let reply = start(&host, &state).expect("start replies");
    assert_eq!(
        reply,
        serde_json::json!({ "type": "already_starting", "state": "idle" })
    );
    assert_eq!(status(&state), DictationStatus::Idle);
    host.recorded()
}

/// A refused start aborts the begin (5679) and emits `idle` (5686) before any
/// context work.
fn expected_start_refused() -> Vec<String> {
    let mut trace = start_through_begin_register(false);
    trace.extend(strings(&[
        "supervisor.start 1 toggle status=Starting",
        "performance.begin_abort 1",
    ]));
    trace.push(status_event("idle"));
    trace
}

/// An Idle status with the previous owner still being cleaned up: the start
/// reads the pending cleanup of the current id (5599), writes `Recovering` with
/// that id and answers `audio_recovering` before any allocation or AX sample.
fn scenario_start_cleanup_pending() -> Recorded {
    let (state, host) = fixture(DictationStatus::Idle, 7);
    host.script().cleanup_pending = true;
    let reply = start(&host, &state).expect("start replies");
    assert_eq!(
        reply,
        serde_json::json!({ "type": "audio_recovering", "state": "recovering" })
    );
    assert_eq!(status(&state), DictationStatus::Recovering);
    assert_eq!(state.app_state.recording_id.load(Ordering::SeqCst), 7);
    host.recorded()
}

fn expected_start_cleanup_pending() -> Vec<String> {
    let mut trace = strings(&[
        "mic_transition",
        "keyboard.is_app_disabled",
        "supervisor.is_dictation_cleanup_pending 7",
    ]);
    trace.push(status_event("recovering"));
    trace
}

fn scenario_start_app_disabled() -> Recorded {
    let (state, host) = fixture(DictationStatus::Idle, 0);
    host.script().app_disabled = true;
    let reply = start(&host, &state).expect("start replies");
    assert_eq!(
        reply,
        serde_json::json!({ "type": "app_disabled", "state": "idle" })
    );
    host.recorded()
}

/// Returns right after the app-disabled read (5507-5510).
fn expected_start_app_disabled() -> Vec<String> {
    strings(&["mic_transition", "keyboard.is_app_disabled"])
}

/// A kept Settings mic test sends the start through the supervisor's
/// after-preview entry on a blocking thread (5663-5671). The join is the causal
/// barrier: everything after it is ordered behind the supervisor call.
fn scenario_start_kept_preview() -> Recorded {
    let (state, host) = fixture(DictationStatus::Idle, 0);
    state
        .app_state
        .microphone_preview
        .claim(50)
        .expect("a mic test is kept");
    host.script().start_ack = Some(AudioStartError::AudioRecovering);
    let reply = start(&host, &state).expect("start replies");
    assert_eq!(
        reply,
        serde_json::json!({ "type": "audio_recovering", "state": "idle" })
    );
    host.recorded()
}

/// The after-preview entry replaces `supervisor.start` (5663-5671); the refusal
/// then aborts (5679) and emits `idle` (5686).
fn expected_start_kept_preview() -> Vec<String> {
    let mut trace = start_through_begin_register(false);
    trace.push("supervisor.start_after_preview 1 toggle".to_string());
    trace.push("performance.begin_abort 1".to_string());
    trace.push(status_event("idle"));
    trace
}

/// A second kept-preview scenario with a different refusal, so the
/// after-preview entry and the abort are ordered by two traces.
fn scenario_start_kept_preview_again() -> Recorded {
    let (state, host) = fixture(DictationStatus::Idle, 0);
    state
        .app_state
        .microphone_preview
        .claim(51)
        .expect("a mic test is kept");
    host.script().start_ack = Some(AudioStartError::AlreadyStarting);
    let reply = start(&host, &state).expect("start replies");
    assert_eq!(
        reply,
        serde_json::json!({ "type": "already_starting", "state": "idle" })
    );
    host.recorded()
}

/// Same path as `expected_start_kept_preview` (5663-5686).
fn expected_start_kept_preview_again() -> Vec<String> {
    expected_start_kept_preview()
}

/// With Smart Destination on, the capture is asked for with `enabled=true`
/// (5715) and the selection is emitted (5751) after the IDE refresh (5739) and
/// before the context is recorded (5753).
fn scenario_start_smart_destination() -> Recorded {
    let (state, host) = fixture(DictationStatus::Idle, 0);
    enable_smart_destination(&state);
    let reply = start(&host, &state).expect("start replies");
    assert_eq!(reply["type"], "recording_starting");
    host.recorded()
}

fn expected_start_smart_destination() -> Vec<String> {
    let mut trace = start_through_begin_register(false);
    trace.extend(strings(&[
        "supervisor.start 1 toggle status=Starting",
        "smart_destination.capture enabled=true",
        "refresh_expired_ide_context",
        "emit_smart_destination 1",
        "performance.production_context 1",
        "publish_recording_ready 1",
        "spawn_model_preparation 1",
        "performance.spawn_begin 1",
    ]));
    trace
}

/// The supervisor's own `Idle` can land while the context resolves (the hook
/// runs inside the destination emit, 5751). The start then flushes the reserved
/// diagnostics (5785) instead of publishing Recording, so the model is never
/// prepared and no ready is published.
fn scenario_start_capture_ended_during_context() -> Recorded {
    let (state, host) = fixture(DictationStatus::Idle, 0);
    enable_smart_destination(&state);
    host.script().on_emit_smart_destination = Some(Box::new(|state| {
        state.app_state.dictation.lock_or_recover().status = DictationStatus::Idle;
    }));
    let reply = start(&host, &state).expect("start replies");
    assert_eq!(
        reply,
        serde_json::json!({ "type": "audio_recovering", "state": "idle" })
    );
    host.recorded()
}

fn expected_start_capture_ended_during_context() -> Vec<String> {
    let mut trace = start_through_begin_register(false);
    trace.extend(strings(&[
        "supervisor.start 1 toggle status=Starting",
        "smart_destination.capture enabled=true",
        "refresh_expired_ide_context",
        "emit_smart_destination 1",
        "performance.production_context 1",
        "performance.spawn_begin 1",
    ]));
    trace
}

/// A newer generation can take over while the context resolves (the hook runs
/// inside the supervisor start, 5675): the start aborts its begin (5797) and
/// publishes nothing further.
fn scenario_start_superseded_during_context() -> Recorded {
    let (state, host) = fixture(DictationStatus::Idle, 0);
    host.script().on_supervisor_start = Some(Box::new(|state| {
        state.app_state.recording_id.store(9, Ordering::SeqCst);
    }));
    let reply = start(&host, &state).expect("start replies");
    assert_eq!(
        reply,
        serde_json::json!({ "type": "audio_recovering", "state": "starting" })
    );
    host.recorded()
}

fn expected_start_superseded_during_context() -> Vec<String> {
    let mut trace = start_through_begin_register(false);
    trace.extend(strings(&[
        "supervisor.start 1 toggle status=Starting",
        "smart_destination.capture enabled=false",
        "refresh_expired_ide_context",
        "performance.production_context 1",
        "performance.begin_abort 1",
    ]));
    trace
}

fn start_stamped_hold(host: &FakeHost, state: &State) -> serde_json::Value {
    let token = audio_lifecycle::gesture::emit(audio_lifecycle::gesture::Candidate::new(), true);
    let reply = run(start_native_recording_inner(
        host,
        state,
        None,
        None,
        Some("hold".to_string()),
        Some(token),
    ))
    .expect("start replies");
    audio_lifecycle::gesture::clear_pending();
    reply
}

/// A stamped hold gesture reads the keyboard listener generation (5593) while
/// the start is accepted, before the delivery target is sampled (5617).
fn scenario_start_stamped_hold() -> Recorded {
    let (state, host) = fixture(DictationStatus::Idle, 0);
    let reply = start_stamped_hold(&host, &state);
    assert_eq!(reply["type"], "recording_starting");
    host.recorded()
}

fn expected_start_stamped_hold() -> Vec<String> {
    let mut trace = start_through_begin_register(true);
    trace.extend(strings(&[
        "supervisor.start 1 hold status=Starting",
        "smart_destination.capture enabled=false",
        "refresh_expired_ide_context",
        "performance.production_context 1",
        "publish_recording_ready 1",
        "spawn_model_preparation 1",
        "performance.spawn_begin 1",
    ]));
    trace
}

/// The same stamped hold with a refused start: the listener generation is read
/// before the AX sample (5593 before 5617) and the abort follows the
/// supervisor (5679, 5686).
fn scenario_start_stamped_hold_refused() -> Recorded {
    let (state, host) = fixture(DictationStatus::Idle, 0);
    host.script().start_ack = Some(AudioStartError::AlreadyStarting);
    let reply = start_stamped_hold(&host, &state);
    assert_eq!(
        reply,
        serde_json::json!({ "type": "already_starting", "state": "idle" })
    );
    host.recorded()
}

fn expected_start_stamped_hold_refused() -> Vec<String> {
    let mut trace = start_through_begin_register(true);
    trace.push("supervisor.start 1 hold status=Starting".to_string());
    trace.push("performance.begin_abort 1".to_string());
    trace.push(status_event("idle"));
    trace
}

// ------------------------------------------------------------------- stop

fn scenario_stop_missing_context() -> Recorded {
    let (state, host) = fixture(DictationStatus::Recording, 7);
    let error = stop(&host, &state, None).expect_err("no context installed");
    assert_eq!(error, "Missing dictation context for recording 7");
    assert_eq!(status(&state), DictationStatus::Idle);
    host.recorded()
}

/// Recording: the preview hide (5971), the stop AX sample (5980), the release
/// and stop-gesture claims (5987, 5990), then the missing-context unwind
/// (6034, 6035, 6037).
fn expected_stop_missing_context() -> Vec<String> {
    let mut trace = strings(&[
        "claim_preview_hide 7",
        "ax.capture_delivery_target_snapshot",
        "keyboard.hotkey_release_take_for 7",
        "keyboard.stop_gesture_take_for 7",
        "keyboard.set_processing false",
    ]);
    trace.push(status_event("idle"));
    trace.push(
        "performance.complete 7 {\"status\":\"failed\",\"stage\":\"captureFinalization\",\"errorCode\":\"internalEarlyExit\"}"
            .to_string(),
    );
    trace
}

/// The spawned stop of an interrupted capture (R215.2 `RS`) for recording 7 on a
/// fake host, carrying the registered `AutoStop` owner. `probe` runs inside the
/// supervisor stop, i.e. after stop admission and before the stop returns.
pub(super) fn auto_stop_with_probe(
    owner: crate::core_history::ContinuationGuard,
    probe: impl Fn() + Send + 'static,
) -> serde_json::Value {
    let (state, host) = fixture(DictationStatus::Recording, 7);
    install_context(&state, 7);
    host.script().stop_result = Some(Ok(Vec::new()));
    host.script().on_supervisor_stop = Some(Box::new(move |_| probe()));
    run(stop_native_recording_admitted(
        &host,
        &state,
        Some(7),
        audio_lifecycle::DictationStopTrigger::Other,
        Some(owner),
    ))
    .expect("stop replies")
}

fn scenario_stop_empty_audio() -> Recorded {
    let (state, host) = fixture(DictationStatus::Recording, 7);
    install_context(&state, 7);
    host.script().stop_result = Some(Ok(Vec::new()));
    let reply = stop(&host, &state, None).expect("stop replies");
    assert_eq!(
        reply,
        serde_json::json!({ "type": "transcription", "text": "", "state": "idle" })
    );
    host.recorded()
}

/// Claims (5971-5990), processing (6050, 6052), the supervisor stop (6076), the
/// sample count (6100), then the no-audio `idle` (6142).
fn expected_stop_empty_audio() -> Vec<String> {
    let mut trace = strings(&[
        "claim_preview_hide 7",
        "ax.capture_delivery_target_snapshot",
        "keyboard.hotkey_release_take_for 7",
        "keyboard.stop_gesture_take_for 7",
        "keyboard.set_processing true",
    ]);
    trace.push(status_event("processing"));
    trace.extend(strings(&[
        "supervisor.stop 7 Other",
        "performance.production_sample_count 7 0",
    ]));
    trace.push(status_event("idle"));
    trace
}

fn scenario_stop_pipeline_error() -> Recorded {
    let (state, host) = fixture(DictationStatus::Recording, 7);
    install_context(&state, 7);
    {
        let mut script = host.script();
        script.stop_result = Some(Ok(vec![0.0; 5_000]));
        script.pipeline_error = Some("scripted pipeline failure".to_string());
    }
    let error = stop(&host, &state, None).expect_err("the pipeline fails");
    assert_eq!(error, "scripted pipeline failure");
    host.recorded()
}

/// As the empty-audio stop up to the sample count (6100), then the pipeline
/// (6198) and the reset after it returns (6218, 6219).
fn expected_stop_pipeline_error() -> Vec<String> {
    let mut trace = strings(&[
        "claim_preview_hide 7",
        "ax.capture_delivery_target_snapshot",
        "keyboard.hotkey_release_take_for 7",
        "keyboard.stop_gesture_take_for 7",
        "keyboard.set_processing true",
    ]);
    trace.push(status_event("processing"));
    trace.extend(strings(&[
        "supervisor.stop 7 Other",
        "performance.production_sample_count 7 5000",
        "pipeline 7 samples=5000",
        "keyboard.set_processing false",
    ]));
    trace.push(status_event("idle"));
    trace
}

fn scenario_stop_stale_expectation() -> Recorded {
    let (state, host) = fixture(DictationStatus::Recording, 7);
    let reply = stop(&host, &state, Some(6)).expect("stop replies");
    assert_eq!(
        reply,
        serde_json::json!({ "type": "recording_superseded", "state": "idle" })
    );
    assert_eq!(status(&state), DictationStatus::Recording);
    host.recorded()
}

/// Only the stale generation's interrupted capture is dropped (5910-5912).
fn expected_stop_stale_expectation() -> Vec<String> {
    strings(&["supervisor.take_interrupted 6"])
}

fn scenario_stop_recovering_with_interrupted_capture() -> Recorded {
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
    host.recorded()
}

/// Recovering takes the interrupted capture first (5949), claims the hide
/// (5950) and samples AX (5959), then the claims (5987, 5990), processing
/// (6050, 6052), the sample count (6100) and the pipeline (6198, 6218, 6219);
/// the supervisor stop (6076) is skipped.
fn expected_stop_recovering_with_interrupted_capture() -> Vec<String> {
    let mut trace = strings(&[
        "supervisor.take_interrupted 7",
        "claim_preview_hide 7",
        "ax.capture_delivery_target_snapshot",
        "keyboard.hotkey_release_take_for 7",
        "keyboard.stop_gesture_take_for 7",
        "keyboard.set_processing true",
    ]);
    trace.push(status_event("processing"));
    trace.extend(strings(&[
        "performance.production_sample_count 7 6000",
        "pipeline 7 samples=6000",
        "keyboard.set_processing false",
    ]));
    trace.push(status_event("idle"));
    trace
}

fn scenario_stop_during_starting() -> Recorded {
    let (state, host) = fixture(DictationStatus::Starting, 7);
    host.script().cancel_result = Some(Ok(true));
    let reply = stop(&host, &state, None).expect("stop replies");
    assert_eq!(
        reply,
        serde_json::json!({ "type": "recording_cancelled", "state": "recovering" })
    );
    host.recorded()
}

/// Starting claims the release (5987, 5990) and cancels the exact owner (6004);
/// it samples no AX and claims no hide.
fn expected_stop_during_starting() -> Vec<String> {
    strings(&[
        "keyboard.hotkey_release_take_for 7",
        "keyboard.stop_gesture_take_for 7",
        "supervisor.cancel 7 User",
    ])
}

// ----------------------------------------------------------------- cancel

fn scenario_cancel_starting() -> Recorded {
    let (state, host) = fixture(DictationStatus::Starting, 7);
    host.script().cancel_result = Some(Ok(true));
    cancel(&host, &state, None).expect("cancel succeeds");
    host.recorded()
}

/// Starting or Recording cancels only the capture (6424).
fn expected_cancel_starting() -> Vec<String> {
    strings(&["supervisor.cancel 7 User"])
}

fn scenario_cancel_refused() -> Recorded {
    let (state, host) = fixture(DictationStatus::Recording, 7);
    host.script().cancel_result = Some(Err("supervisor gone".to_string()));
    let error = cancel(&host, &state, None).expect_err("cancel fails");
    assert_eq!(error, "supervisor gone");
    host.recorded()
}

/// The refusal returns the error right after the cancel (6424-6436).
fn expected_cancel_refused() -> Vec<String> {
    strings(&["supervisor.cancel 7 User"])
}

fn scenario_cancel_processing() -> Recorded {
    let (state, host) = fixture(DictationStatus::Processing, 7);
    cancel(&host, &state, None).expect("cancel succeeds");
    assert_eq!(status(&state), DictationStatus::Idle);
    assert!(state.app_state.is_cancelled(7));
    host.recorded()
}

/// Processing resets the UI (6459, 6460, 6461) and only then completes the
/// ledger (6471).
fn expected_cancel_processing() -> Vec<String> {
    let mut trace = strings(&["keyboard.set_processing false"]);
    trace.push(status_event("idle"));
    trace.extend(strings(&[
        "emit recording-cancelled {\"recordingId\":7}",
        "performance.complete 7 {\"status\":\"cancelled\",\"stage\":\"inferenceDecode\"}",
    ]));
    trace
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

// -------------------------------------------------------------- scenarios

fn scenarios() -> Vec<Scenario> {
    vec![
        Scenario {
            name: "start_acknowledged",
            run: scenario_start_acknowledged,
            expected: expected_start_acknowledged,
        },
        Scenario {
            name: "start_smart_auto_resolved",
            run: scenario_start_smart_auto_resolved,
            expected: expected_start_smart_auto_resolved,
        },
        Scenario {
            name: "start_smart_auto_refused",
            run: scenario_start_smart_auto_refused,
            expected: expected_start_smart_auto_refused,
        },
        Scenario {
            name: "start_refused",
            run: scenario_start_refused,
            expected: expected_start_refused,
        },
        Scenario {
            name: "start_cleanup_pending",
            run: scenario_start_cleanup_pending,
            expected: expected_start_cleanup_pending,
        },
        Scenario {
            name: "start_app_disabled",
            run: scenario_start_app_disabled,
            expected: expected_start_app_disabled,
        },
        Scenario {
            name: "start_kept_preview",
            run: scenario_start_kept_preview,
            expected: expected_start_kept_preview,
        },
        Scenario {
            name: "start_kept_preview_again",
            run: scenario_start_kept_preview_again,
            expected: expected_start_kept_preview_again,
        },
        Scenario {
            name: "start_smart_destination",
            run: scenario_start_smart_destination,
            expected: expected_start_smart_destination,
        },
        Scenario {
            name: "start_capture_ended_during_context",
            run: scenario_start_capture_ended_during_context,
            expected: expected_start_capture_ended_during_context,
        },
        Scenario {
            name: "start_superseded_during_context",
            run: scenario_start_superseded_during_context,
            expected: expected_start_superseded_during_context,
        },
        Scenario {
            name: "start_stamped_hold",
            run: scenario_start_stamped_hold,
            expected: expected_start_stamped_hold,
        },
        Scenario {
            name: "start_stamped_hold_refused",
            run: scenario_start_stamped_hold_refused,
            expected: expected_start_stamped_hold_refused,
        },
        Scenario {
            name: "stop_missing_context",
            run: scenario_stop_missing_context,
            expected: expected_stop_missing_context,
        },
        Scenario {
            name: "stop_empty_audio",
            run: scenario_stop_empty_audio,
            expected: expected_stop_empty_audio,
        },
        Scenario {
            name: "stop_pipeline_error",
            run: scenario_stop_pipeline_error,
            expected: expected_stop_pipeline_error,
        },
        Scenario {
            name: "stop_stale_expectation",
            run: scenario_stop_stale_expectation,
            expected: expected_stop_stale_expectation,
        },
        Scenario {
            name: "stop_recovering_with_interrupted_capture",
            run: scenario_stop_recovering_with_interrupted_capture,
            expected: expected_stop_recovering_with_interrupted_capture,
        },
        Scenario {
            name: "stop_during_starting",
            run: scenario_stop_during_starting,
            expected: expected_stop_during_starting,
        },
        Scenario {
            name: "cancel_starting",
            run: scenario_cancel_starting,
            expected: expected_cancel_starting,
        },
        Scenario {
            name: "cancel_refused",
            run: scenario_cancel_refused,
            expected: expected_cancel_refused,
        },
        Scenario {
            name: "cancel_processing",
            run: scenario_cancel_processing,
            expected: expected_cancel_processing,
        },
    ]
}

/// Every scenario's complete port-call trace equals the production order.
/// One test per scenario would only repeat `check`; a single pass names the
/// scenario that drifted.
#[test]
fn every_scenario_follows_the_production_order() {
    for scenario in scenarios() {
        check(&scenario);
    }
}

// -------------------------------------------------------- port coverage

/// Each `Call` is tied to one trait method, and every trait method (bar the
/// accessors that only hand out a port) has a `Call`.
#[test]
fn call_kinds_match_the_trait_declarations() {
    use std::collections::BTreeSet;
    const ACCESSORS: [(&str, &str); 5] = [
        ("DictationHost", "supervisor"),
        ("DictationHost", "keyboard"),
        ("DictationHost", "ax"),
        ("DictationHost", "smart_destination"),
        ("DictationHost", "performance"),
    ];
    let mut declared: BTreeSet<(String, String)> = BTreeSet::new();
    let mut current: Option<String> = None;
    let ident = |rest: &str| -> String {
        rest.chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect()
    };
    for line in include_str!("recording_host.rs").lines() {
        let top_level = !line.starts_with(char::is_whitespace);
        let words: Vec<&str> = line.split_whitespace().collect();
        if top_level && !line.starts_with("//") && words.contains(&"trait") {
            // Any trait header, whatever its visibility or qualifiers.
            let rest = line.split("trait ").nth(1).unwrap_or_default();
            current = Some(ident(rest));
        } else if line == "}" {
            current = None;
        } else if let Some(trait_name) = &current {
            let item = line.strip_prefix("    ").filter(|r| !r.starts_with(' '));
            let Some(item) = item else { continue };
            if item.starts_with("//") || item.starts_with("#[") {
                continue;
            }
            if let Some(rest) = item.strip_prefix("fn ") {
                declared.insert((trait_name.clone(), ident(rest)));
            } else if item
                .split_whitespace()
                .any(|w| w == "fn" || w.starts_with("fn("))
            {
                // `unsafe fn`, `async fn`, `const fn`, `extern "C" fn`, ...:
                // refuse rather than silently skip a method.
                panic!("unsupported trait method syntax in {trait_name}: {line:?}");
            }
        }
    }
    for (trait_name, method) in ACCESSORS {
        assert!(
            declared.remove(&(trait_name.to_string(), method.to_string())),
            "accessor {trait_name}::{method} is not declared"
        );
    }
    let covered: BTreeSet<(String, String)> = Call::ALL
        .iter()
        .map(|call| {
            let (trait_name, method, _) = call.spec();
            (trait_name.to_string(), method.to_string())
        })
        .collect();
    assert_eq!(
        covered.len(),
        Call::ALL.len(),
        "two `Call`s stand for one method"
    );
    assert_eq!(
        covered, declared,
        "a port method has no `Call` (or a `Call` names no method)"
    );
    let labels: BTreeSet<&str> = Call::ALL.iter().map(|call| call.label()).collect();
    assert_eq!(labels.len(), Call::ALL.len(), "log labels are unique");
}

/// Every port method is reached by at least two scenarios, each of which also
/// asserts its complete production-order trace, so a regression in one scenario
/// cannot silently drop the only check on a port.
#[test]
fn every_port_is_reached_in_two_scenarios() {
    let mut reached: std::collections::HashMap<Call, usize> = std::collections::HashMap::new();
    for scenario in scenarios() {
        check(&scenario);
        let recorded = (scenario.run)();
        let distinct: std::collections::HashSet<Call> = recorded.kinds.iter().copied().collect();
        for call in distinct {
            *reached.entry(call).or_default() += 1;
        }
    }
    for call in Call::ALL {
        let count = reached.get(&call).copied().unwrap_or(0);
        assert!(
            count >= 2,
            "port `{call:?}` is reached by {count} scenario(s); two are required"
        );
    }
}
