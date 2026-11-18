//! `DictationCore::handle` (plan §6 Stage 2c, appendix C1.2): the single entry
//! that routes a typed [`Input`] to the one `decide_*` function that owns it.
//!
//! The shadow comparison (C8.3) calls this to re-decide a recorded input off
//! thread, and the cutover PRs (C9.7) call it from the persisted-core entries, so
//! there is exactly one place that says which decide function a site uses. It adds
//! no behaviour: every arm is one call, and the routing follows the shell's own
//! choices at each site:
//!
//! * a site with a D-less entry and a D entry (`202`/`202n`, the `204b` D emit and
//!   its D-less tail) picks the entry the input's own facts select, exactly as
//!   [`Input::site`] does, so the decision's row belongs to the input's site;
//! * a `*_nolock` entry gets the [`LockFreeFacts`] of the view (M2 and M3), the
//!   atomics the shell reads for it, never M1 or M4;
//! * an input that still lacks an observation answer returns
//!   [`crate::decision::Verdict::NeedObservation`] untouched: the caller answers it
//!   and calls again, or (the shadow) reports it as a missing observation.
//!
//! [`DictationCore::project`] (state.rs) is the other half of the legacy path: it
//! builds the view this method is called on.

use crate::cancel::{decide_cancel_capture_result_nolock, decide_cancel_request};
use crate::decision::Decision;
use crate::ids::Now;
use crate::inputs::{Input, SupervisorEvent};
use crate::inputs::{PostKind, StopContextInput};
use crate::lifecycle::{
    decide_sup_accepted_nolock, decide_sup_cleanup_finished, decide_sup_idle,
    decide_sup_init_failed_nolock, decide_sup_interrupted_nolock,
    decide_sup_interrupted_short_idle, decide_sup_ready, decide_sup_recovering,
    decide_sup_stalled_nolock, decide_sup_still_connecting, SupInitFailedInput,
    SupInterruptedInput, SupReadyInput, SupRecoveringInput,
};
use crate::process_audio::{
    decide_process_audio_destination, decide_process_audio_finish, decide_process_audio_start,
};
use crate::start::{
    decide_ready_publish, decide_start_capture_result, decide_start_context,
    decide_start_destination, decide_start_flight_end, decide_start_request,
};
use crate::state::DictationCore;
use crate::stop::{
    decide_guard_finish, decide_pipeline_idle_emit, decide_pipeline_idle_emit_tail,
    decide_stop_capture_result, decide_stop_context, decide_stop_context_nolock,
    decide_stop_request,
};

impl DictationCore {
    /// Decide `input` against this view: the one entry of the core.
    ///
    /// Pure like every `decide_*`: it mutates nothing, performs no I/O and reads no
    /// clock (`now` is only the shell's stamp, which no decision branches on).
    pub fn handle(&self, input: Input, now: Now) -> Decision {
        let facts = self.lock_free_facts();
        match input {
            Input::StartRequest(i) => decide_start_request(self, i, now),
            Input::StartCaptureResult(i) => decide_start_capture_result(self, i, now),
            Input::StartContext(i) => decide_start_context(self, i, now),
            Input::ReadyPublish(i) => decide_ready_publish(self, i, now),
            Input::StopRequest(i) => decide_stop_request(self, i, now),
            Input::StopContext(i) => self.handle_stop_context(i, now),
            Input::StopCaptureResult(i) => decide_stop_capture_result(self, i, now),
            Input::GuardFinish(i) => decide_guard_finish(self, i, now),
            Input::PipelineIdleEmit(i) => {
                // The stop command's tail (a result in hand) takes no D; the
                // in-D emit of the stop and the legacy import carries no result.
                if i.kind == PostKind::StopPost && i.result.is_some() {
                    decide_pipeline_idle_emit_tail(i, now)
                } else {
                    decide_pipeline_idle_emit(self, i, now)
                }
            }
            Input::CancelRequest(i) => decide_cancel_request(self, i, now),
            Input::CancelCaptureResult(i) => decide_cancel_capture_result_nolock(facts, i, now),
            Input::Supervisor(event) => match event {
                SupervisorEvent::Accepted { rid } => decide_sup_accepted_nolock(facts, rid, now),
                SupervisorEvent::Ready {
                    rid,
                    context_present,
                    ready_received,
                } => decide_sup_ready(
                    self,
                    SupReadyInput {
                        rid,
                        context_present: Some(context_present),
                        ready_received,
                    },
                    now,
                ),
                SupervisorEvent::StillConnecting { rid } => {
                    decide_sup_still_connecting(self, rid, now)
                }
                SupervisorEvent::Recovering { rid, reason } => {
                    decide_sup_recovering(self, SupRecoveringInput { rid, reason }, now)
                }
                SupervisorEvent::CleanupFinished { rid } => {
                    decide_sup_cleanup_finished(self, rid, now)
                }
                SupervisorEvent::Idle { rid } => decide_sup_idle(self, rid, now),
                SupervisorEvent::InitializationFailed {
                    rid,
                    kind,
                    error_code,
                    recovery_reason,
                } => decide_sup_init_failed_nolock(
                    facts,
                    SupInitFailedInput {
                        rid,
                        kind,
                        error_code,
                        recovery_reason,
                    },
                    now,
                ),
                SupervisorEvent::RecoveryStalled { rid } => {
                    decide_sup_stalled_nolock(facts, rid, now)
                }
                SupervisorEvent::Interrupted {
                    rid,
                    delivered_samples,
                    duration_ms,
                    reason,
                } => decide_sup_interrupted_nolock(
                    facts,
                    SupInterruptedInput {
                        rid,
                        delivered_samples,
                        duration_ms,
                        reason,
                    },
                    now,
                ),
                SupervisorEvent::InterruptedShortIdle { rid } => {
                    decide_sup_interrupted_short_idle(self, rid, now)
                }
            },
            Input::ProcessAudioStart(i) => decide_process_audio_start(self, i, now),
            Input::ProcessAudioFinish(i) => decide_process_audio_finish(self, i, now),
            Input::StartDestination(i) => {
                decide_start_destination(self, i.rid, i.smart_enabled, now)
            }
            Input::StartFlightEnd(i) => decide_start_flight_end(i.rid, now),
            Input::ProcessAudioDestination(i) => {
                decide_process_audio_destination(self, i.rid, i.smart_enabled, now)
            }
        }
    }

    /// Site 202 / 202n: the `[pre]` context read selects the D-less entry
    /// (R202.1-.2) or the one that takes D (R202.3-.5), the same split
    /// [`Input::site`] makes.
    fn handle_stop_context(&self, input: StopContextInput, now: Now) -> Decision {
        if input.context_present {
            decide_stop_context_nolock(self.lock_free_facts(), input, now)
        } else {
            decide_stop_context(self, input, now)
        }
    }
}

#[cfg(test)]
mod tests;
