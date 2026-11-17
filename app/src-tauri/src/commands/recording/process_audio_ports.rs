//! The host seam of the legacy `process_audio` command (sites 301 and 302).
//!
//! `process_audio` takes a concrete `tauri::AppHandle` (the Wry runtime), which no
//! test can build off the main thread. Like the three dictation commands
//! (`recording_host.rs`), the command keeps its `#[tauri::command]` wrapper and
//! calls an inner function over `&dyn ImportedAudioHost`. [`TauriImportHost`] is the
//! only implementation compiled outside tests and every port forwards to the
//! function the command called before the seam existed, so statement order and
//! behaviour are unchanged. The state is passed through (`&State`); no port does a
//! managed-state lookup.

use super::dictation_host::{HostEmitError, HostFuture};
use super::{DictationContextSnapshot, PipelineResult, PipelineStopContext, State};
use crate::performance_metrics::PerformanceRunGuard;
use crate::state::AppState;
use std::sync::Arc;
use tauri::Emitter;

/// What the legacy `process_audio` needs from the application handle.
pub(super) trait ImportedAudioHost: Send + Sync + 'static {
    /// `AppHandle::emit` for an already serialized payload.
    fn emit_json(&self, event: &str, payload: serde_json::Value) -> Result<(), HostEmitError>;
    /// `microphone_preview::transition_after_stopping_preview`.
    fn transition<'a>(
        &'a self,
        state: &'a State,
    ) -> HostFuture<'a, Result<tokio::sync::MutexGuard<'a, ()>, String>>;
    /// `frontmost::capture_delivery_target_snapshot`.
    fn capture_delivery_target_snapshot(&self) -> crate::frontmost::DeliveryTargetSnapshot;
    /// `keyboard::set_processing`.
    fn set_processing(&self, processing: bool);
    /// `smart_destination::capture`.
    fn capture_smart_destination<'a>(
        &'a self,
        expected: &'a crate::frontmost::DeliveryTargetSnapshot,
        enabled: bool,
    ) -> HostFuture<'a, crate::smart_destination::Snapshot>;
    /// `emit_smart_destination`.
    fn emit_smart_destination(
        &self,
        app_state: &AppState,
        recording_id: u64,
        context: &DictationContextSnapshot,
        accepted_target: &crate::frontmost::DeliveryTargetSnapshot,
    );
    /// `run_transcription_pipeline`.
    fn run_transcription_pipeline<'a>(
        &'a self,
        samples: &'a [f32],
        app_state: &'a AppState,
        recording_id: u64,
        performance_guard: &'a mut PerformanceRunGuard,
        context: Arc<DictationContextSnapshot>,
        stop: PipelineStopContext,
    ) -> HostFuture<'a, Result<PipelineResult, String>>;
}

/// The production host: a concrete `AppHandle`.
pub(super) struct TauriImportHost {
    app: tauri::AppHandle,
}

impl TauriImportHost {
    pub(super) fn new(app: tauri::AppHandle) -> Self {
        Self { app }
    }
}

impl ImportedAudioHost for TauriImportHost {
    fn emit_json(&self, event: &str, payload: serde_json::Value) -> Result<(), HostEmitError> {
        self.app
            .emit(event, payload)
            .map_err(|e| HostEmitError(e.to_string()))
    }

    fn transition<'a>(
        &'a self,
        state: &'a State,
    ) -> HostFuture<'a, Result<tokio::sync::MutexGuard<'a, ()>, String>> {
        Box::pin(
            crate::commands::microphone_preview::transition_after_stopping_preview(
                &self.app, state,
            ),
        )
    }

    fn capture_delivery_target_snapshot(&self) -> crate::frontmost::DeliveryTargetSnapshot {
        crate::frontmost::capture_delivery_target_snapshot()
    }

    fn set_processing(&self, processing: bool) {
        crate::keyboard::set_processing(processing);
    }

    fn capture_smart_destination<'a>(
        &'a self,
        expected: &'a crate::frontmost::DeliveryTargetSnapshot,
        enabled: bool,
    ) -> HostFuture<'a, crate::smart_destination::Snapshot> {
        Box::pin(crate::smart_destination::capture(
            &self.app, expected, enabled,
        ))
    }

    fn emit_smart_destination(
        &self,
        app_state: &AppState,
        recording_id: u64,
        context: &DictationContextSnapshot,
        accepted_target: &crate::frontmost::DeliveryTargetSnapshot,
    ) {
        super::emit_smart_destination(&self.app, app_state, recording_id, context, accepted_target)
    }

    fn run_transcription_pipeline<'a>(
        &'a self,
        samples: &'a [f32],
        app_state: &'a AppState,
        recording_id: u64,
        performance_guard: &'a mut PerformanceRunGuard,
        context: Arc<DictationContextSnapshot>,
        stop: PipelineStopContext,
    ) -> HostFuture<'a, Result<PipelineResult, String>> {
        Box::pin(super::run_transcription_pipeline(
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
