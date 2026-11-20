//! The production [`Ports`] of the supervisor-lifecycle sites (Stage 2a-1,
//! sites 207-216): each effect is today's statement, run by the executor at the
//! placement the decision gave it.
//!
//! One `LifecyclePorts` serves one `handle_audio_lifecycle_with` call. It owns
//! the shell-local payloads an effect needs and the core never sees (X4): the
//! failure text and kind, the interruption counters, the recovery reason, and the
//! two spawn closures (`RS`, `SD`). It takes no lock of its own: the field effects
//! (`ST`, `CX-`) go through the `FieldAdapter` on the guard the shell passes the
//! executor, and every other effect is the statement the arm ran inline before.

use super::{
    complete_dictation_performance, emit_dictation_presentation, emit_dictation_terminal,
    register_partial_ticker_locked, DictationPresentation, State,
};
use crate::audio::AudioFailureKind;
use crate::audio_lifecycle::{self, AudioCancelReason};
use crate::core_history::ContinuationGuard;
use crate::dictation_context::DictationContextSnapshot;
use crate::dictation_executor::convert::{error_code_from_core, terminal_outcome_from_core};
use crate::dictation_executor::executor::{
    Executor, PhaseCtx, PhaseReport, PortFailure, PortResult, Ports,
};
use crate::performance_metrics::{
    ContentFreeInputSummaryV1, PerformanceStageV1, RunOutcomeV1, StableRunErrorV1,
};
use crate::state::{AppState, DictationState};
use crate::{keyboard, MutexExt};
use murmur_dictation_core::decision::Decision;
use murmur_dictation_core::effects::{
    Effect, EventKind, PerfCause, PerfOutcome, PerfStage, Placement, PresentationKind, TakeMode,
};
use murmur_dictation_core::ids::RecordingId;
use std::cell::{Cell, RefCell};
use std::sync::Arc;
use tauri::{Emitter, Manager};

type SpawnReadyTasks<'a, R> =
    Box<dyn FnOnce(tauri::AppHandle<R>, u64, Box<dyn FnOnce() -> bool>) + 'a>;
type TranscribeInterruption<'a, R> =
    Box<dyn FnOnce(tauri::AppHandle<R>, u64, ContinuationGuard) + 'a>;

/// The payloads of the event being handled, bound by the shell (X4).
#[derive(Default)]
pub(super) struct LifecyclePayload {
    /// `Recovering` reason.
    pub(super) reason: Option<AudioCancelReason>,
    /// `InitializationFailed` text and kind.
    pub(super) failure: Option<(String, AudioFailureKind)>,
    /// `Interrupted` reason, delivered samples and duration.
    pub(super) interrupted: Option<(AudioCancelReason, u64, u64)>,
}

pub(super) struct LifecyclePorts<'a, R: tauri::Runtime> {
    app: &'a tauri::AppHandle<R>,
    state: &'a State,
    recording_id: u64,
    payload: RefCell<LifecyclePayload>,
    /// The `autoTranscribe` flag of the `Interrupted` event just emitted, for its
    /// paired presentation trace.
    auto_transcribe: Cell<bool>,
    transcribe_interruption: RefCell<Option<TranscribeInterruption<'a, R>>>,
    /// The registered `AutoStop` op of R215.2 until `RS` hands it to the spawned
    /// stop. Dropped with the ports if `RS` never runs, which retires the op.
    auto_stop_owner: RefCell<Option<ContinuationGuard>>,
    spawn_ready_tasks: RefCell<Option<SpawnReadyTasks<'a, R>>>,
}

impl<'a, R: tauri::Runtime> LifecyclePorts<'a, R> {
    pub(super) fn new(
        app: &'a tauri::AppHandle<R>,
        state: &'a State,
        recording_id: u64,
        transcribe_interruption: impl FnOnce(tauri::AppHandle<R>, u64, ContinuationGuard) + 'a,
        spawn_ready_tasks: impl FnOnce(tauri::AppHandle<R>, u64, Box<dyn FnOnce() -> bool>) + 'a,
    ) -> Self {
        Self {
            app,
            state,
            recording_id,
            payload: RefCell::default(),
            auto_transcribe: Cell::new(false),
            transcribe_interruption: RefCell::new(Some(Box::new(transcribe_interruption))),
            auto_stop_owner: RefCell::new(None),
            spawn_ready_tasks: RefCell::new(Some(Box::new(spawn_ready_tasks))),
        }
    }

