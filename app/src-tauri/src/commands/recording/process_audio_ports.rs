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

/// An owned `emit_json` (see [`ImportedAudioHost::idle_emitter`]).
pub(super) type OwnedJsonEmit =
    Box<dyn Fn(&str, serde_json::Value) -> Result<(), HostEmitError> + Send + 'static>;

/// What the legacy `process_audio` needs from the application handle.
pub(super) trait ImportedAudioHost: Send + Sync + 'static {
    /// `AppHandle::emit` for an already serialized payload.
    fn emit_json(&self, event: &str, payload: serde_json::Value) -> Result<(), HostEmitError>;
    /// An [`Self::emit_json`] that owns what it needs (an `AppHandle` clone), for the
    /// idle emit of a parse-failure finish: a finish deferred inside a core span
    /// carries its emit as data and makes it at the drain (D-S2-7).
    fn idle_emitter(&self) -> OwnedJsonEmit;
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

    fn idle_emitter(&self) -> OwnedJsonEmit {
        let app = self.app.clone();
        Box::new(move |event, payload| {
            app.emit(event, payload)
                .map_err(|e| HostEmitError(e.to_string()))
        })
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

// ------------------------------------------------------------ the effects

use crate::dictation_context::DictationContextSnapshot as ContextSnapshot;
use crate::dictation_executor::executor::{
    Executor, PhaseCtx, PhaseReport, PortFailure, PortResult, Ports,
};
use crate::state::DictationState;
use murmur_dictation_core::decision::Decision;
use murmur_dictation_core::effects::{Effect, EventKind, Placement};
use murmur_dictation_core::ids::RecordingId;
use std::cell::{Cell, RefCell};

/// Wrap a decision in an executor. The 301 decision has no awaited effect and the
/// crate tests validate every row, so a failure here is a programming error.
pub(super) fn executor<'a>(ports: &'a dyn Ports, decision: Decision) -> Executor<'a> {
    Executor::new(ports, decision, false)
        .expect("a process_audio decision is structurally valid (crate row tests)")
}

/// Run a (pure) decide function under the re-entrancy guard (C4.2).
pub(super) fn decide(decide: impl FnOnce() -> Decision) -> Decision {
    crate::dictation_executor::executor::decide_guarded(decide)
}

/// A decision made outside any core span is never `Invalid`.
pub(super) fn expect_valid(decision: &Decision) {
    if let murmur_dictation_core::decision::Verdict::Invalid(kind) = &decision.verdict {
        unreachable!("a process_audio decision outside any core span was refused: {kind:?}");
    }
}

/// Run one placement; the shell runs the phases in order outside any core span,
/// so a refused or out-of-order run cannot happen and is never ignored.
pub(super) fn run_phase(
    executor: &mut Executor<'_>,
    phase: Placement,
    dictation: Option<&mut DictationState>,
    recording_id: u64,
) {
    let mut ctx = PhaseCtx {
        dictation,
        recording_id,
    };
    if let PhaseReport::Invalid(kind) = executor.run(phase, &mut ctx) {
        unreachable!(
            "process_audio's {} phase was refused: {kind:?}",
            phase.label()
        );
    }
}

/// Keep `IN_CORE` set around a synchronous host call that can emit inline (a
/// resolver or helper outside the executor): a listener it invokes that re-enters
/// the core is refused instead of asking for a lock the caller may hold. The shell
/// is outside any core span here (it waited with `until_outside_core`); if it were
/// not, the flag is already set and stays so.
pub(super) fn in_core_span<R>(call: impl FnOnce() -> R) -> R {
    let _entry = crate::dictation_executor::executor::CoreEntry::enter().ok();
    call()
}

/// C4.1 step 0 / C4.2: the **first** statement of the command's entry, before any
/// lock or await. D-S2-3: Tauri's own `State` extraction in IPC dispatch is exempt;
/// the core never invokes an IPC command.
pub(super) fn refuse_reentrant() -> Result<(), String> {
    match crate::dictation_executor::executor::CoreEntry::enter() {
        Ok(entry) => {
            drop(entry);
            Ok(())
        }
        Err(kind) => {
            tracing::error!(
                target: "pipeline",
                refusal = ?kind,
                "re-entrant process_audio refused before any lock"
            );
            Err(format!(
                "The dictation core refused process_audio: {kind:?}"
            ))
        }
    }
}

