//! Stage 0a test seam for the dictation commands (plan §6 Stage 0a, C9.2).
//!
//! `start_native_recording`, `stop_native_recording` and
//! `cancel_native_recording` keep their `#[tauri::command]` wrappers. Each
//! wrapper builds a [`TauriHost`] and calls an inner function that takes
//! `&dyn DictationHost`. The inner bodies are the former command bodies with
//! every call that needed a concrete `AppHandle` (or a process-global
//! singleton) routed through a port on the host. [`TauriHost`] is the only
//! implementation compiled outside tests, and each of its ports forwards to
//! the existing function unchanged, so production behaviour and statement
//! order are the same as before.
//!
//! This is a child module of `recording` so a port can name that module's
//! private types without widening their visibility. Everything here is
//! `pub(super)`, which is exactly the scope of those private types.

use super::*;
use std::future::Future;
use std::pin::Pin;

/// A boxed future for a port that is awaited. Boxing is what lets the port
/// live behind `&dyn`; it adds no await and no spawn to the caller.
pub(super) type HostFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What `AppHandle::emit` reported. No caller propagates it: every call site
/// is `let _ =` or reads `.is_ok()` (C9.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HostEmitError(pub(super) String);

/// The host the three dictation commands run against.
pub(super) trait DictationHost: Send + Sync + 'static {
    /// `AppHandle::emit` for an already serialized payload. Returns `Err`
    /// exactly where `AppHandle::emit` does. Call sites use
    /// [`DictationHost::emit`] on `dyn DictationHost`.
    fn emit_json(&self, event: &str, payload: serde_json::Value) -> Result<(), HostEmitError>;

    /// The audio supervisor's command side. Production keeps the lazy
    /// `get_or_init` construction inside `audio_lifecycle`; a fake is built
    /// ahead of time. Shared ownership because the kept-preview start moves it
    /// into `spawn_blocking`.
    fn supervisor(&self) -> Arc<dyn SupervisorPort>;
    fn keyboard(&self) -> &dyn KeyboardPort;
    fn ax(&self) -> &dyn TargetSnapshotPort;
    fn smart_destination(&self) -> &dyn SmartDestinationPort;
    fn performance(&self) -> &dyn PerformancePort;

    /// `microphone_preview::transition_for_dictation_start`: takes over or
    /// stops a Settings mic test and returns the recording transition guard.
    fn microphone_transition<'a>(
        &'a self,
        state: &'a State,
    ) -> HostFuture<'a, Result<tokio::sync::MutexGuard<'a, ()>, String>>;
    /// `audio_inventory::resolve_smart_auto`: the Smart Auto observation of the
    /// start (decision D-S1-1). Read only when the start's decision asks for it
    /// (`NeedObservation(SmartAutoSelection)`), after the blockers, the status
    /// step and the retiring-capture check passed.
    fn resolve_smart_auto(
        &self,
        request: &SmartAutoRequest,
    ) -> Result<crate::microphone_auto::SmartAutoSelection, String>;
    /// `refresh_expired_ide_context`.
    fn refresh_expired_ide_context(&self, app_state: &AppState, bundle_id: Option<&str>);
    /// `emit_smart_destination`.
    fn emit_smart_destination(
        &self,
        app_state: &AppState,
        recording_id: u64,
        context: &DictationContextSnapshot,
        accepted_target: &crate::frontmost::DeliveryTargetSnapshot,
    );
    /// `publish_recording_ready_if_audio_ready`. `ready_op` is the flight's
    /// `K:ReadyPublish` (R103.1), consumed by the publish's own decision.
    fn publish_recording_ready_if_audio_ready(
        &self,
        recording_id: u64,
        context_ready_at: std::time::Instant,
        ready_op: crate::core_history::ContinuationGuard,
    ) -> bool;
    /// `spawn_model_preparation`.
    fn spawn_model_preparation(&self, model_name: String, recording_id: u64);
    /// `PreviewHideOwner::claim`.
    fn claim_preview_hide(
        &self,
        recording_id: u64,
    ) -> Option<crate::commands::dictation_preview::PreviewHideOwner>;
    /// `run_transcription_pipeline`.
    fn run_transcription_pipeline<'a>(
        &'a self,
        samples: &'a [f32],
        app_state: &'a AppState,
        recording_id: u64,
        performance_guard: &'a mut NativeDictationPerformanceGuard,
        context: Arc<DictationContextSnapshot>,
        stop: PipelineStopContext,
    ) -> HostFuture<'a, Result<PipelineResult, String>>;
}