    /// Hold the `AutoStop` owner the shell registered after the decide (C4.3).
    pub(super) fn hold_auto_stop(&self, owner: ContinuationGuard) {
        *self.auto_stop_owner.borrow_mut() = Some(owner);
    }

    /// The H1 registration of the `AutoStop` owner held here, for the shadow (C8.2).
    #[cfg(feature = "dictation-shadow")]
    pub(super) fn auto_stop_registration(&self) -> Option<crate::dictation_shadow::types::LiveOp> {
        self.auto_stop_owner
            .borrow()
            .as_ref()
            .map(|owner| crate::dictation_executor::shadow::registered(owner.token()))
    }

    pub(super) fn set_payload(&self, payload: LifecyclePayload) {
        *self.payload.borrow_mut() = payload;
    }

    fn presentation(&self, kind: PresentationKind) -> Option<DictationPresentation> {
        match kind {
            PresentationKind::CleanupInProgress => Some(DictationPresentation::CleanupInProgress),
            PresentationKind::InitFailed => {
                let payload = self.payload.borrow();
                let (_, kind) = payload.failure.as_ref()?;
                Some(DictationPresentation::InitializationFailed(*kind))
            }
            PresentationKind::RecoveryStalled => Some(DictationPresentation::CleanupStalled),
            PresentationKind::Interrupted => Some(DictationPresentation::Interrupted {
                partial_transcription: self.auto_transcribe.get(),
            }),
        }
    }
}

/// Wrap a decision in an executor. A lifecycle decision has no awaited effect
/// and the crate tests validate every row, so a failure here is a programming
/// error.
pub(super) fn executor<'a>(ports: &'a dyn Ports, decision: Decision) -> Executor<'a> {
    Executor::new(ports, decision, false)
        .expect("a lifecycle decision is structurally valid (crate row tests)")
}

#[cfg(test)]
thread_local! {
    /// Test builds: the executor's witness (legend code, outcome), appended after
    /// every phase run on this thread (a test clears it first).
    pub(super) static LAST_WITNESS: std::cell::RefCell<
        Vec<(&'static str, crate::dictation_executor::executor::WitnessOutcome)>,
    > = const { std::cell::RefCell::new(Vec::new()) };
}

/// Run one placement. `dictation` is `Some` only for `In`: the D guard's contents
/// the shell holds. A refused or out-of-order run does nothing and is logged;
/// today's code has no such path.
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
    let report = executor.run(phase, &mut ctx);
    #[cfg(test)]
    LAST_WITNESS.with(|slot| {
        slot.borrow_mut().extend(
            executor
                .witness()
                .iter()
                .map(|witness| (witness.code, witness.outcome)),
        );
    });
    match report {
        PhaseReport::Done => {}
        PhaseReport::Awaiting(_) | PhaseReport::Invalid(_) => {
            tracing::error!(
                target: "pipeline",
                recording_id,
                phase = phase.label(),
                "lifecycle effects were not run: the executor refused the phase"
            );
        }
    }
}