/// Take T, never inside a core span (D-S2-2/3, T before D). An import that parked
/// on T and is polled again by a listener that holds D parks on every such poll
/// (its waker is woken at once) instead of taking T, and goes on when it is next
/// polled outside the span. After T it waits the span out once more before the
/// caller asks for D, as the stop family does after its T acquisition. Ready on
/// the first poll outside a span.
pub(super) async fn transition_outside_core<'a>(
    host: &'a dyn ImportedAudioHost,
    state: &'a State,
) -> Result<tokio::sync::MutexGuard<'a, ()>, String> {
    use std::task::Poll;
    let mut transition = host.transition(state);
    let held = std::future::poll_fn(|cx| {
        if crate::dictation_executor::executor::in_core() {
            cx.waker().wake_by_ref();
            return Poll::Pending;
        }
        transition.as_mut().poll(cx)
    })
    .await?;
    super::until_outside_core().await;
    Ok(held)
}

/// The ports of R301.5: `AX` (blocking) freezes the delivery target, `KB` and the
/// two emits run after D and T are released. The recording id is a shell local the
/// `ID+` effect produced (X4); the shell hands it over before `PostT`.
pub(super) struct ProcessAudioPorts<'a> {
    host: &'a dyn ImportedAudioHost,
    app_state: &'a AppState,
    recording_id: Cell<u64>,
    delivery_target: RefCell<Option<crate::frontmost::DeliveryTargetSnapshot>>,
}

impl<'a> ProcessAudioPorts<'a> {
    pub(super) fn new(host: &'a dyn ImportedAudioHost, app_state: &'a AppState) -> Self {
        Self {
            host,
            app_state,
            recording_id: Cell::new(0),
            delivery_target: RefCell::new(None),
        }
    }

    /// The id `ID+` allocated, for the `dictation-generation-started` payload.
    pub(super) fn bind_recording_id(&self, recording_id: u64) {
        self.recording_id.set(recording_id);
    }

    /// What `AX` froze at the accepted transition.
    pub(super) fn take_delivery_target(&self) -> Option<crate::frontmost::DeliveryTargetSnapshot> {
        self.delivery_target.borrow_mut().take()
    }
}

impl Ports for ProcessAudioPorts<'_> {
    fn app_state(&self) -> &AppState {
        self.app_state
    }

    fn speculative_flowing(&self) -> bool {
        false
    }

    fn context_snapshot(&self, _id: RecordingId) -> Option<Arc<ContextSnapshot>> {
        None
    }

    fn emit(&self, effect: &Effect) -> PortResult {
        match effect {
            Effect::EmitEvent(EventKind::GenStarted) => self
                .host
                .emit_json(
                    "dictation-generation-started",
                    serde_json::json!({ "recordingId": self.recording_id.get() }),
                )
                .map_err(|_| PortFailure),
            Effect::EmitStatus { status, .. } => self
                .host
                .emit_json(
                    "recording-status-changed",
                    serde_json::json!(status.as_str()),
                )
                .map_err(|_| PortFailure),
            _ => Err(PortFailure),
        }
    }

    fn trace(&self, _effect: &Effect) -> PortResult {
        Err(PortFailure)
    }

    fn terminal(&self, _effect: &Effect) -> PortResult {
        // `process_audio` never emits a terminal (A0 C2).
        Err(PortFailure)
    }

    fn perf(&self, _effect: &Effect) -> PortResult {
        Err(PortFailure)
    }

    fn keyboard(&self, effect: &Effect) -> PortResult {
        let Effect::KeyboardProcessing(processing) = effect else {
            return Err(PortFailure);
        };
        self.host.set_processing(*processing);
        Ok(())
    }

    fn claims(&self, _effect: &Effect) -> PortResult {
        Err(PortFailure)
    }

    fn blocking(&self, effect: &Effect) -> PortResult {
        let Effect::AxSnapshot(_) = effect else {
            return Err(PortFailure);
        };
        // Freeze the delivery target at the accepted transition, under the same
        // lock that commits Processing.
        *self.delivery_target.borrow_mut() = Some(self.host.capture_delivery_target_snapshot());
        Ok(())
    }

    fn detached(&self, _effect: &Effect) -> PortResult {
        Err(PortFailure)
    }
}