impl dyn DictationHost + '_ {
    /// Same shape as `AppHandle::emit`, so call sites read `host.emit(..)`.
    pub(super) fn emit<S: serde::Serialize>(
        &self,
        event: &str,
        payload: S,
    ) -> Result<(), HostEmitError> {
        let payload = serde_json::to_value(payload).map_err(|e| HostEmitError(e.to_string()))?;
        self.emit_json(event, payload)
    }
}

/// The supervisor calls the three commands make.
pub(super) trait SupervisorPort: Send + Sync + 'static {
    fn start_dictation_recording(
        &self,
        device_id: Option<String>,
        recording_id: u64,
        origin: &str,
        gesture: audio_lifecycle::gesture::StartGesture,
    ) -> Result<(), AudioStartError>;
    fn start_dictation_recording_after_preview(
        &self,
        device_id: Option<String>,
        recording_id: u64,
        origin: &str,
        gesture: audio_lifecycle::gesture::StartGesture,
    ) -> Result<(), AudioStartError>;
    fn stop_dictation_recording(
        &self,
        recording_id: u64,
        trigger: audio_lifecycle::DictationStopTrigger,
        released_at: Option<std::time::Instant>,
    ) -> Result<Vec<f32>, String>;
    fn cancel_dictation_capture(
        &self,
        recording_id: u64,
        reason: AudioCancelReason,
    ) -> Result<bool, String>;
    fn take_interrupted_dictation(
        &self,
        recording_id: u64,
    ) -> Option<audio_lifecycle::InterruptedCapture>;
    /// `audio_lifecycle::is_dictation_cleanup_pending`: the previous owner is
    /// still tearing down. Read by the start under the dictation lock.
    fn is_dictation_cleanup_pending(&self, recording_id: u64) -> bool;
}

/// The keyboard module's state the commands read or push, and the gesture
/// stamps it leaves for a stop. The detector logic is not touched.
pub(super) trait KeyboardPort: Send + Sync {
    fn is_app_disabled(&self) -> bool;
    fn listener_generation(&self) -> u64;
    fn set_processing(&self, processing: bool);
    fn hotkey_release_take_for(
        &self,
        recording_id: u64,
        stop_entered_at: std::time::Instant,
    ) -> Option<crate::hotkey_release::HotkeyRelease>;
    fn stop_gesture_take_for(
        &self,
        recording_id: u64,
        entered: std::time::Instant,
    ) -> Option<crate::stop_gesture::StopStamp>;
}

/// The Accessibility sample of the paste target.
pub(super) trait TargetSnapshotPort: Send + Sync {
    fn capture_delivery_target_snapshot(&self) -> crate::frontmost::DeliveryTargetSnapshot;
}

/// Smart-destination capture, which needs the concrete `AppHandle`.
pub(super) trait SmartDestinationPort: Send + Sync {
    fn capture<'a>(
        &'a self,
        expected: &'a crate::frontmost::DeliveryTargetSnapshot,
        enabled: bool,
    ) -> HostFuture<'a, crate::smart_destination::Snapshot>;
}

/// The performance ledger and begin-coordinator entry points the commands call
/// directly. The stage guard is built from `State::performance` as before.
pub(super) trait PerformancePort: Send + Sync {
    fn register_production(
        &self,
        recording_id: u64,
        selection: crate::performance_metrics::production::MicrophoneSelectionV1,
        kind: Option<crate::performance_metrics::production::MicrophoneKindV1>,
    );
    fn production_context(&self, recording_id: u64, context: &DictationContextSnapshot);
    fn production_sample_count(&self, recording_id: u64, count: usize);
    fn begin_register(&self, recording_id: u64);
    fn begin_abort(&self, recording_id: u64);
    fn spawn_begin(&self, recording_id: u64, runtimes: Vec<RuntimeIdentityV1>, started_at_ms: i64);
    fn complete(
        &self,
        recording_id: u64,
        outcome: RunOutcomeV1,
        stages: Vec<StageTimingV1>,
        input: Option<ContentFreeInputSummaryV1>,
        runtimes: Option<Vec<RuntimeIdentityV1>>,
    );
}

