//! The production [`Ports`] of the readiness publish (Stage 2a-4b, site 104,
//! rows R104.1-R104.6): `publish_recording_ready_with`'s statements, run by the
//! executor at the placement the decision gave them.
//!
//! Only R104.2 has effects: `In:ST→Recording` (the executor, through the
//! `FieldAdapter`), `In:TL`, `In:EM(recording)` and `In:SD`, all under the D hold
//! the publish already takes. `SD` runs through [`Ports::detached_under_d`]: the
//! partial ticker's `register` takes the held dictation state, exactly as the
//! closure the old body passed did. The ports are generic over the Tauri runtime
//! because the publish is (the golden harness drives it on the mock runtime).
//!
//! Every result a port reports is the real one: the emit's `Result`, not a
//! constant. `IN_CORE` stays set across the ports (decision D-S2-2); the log line
//! that followed the emit stays right after it, before `SD`.

use super::{register_partial_ticker_locked, State};
use crate::dictation_context::DictationContextSnapshot;
use crate::dictation_executor::executor::{PortFailure, PortResult, Ports};
use crate::state::{AppState, DictationState};
use crate::MutexExt;
use murmur_dictation_core::effects::Effect;
use murmur_dictation_core::ids::RecordingId;
use std::cell::RefCell;
use std::sync::Arc;
use tauri::Emitter;

/// What `publish_recording_ready_with` is handed to start the side tasks (the
/// incremental VAD and the partial ticker, whose `register` runs under D).
pub(super) type SpawnReadyTasks<'a, R> =
    Box<dyn FnOnce(&tauri::AppHandle<R>, u64, Box<dyn FnOnce() -> bool + '_>) + 'a>;

pub(super) struct ReadyPorts<'a, R: tauri::Runtime> {
    app_handle: &'a tauri::AppHandle<R>,
    state: &'a State,
    recording_id: u64,
    context_ready_at: std::time::Instant,
    spawn_ready_tasks: RefCell<Option<SpawnReadyTasks<'a, R>>>,
}

impl<'a, R: tauri::Runtime> ReadyPorts<'a, R> {
    pub(super) fn new(
        app_handle: &'a tauri::AppHandle<R>,
        state: &'a State,
        recording_id: u64,
        context_ready_at: std::time::Instant,
        spawn_ready_tasks: SpawnReadyTasks<'a, R>,
    ) -> Self {
        Self {
            app_handle,
            state,
            recording_id,
            context_ready_at,
            spawn_ready_tasks: RefCell::new(Some(spawn_ready_tasks)),
        }
    }
}

impl<R: tauri::Runtime> Ports for ReadyPorts<'_, R> {
    // Only the cancel's R205.5 pairs a `TERM` with its `PC` (PD-20).
    fn terminal_then_perf(&self, _terminal: &Effect, _perf: &Effect) -> (PortResult, PortResult) {
        crate::dictation_executor::executor::no_paired_terminal()
    }

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
        let Effect::EmitStatus { status, .. } = effect else {
            return Err(PortFailure);
        };
        let emitted = self
            .app_handle
            .emit("recording-status-changed", status.as_str());
        tracing::info!(
            target: "pipeline",
            event_code = "audio.recording_status_timing",
            recording_id = self.recording_id,
            context_to_status_emit_ms = self.context_ready_at.elapsed().as_millis() as u64,
            "recording status emitted after context ready"
        );
        emitted.map_err(|_| PortFailure)
    }

    fn trace(&self, _effect: &Effect) -> PortResult {
        Err(PortFailure)
    }

    fn terminal(&self, _effect: &Effect) -> PortResult {
        Err(PortFailure)
    }

    fn perf(&self, _effect: &Effect) -> PortResult {
        Err(PortFailure)
    }

    fn keyboard(&self, _effect: &Effect) -> PortResult {
        Err(PortFailure)
    }

    fn claims(&self, effect: &Effect) -> PortResult {
        match effect {
            Effect::TouchLastTranscription => {
                *self.state.app_state.last_transcription_at.lock_or_recover() =
                    Some(std::time::Instant::now());
                Ok(())
            }
            _ => Err(PortFailure),
        }
    }

    fn blocking(&self, _effect: &Effect) -> PortResult {
        Err(PortFailure)
    }

    /// `SD` always runs `In` here (R104.2), so it arrives through
    /// [`Ports::detached_under_d`].
    fn detached(&self, _effect: &Effect) -> PortResult {
        Err(PortFailure)
    }

    fn detached_under_d(&self, effect: &Effect, dictation: &DictationState) -> PortResult {
        let Effect::StartSideTasks = effect else {
            return Err(PortFailure);
        };
        let spawn = self
            .spawn_ready_tasks
            .borrow_mut()
            .take()
            .ok_or(PortFailure)?;
        let app_state = &self.state.app_state;
        let recording_id = self.recording_id;
        spawn(
            self.app_handle,
            recording_id,
            Box::new(|| register_partial_ticker_locked(app_state, dictation, recording_id)),
        );
        Ok(())
    }

    /// R104.2's `SD` with the flight's slots (see `LifecyclePorts::detached_observed`).
    #[cfg(feature = "dictation-shadow")]
    fn detached_under_d_observed(
        &self,
        effect: &Effect,
        dictation: &DictationState,
        slots: &crate::dictation_executor::shadow::ShadowSlots,
    ) -> PortResult {
        let Effect::StartSideTasks = effect else {
            return self.detached_under_d(effect, dictation);
        };
        let spawn = self
            .spawn_ready_tasks
            .borrow_mut()
            .take()
            .ok_or(PortFailure)?;
        let app_state = &self.state.app_state;
        let recording_id = self.recording_id;
        let gate = std::cell::Cell::new(None);
        spawn(
            self.app_handle,
            recording_id,
            Box::new(|| {
                super::register_partial_ticker_locked_observed(
                    app_state,
                    dictation,
                    recording_id,
                    &gate,
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
