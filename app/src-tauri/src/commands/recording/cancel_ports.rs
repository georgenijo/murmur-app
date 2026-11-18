//! The production [`Ports`] of the cancel command (Stage 2a-3, sites 205 and
//! 206): each effect is today's statement, run by the executor at the placement
//! the decision gave it.
//!
//! The ports take no lock of their own: the field effects (`ST`, `CX-`, `MC`) go
//! through the `FieldAdapter` on the guard the shell passes the executor, and every
//! other effect is the statement the cancel command ran inline before. Every
//! result a port reports is the real one (an emit's `Result`, the capture
//! cancel's `Err`, the terminal claim), never a constant.
//!
//! There is no `outside_core` span clearing (decision D-S2-2): `IN_CORE` stays set
//! across every port call, so an inline callback that re-enters a family entry is
//! refused there, before any lock.

use super::{emit_dictation_terminal, DictationHost, State};
use crate::dictation_context::DictationContextSnapshot;
use crate::dictation_executor::convert::{
    cancel_reason_from_core, error_code_from_core, terminal_outcome_from_core,
};
use crate::dictation_executor::executor::{
    Executor, PhaseCtx, PhaseReport, PortFailure, PortResult, Ports,
};
use crate::dictation_telemetry::DictationTerminalOutcome;
use crate::performance_metrics::{PerformanceStageV1, RunOutcomeV1};
use crate::state::{AppState, DictationState};
use murmur_dictation_core::decision::{Decision, InvalidKind};
use murmur_dictation_core::effects::Placement;
use murmur_dictation_core::effects::{Effect, EventKind, PerfCause, PerfOutcome, PerfStage};
use murmur_dictation_core::ids::RecordingId;
use std::cell::RefCell;
use std::sync::Arc;

/// Wrap a decision in an executor. A cancel decision has no awaited effect and the
/// crate tests validate every row, so a failure here is a programming error.
pub(super) fn executor<'a>(ports: &'a dyn Ports, decision: Decision) -> Executor<'a> {
    Executor::new(ports, decision, false)
        .expect("a cancel decision is structurally valid (crate row tests)")
}

/// Run a (pure) cancel decide function under the re-entrancy guard
/// (`decide_guarded`): a nested call returns `Invalid(Reentrant)`.
pub(super) fn decide(decide: impl FnOnce() -> Decision) -> Decision {
    crate::dictation_executor::executor::decide_guarded(decide)
}

/// Run one placement. `dictation` is `Some` only for `In`: the D guard's contents
/// the shell holds. A cancel decision has no awaited effect (`CC` is blocking,
/// C4.1a), so the only other report is a refusal, which the caller propagates:
/// nothing was run.
pub(super) fn run_phase(
    executor: &mut Executor<'_>,
    phase: Placement,
    dictation: Option<&mut DictationState>,
    recording_id: u64,
) -> Result<(), InvalidKind> {
    let mut ctx = PhaseCtx {
        dictation,
        recording_id,
    };
    match executor.run(phase, &mut ctx) {
        PhaseReport::Done => Ok(()),
        PhaseReport::Invalid(kind) => {
            tracing::error!(
                target: "pipeline",
                recording_id,
                phase = phase.label(),
                refusal = ?kind,
                "cancel effects were not run: the executor refused the phase"
            );
            Err(kind)
        }
        PhaseReport::Awaiting(_) => {
            unreachable!("a cancel decision has no awaited effect (CC is blocking)")
        }
    }
}

// Test builds: the witness of the executor that ran site 206's `PT` phase, set by
// both callers (the cancel command and the stop command's R201.4), so a test
// sees the real result recorded for every effect the decision placed.
#[cfg(test)]
thread_local! {
    pub(super) static LAST_206_WITNESS: RefCell<Vec<crate::dictation_executor::executor::Witness>> =
        const { RefCell::new(Vec::new()) };
}

/// What the capture cancel (`CC`) reported.
#[derive(Debug)]
pub(super) enum CaptureCancelOutcome {
    /// `Ok(accepted)`.
    Done(bool),
    /// `Err(text)`.
    Failed(String),
}