/// The production host: a concrete `AppHandle`. Every port forwards to the
/// function the command called before this seam existed.
pub(super) struct TauriHost {
    app: tauri::AppHandle,
    supervisor: Arc<TauriSupervisor>,
    /// The command's own `State::performance` handle (an `Arc` clone), so the
    /// performance port never goes through Tauri's managed-state map, which
    /// takes a lock the old code did not.
    performance: crate::performance_metrics::PerformanceMetrics,
}

impl TauriHost {
    pub(super) fn new(
        app: tauri::AppHandle,
        performance: crate::performance_metrics::PerformanceMetrics,
    ) -> Self {
        Self {
            supervisor: Arc::new(TauriSupervisor(app.clone())),
            app,
            performance,
        }
    }
}

struct TauriSupervisor(tauri::AppHandle);

struct TauriKeyboard;

struct TauriAx;

impl DictationHost for TauriHost {
    fn emit_json(&self, event: &str, payload: serde_json::Value) -> Result<(), HostEmitError> {
        self.app
            .emit(event, payload)
            .map_err(|e| HostEmitError(e.to_string()))
    }

    fn supervisor(&self) -> Arc<dyn SupervisorPort> {
        Arc::clone(&self.supervisor) as Arc<dyn SupervisorPort>
    }

    fn keyboard(&self) -> &dyn KeyboardPort {
        &TauriKeyboard
    }

    fn ax(&self) -> &dyn TargetSnapshotPort {
        &TauriAx
    }

    fn smart_destination(&self) -> &dyn SmartDestinationPort {
        self
    }

    fn performance(&self) -> &dyn PerformancePort {
        self
    }

    fn microphone_transition<'a>(
        &'a self,
        state: &'a State,
    ) -> HostFuture<'a, Result<tokio::sync::MutexGuard<'a, ()>, String>> {
        Box::pin(
            crate::commands::microphone_preview::transition_for_dictation_start(&self.app, state),
        )
    }

    fn resolve_smart_auto(
        &self,
        request: &SmartAutoRequest,
    ) -> Result<crate::microphone_auto::SmartAutoSelection, String> {
        crate::audio_inventory::resolve_smart_auto(request)
    }

    fn refresh_expired_ide_context(&self, app_state: &AppState, bundle_id: Option<&str>) {
        refresh_expired_ide_context(&self.app, app_state, bundle_id)
    }

    fn emit_smart_destination(
        &self,
        app_state: &AppState,
        recording_id: u64,
        context: &DictationContextSnapshot,
        accepted_target: &crate::frontmost::DeliveryTargetSnapshot,
    ) {
        emit_smart_destination(&self.app, app_state, recording_id, context, accepted_target)
    }

    fn publish_recording_ready_if_audio_ready(
        &self,
        recording_id: u64,
        context_ready_at: std::time::Instant,
        ready_op: crate::core_history::ContinuationGuard,
    ) -> bool {
        publish_recording_ready_if_audio_ready(&self.app, recording_id, context_ready_at, ready_op)
    }

    fn spawn_model_preparation(&self, model_name: String, recording_id: u64) {
        spawn_model_preparation(self.app.clone(), model_name, recording_id)
    }

    fn claim_preview_hide(
        &self,
        recording_id: u64,
    ) -> Option<crate::commands::dictation_preview::PreviewHideOwner> {
        crate::commands::dictation_preview::PreviewHideOwner::claim(&self.app, recording_id)
    }

    fn run_transcription_pipeline<'a>(
        &'a self,
        samples: &'a [f32],
        app_state: &'a AppState,
        recording_id: u64,
        performance_guard: &'a mut NativeDictationPerformanceGuard,
        context: Arc<DictationContextSnapshot>,
        stop: PipelineStopContext,
    ) -> HostFuture<'a, Result<PipelineResult, String>> {
        Box::pin(run_transcription_pipeline(
            samples,
            &self.app,
            app_state,
            recording_id,
            performance_guard,
            context,
            stop,
        ))
    }
}

impl SupervisorPort for TauriSupervisor {
    fn start_dictation_recording(
        &self,
        device_id: Option<String>,
        recording_id: u64,
        origin: &str,
        gesture: audio_lifecycle::gesture::StartGesture,
    ) -> Result<(), AudioStartError> {
        audio_lifecycle::start_dictation_recording(
            self.0.clone(),
            device_id,
            recording_id,
            origin,
            gesture,
        )
    }