impl<R: tauri::Runtime> Ports for LifecyclePorts<'_, R> {
    fn app_state(&self) -> &AppState {
        &self.state.app_state
    }

    fn speculative_flowing(&self) -> bool {
        false
    }

    fn context_snapshot(&self, _id: RecordingId) -> Option<Arc<DictationContextSnapshot>> {
        None
    }

    fn emit(&self, effect: &Effect) -> PortResult {
        let recording_id = self.recording_id;
        let result = match effect {
            Effect::EmitStatus { status, .. } => {
                self.app.emit("recording-status-changed", status.as_str())
            }
            Effect::EmitEvent(EventKind::InitStalled) => self.app.emit(
                "audio-initialization-stalled",
                serde_json::json!({ "recordingId": recording_id }),
            ),
            Effect::EmitEvent(EventKind::RecoveryStarted) => {
                let reason = self.payload.borrow().reason.ok_or(PortFailure)?;
                self.app.emit(
                    "audio-recovery-started",
                    serde_json::json!({
                        "recordingId": recording_id,
                        "reason": reason.as_str(),
                    }),
                )
            }
            Effect::EmitEvent(EventKind::Cancelled) => self.app.emit(
                "recording-cancelled",
                serde_json::json!({ "recordingId": recording_id }),
            ),
            Effect::EmitEvent(EventKind::InitFailed) => {
                let payload = self.payload.borrow();
                let (error, kind) = payload.failure.as_ref().ok_or(PortFailure)?;
                let (status_code, action_code) =
                    DictationPresentation::InitializationFailed(*kind).codes();
                self.app.emit(
                    "recording-initialization-failed",
                    serde_json::json!({
                        "recordingId": recording_id,
                        "error": error,
                        "errorKind": kind.as_str(),
                        "statusCode": status_code,
                        "actionCode": action_code,
                    }),
                )
            }
            Effect::EmitEvent(EventKind::CleanupFinished) => self.app.emit(
                "recording-cleanup-finished",
                serde_json::json!({ "recordingId": recording_id }),
            ),
            Effect::EmitEvent(EventKind::RecoveryStalled) => {
                let (status_code, action_code) = DictationPresentation::CleanupStalled.codes();
                self.app.emit(
                    "recording-recovery-stalled",
                    serde_json::json!({
                        "recordingId": recording_id,
                        "statusCode": status_code,
                        "actionCode": action_code,
                    }),
                )
            }
            Effect::EmitEvent(EventKind::Interrupted { auto_transcribe }) => {
                let (reason, delivered_samples, duration_ms) =
                    self.payload.borrow().interrupted.ok_or(PortFailure)?;
                self.auto_transcribe.set(*auto_transcribe);
                let (status_code, action_code) = DictationPresentation::Interrupted {
                    partial_transcription: *auto_transcribe,
                }
                .codes();
                self.app.emit(
                    "recording-interrupted",
                    serde_json::json!({
                        "recordingId": recording_id,
                        "reason": reason.as_str(),
                        "deliveredSamples": delivered_samples,
                        "durationMs": duration_ms,
                        "autoTranscribe": auto_transcribe,
                        "statusCode": status_code,
                        "actionCode": action_code,
                    }),
                )
            }
            _ => return Err(PortFailure),
        };
        result.map_err(|_| PortFailure)
    }

    fn trace(&self, effect: &Effect) -> PortResult {
        let Effect::Presentation { kind, .. } = effect else {
            return Err(PortFailure);
        };
        let presentation = self.presentation(*kind).ok_or(PortFailure)?;
        emit_dictation_presentation(self.recording_id, presentation);
        Ok(())
    }

    fn terminal(&self, effect: &Effect) -> PortResult {
        let Effect::Terminal(terminal) = effect else {
            return Err(PortFailure);
        };
        let claimed = emit_dictation_terminal(
            self.state,
            self.recording_id,
            terminal_outcome_from_core(terminal.outcome),
            error_code_from_core(terminal.code),
        );
        // D-S2-9: the ledger's answer is the witness, as in the cancel family. A
        // claim another sink already won (`Lost`) is recorded as failed, never as a
        // constant success.
        if claimed {
            Ok(())
        } else {
            Err(PortFailure)
        }
    }

    fn perf(&self, effect: &Effect) -> PortResult {
        match effect {
            Effect::ProductionCaptureFailure(_) => {
                let payload = self.payload.borrow();
                let (_, kind) = payload.failure.as_ref().ok_or(PortFailure)?;
                self.state
                    .performance
                    .production_capture_failure(self.recording_id, *kind);
                Ok(())
            }
            Effect::PerfComplete(pc) => {
                let stage = match pc.stage {
                    PerfStage::CaptureFinalization => PerformanceStageV1::CaptureFinalization,
                    PerfStage::InferenceDecode => return Err(PortFailure),
                };
                let outcome = match (pc.outcome, pc.cause) {
                    (PerfOutcome::Failed, Some(PerfCause::AudioCaptureFailed)) => {
                        RunOutcomeV1::Failed {
                            stage,
                            error_code: StableRunErrorV1::AudioCaptureFailed,
                        }
                    }
                    (PerfOutcome::Cancelled, None) => RunOutcomeV1::Cancelled { stage },
                    _ => return Err(PortFailure),
                };
                let input = if pc.with_audio_duration {
                    let (_, _, duration_ms) =
                        self.payload.borrow().interrupted.ok_or(PortFailure)?;
                    Some(ContentFreeInputSummaryV1::audio(duration_ms))
                } else {
                    None
                };
                complete_dictation_performance(
                    &self.state.performance,
                    self.recording_id,
                    outcome,
                    Vec::new(),
                    input,
                    None,
                );
                Ok(())
            }
            _ => Err(PortFailure),
        }
    }

    fn keyboard(&self, effect: &Effect) -> PortResult {
        let Effect::KeyboardProcessing(processing) = effect else {
            return Err(PortFailure);
        };
        keyboard::set_processing(*processing);
        Ok(())
    }

    fn claims(&self, effect: &Effect) -> PortResult {
        match effect {
            Effect::TouchLastTranscription => {
                *self.state.app_state.last_transcription_at.lock_or_recover() =
                    Some(std::time::Instant::now());
                Ok(())
            }
            Effect::AcceptTelemetry => {
                self.state
                    .app_state
                    .dictation_telemetry
                    .accepted(self.recording_id);
                self.state.dictation_diagnostics.claim(self.recording_id);
                Ok(())
            }
            Effect::TakeInterrupted {
                mode: TakeMode::Dispose,
                ..
            } => {
                if audio_lifecycle::take_interrupted_dictation(self.recording_id).is_some() {
                    self.state
                        .app_state
                        .note_interrupted_taken(self.recording_id);
                }
                Ok(())
            }
            _ => Err(PortFailure),
        }
    }

    fn blocking(&self, _effect: &Effect) -> PortResult {
        Err(PortFailure)
    }

    fn detached(&self, effect: &Effect) -> PortResult {
        match effect {
            Effect::SelfStop { .. } => {
                let spawn = self
                    .transcribe_interruption
                    .borrow_mut()
                    .take()
                    .ok_or(PortFailure)?;
                // The spawned stop owns the op from here (`RS`, C1.4.1 rule 4).
                let owner = self
                    .auto_stop_owner
                    .borrow_mut()
                    .take()
                    .ok_or(PortFailure)?;
                spawn(self.app.clone(), self.recording_id, owner);
                Ok(())
            }
            Effect::StartSideTasks => {
                let spawn = self
                    .spawn_ready_tasks
                    .borrow_mut()
                    .take()
                    .ok_or(PortFailure)?;
                let recording_id = self.recording_id;
                let register_app = self.app.clone();
                spawn(
                    self.app.clone(),
                    recording_id,
                    Box::new(move || {
                        let state = register_app.state::<State>();
                        let dictation = state.app_state.dictation.lock_or_recover();
                        register_partial_ticker_locked(&state.app_state, &dictation, recording_id)
                    }),
                );
                Ok(())
            }
            _ => Err(PortFailure),
        }
    }

    /// `SD` with the flight's slots: the same spawn, whose `register` closure notes the
    /// gate it asked, and the skip the spawner took when it never called it. Every other
    /// detached effect is [`Self::detached`] unchanged (D-S2c-9 c, f).
    #[cfg(feature = "dictation-shadow")]
    fn detached_observed(
        &self,
        effect: &Effect,
        slots: &crate::dictation_executor::shadow::ShadowSlots,
    ) -> PortResult {
        let Effect::StartSideTasks = effect else {
            return self.detached(effect);
        };
        let spawn = self
            .spawn_ready_tasks
            .borrow_mut()
            .take()
            .ok_or(PortFailure)?;
        let recording_id = self.recording_id;
        let register_app = self.app.clone();
        let gate = std::rc::Rc::new(std::cell::Cell::new(None));
        let noted = std::rc::Rc::clone(&gate);
        spawn(
            self.app.clone(),
            recording_id,
            Box::new(move || {
                let state = register_app.state::<State>();
                let dictation = state.app_state.dictation.lock_or_recover();
                super::register_partial_ticker_locked_observed(
                    &state.app_state,
                    &dictation,
                    recording_id,
                    &noted,
                )
            }),
        );
        slots.ticker.set(super::ticker_note(
            recording_id,
            crate::dictation_executor::shadow::take_ticker_skip(),
            gate.take(),
        ));
        Ok(())
    }
}