pub(super) struct CancelPorts<'a> {
    host: &'a dyn DictationHost,
    state: &'a State,
    recording_id: u64,
    capture_cancel: RefCell<Option<CaptureCancelOutcome>>,
}

impl<'a> CancelPorts<'a> {
    pub(super) fn new(host: &'a dyn DictationHost, state: &'a State, recording_id: u64) -> Self {
        Self {
            host,
            state,
            recording_id,
            capture_cancel: RefCell::new(None),
        }
    }

    /// The capture cancel's result, once `CC` ran.
    pub(super) fn take_capture_cancel(&self) -> Option<CaptureCancelOutcome> {
        self.capture_cancel.borrow_mut().take()
    }
}

impl Ports for CancelPorts<'_> {
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
        match effect {
            Effect::EmitStatus { status, .. } => self
                .host
                .emit_event("recording-status-changed", status.as_str())
                .map_err(|_| PortFailure),
            Effect::EmitEvent(EventKind::Cancelled) => self
                .host
                .emit_event(
                    "recording-cancelled",
                    serde_json::json!({ "recordingId": self.recording_id }),
                )
                .map_err(|_| PortFailure),
            _ => Err(PortFailure),
        }
    }

    fn trace(&self, _effect: &Effect) -> PortResult {
        Err(PortFailure)
    }

    fn terminal(&self, effect: &Effect) -> PortResult {
        let Effect::Terminal(terminal) = effect else {
            return Err(PortFailure);
        };
        let outcome = terminal_outcome_from_core(terminal.outcome);
        let code = error_code_from_core(terminal.code);
        let claimed = emit_dictation_terminal(self.state, self.recording_id, outcome, code);
        if outcome == DictationTerminalOutcome::UserCancelledProcessing {
            tracing::info!(
                target: "pipeline",
                "cancel_native_recording: processing cancelled (recording_id={})",
                self.recording_id
            );
        }
        // The ledger's answer is the witness: `Lost` (another sink won) is a
        // failure of this claim, never a constant success.
        if claimed {
            Ok(())
        } else {
            Err(PortFailure)
        }
    }

    fn perf(&self, effect: &Effect) -> PortResult {
        let Effect::PerfComplete(pc) = effect else {
            return Err(PortFailure);
        };
        let stage = match pc.stage {
            PerfStage::InferenceDecode => PerformanceStageV1::InferenceDecode,
            PerfStage::CaptureFinalization => return Err(PortFailure),
        };
        match (pc.outcome, pc.cause) {
            (PerfOutcome::Cancelled, None) => {
                self.host.performance().complete(
                    self.recording_id,
                    RunOutcomeV1::Cancelled { stage },
                    Vec::new(),
                    None,
                    None,
                );
                Ok(())
            }
            (
                PerfOutcome::Cancelled,
                Some(PerfCause::AudioCaptureFailed | PerfCause::InternalEarlyExit),
            )
            | (PerfOutcome::Failed | PerfOutcome::NoSpeech | PerfOutcome::Completed, _) => {
                Err(PortFailure)
            }
        }
    }

    fn keyboard(&self, effect: &Effect) -> PortResult {
        let Effect::KeyboardProcessing(processing) = effect else {
            return Err(PortFailure);
        };
        self.host.keyboard().set_processing(*processing);
        Ok(())
    }

    fn claims(&self, _effect: &Effect) -> PortResult {
        Err(PortFailure)
    }

    fn blocking(&self, effect: &Effect) -> PortResult {
        let Effect::CaptureCancel { id, reason } = effect else {
            return Err(PortFailure);
        };
        // Cancel the exact owner even if readiness races with this command after
        // the status snapshot.
        match self
            .host
            .supervisor()
            .cancel_dictation_capture(id.0, cancel_reason_from_core(*reason))
        {
            Ok(accepted) => {
                *self.capture_cancel.borrow_mut() = Some(CaptureCancelOutcome::Done(accepted));
                Ok(())
            }
            Err(error) => {
                *self.capture_cancel.borrow_mut() = Some(CaptureCancelOutcome::Failed(error));
                Err(PortFailure)
            }
        }
    }

    fn detached(&self, _effect: &Effect) -> PortResult {
        Err(PortFailure)
    }
}