    fn start_dictation_recording_after_preview(
        &self,
        device_id: Option<String>,
        recording_id: u64,
        origin: &str,
        gesture: audio_lifecycle::gesture::StartGesture,
    ) -> Result<(), AudioStartError> {
        audio_lifecycle::start_dictation_recording_after_preview(
            self.0.clone(),
            device_id,
            recording_id,
            origin,
            gesture,
        )
    }

    fn stop_dictation_recording(
        &self,
        recording_id: u64,
        trigger: audio_lifecycle::DictationStopTrigger,
        released_at: Option<std::time::Instant>,
    ) -> Result<Vec<f32>, String> {
        audio_lifecycle::stop_dictation_recording(recording_id, trigger, released_at)
    }

    fn cancel_dictation_capture(
        &self,
        recording_id: u64,
        reason: AudioCancelReason,
    ) -> Result<bool, String> {
        audio_lifecycle::cancel_dictation_capture(recording_id, reason)
    }

    fn take_interrupted_dictation(
        &self,
        recording_id: u64,
    ) -> Option<audio_lifecycle::InterruptedCapture> {
        audio_lifecycle::take_interrupted_dictation(recording_id)
    }

    fn is_dictation_cleanup_pending(&self, recording_id: u64) -> bool {
        audio_lifecycle::is_dictation_cleanup_pending(recording_id)
    }
}

impl KeyboardPort for TauriKeyboard {
    fn is_app_disabled(&self) -> bool {
        keyboard::is_app_disabled()
    }

    fn listener_generation(&self) -> u64 {
        keyboard::listener_generation()
    }

    fn set_processing(&self, processing: bool) {
        keyboard::set_processing(processing)
    }

    fn hotkey_release_take_for(
        &self,
        recording_id: u64,
        stop_entered_at: std::time::Instant,
    ) -> Option<crate::hotkey_release::HotkeyRelease> {
        crate::hotkey_release::take_for(recording_id, stop_entered_at)
    }

    fn stop_gesture_take_for(
        &self,
        recording_id: u64,
        entered: std::time::Instant,
    ) -> Option<crate::stop_gesture::StopStamp> {
        crate::stop_gesture::take_for(recording_id, entered)
    }
}

impl TargetSnapshotPort for TauriAx {
    fn capture_delivery_target_snapshot(&self) -> crate::frontmost::DeliveryTargetSnapshot {
        crate::frontmost::capture_delivery_target_snapshot()
    }
}

impl SmartDestinationPort for TauriHost {
    fn capture<'a>(
        &'a self,
        expected: &'a crate::frontmost::DeliveryTargetSnapshot,
        enabled: bool,
    ) -> HostFuture<'a, crate::smart_destination::Snapshot> {
        Box::pin(crate::smart_destination::capture(
            &self.app, expected, enabled,
        ))
    }
}

impl PerformancePort for TauriHost {
    fn register_production(
        &self,
        recording_id: u64,
        selection: crate::performance_metrics::production::MicrophoneSelectionV1,
        kind: Option<crate::performance_metrics::production::MicrophoneKindV1>,
    ) {
        self.performance
            .register_production(recording_id, selection, kind)
    }

    fn production_context(&self, recording_id: u64, context: &DictationContextSnapshot) {
        self.performance.production_context(recording_id, context)
    }

    fn production_sample_count(&self, recording_id: u64, count: usize) {
        self.performance
            .production_sample_count(recording_id, count)
    }

    fn begin_register(&self, recording_id: u64) {
        dictation_performance_begins().register(recording_id)
    }

    fn begin_abort(&self, recording_id: u64) {
        dictation_performance_begins().abort(recording_id)
    }

    fn spawn_begin(&self, recording_id: u64, runtimes: Vec<RuntimeIdentityV1>, started_at_ms: i64) {
        spawn_native_dictation_performance_begin(
            self.performance.clone(),
            recording_id,
            runtimes,
            started_at_ms,
        )
    }

    fn complete(
        &self,
        recording_id: u64,
        outcome: RunOutcomeV1,
        stages: Vec<StageTimingV1>,
        input: Option<ContentFreeInputSummaryV1>,
        runtimes: Option<Vec<RuntimeIdentityV1>>,
    ) {
        complete_dictation_performance(
            &self.performance,
            recording_id,
            outcome,
            stages,
            input,
            runtimes,
        )
    }
}
