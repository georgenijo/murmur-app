//! Stage 2a-3: the cancel-family decide functions (appendix C2.3 sites 205 and
//! 206; rows of C3.5 R205.1-R206.2).
//!
//! `decide_cancel_request` (205) is the one D critical section of the cancel
//! command (T held, D section 6385-6402 in the appendix numbering); the shell
//! calls it inline at today's line with D and T held exactly as today.
//! `decide_cancel_capture_result_nolock` (206) decides the result of the capture
//! cancel (`CC`) that R205.4 (cancel command) and R201.4 (stop command during
//! `Starting`) both issue; it takes no D, so it is a `*_nolock` entry and reads
//! nothing but its input. Both are pure: they mutate nothing, and M1 commits at the
//! `ST` effect through the field adapter, `MC` and `CX-` write only their live
//! slot (C1.4.2). Neither reads M3 or M4.
//!
//! Shapes the appendix does not print, decided from the code (code wins):
//!
//! * **No-op rows.** R205.2 and R205.3 return [`Verdict::Continue`] with no effect
//!   and the reply `Ok(())`: the cancel command returns without touching anything
//!   (it holds T and D only to read them).
//! * **R205.1 stale.** The expected-owner mismatch is checked before the status is
//!   read, whatever the status is, and emits no terminal (**PD-12**, unlike stop's
//!   R201.1).
//! * **R205.4 reply.** `Ok(())`; the capture cancel's own result is site 206's.
//! * **Site 206 reply.** The success reply is the caller's: stop answers
//!   `{"type":"recording_cancelled","state":"recovering"}`, cancel `Ok(())`
//!   ([`CancelOrigin`]). The `Err` row is the same for both callers: no effect and the
//!   error text of the failed call (**PD-21**, fixed: it used to claim
//!   `PipelineFailure/RuntimeFailure`).
//! * **Op status.** Site 206 carries a token, and no row branches on its
//!   `op_status` (C1.4.1 rule 3).
//!
//! Rows that reproduce a preserved defect keep it: PD-12 (R205.1), PD-16 (R205.5:
//! the post-D clears and emits run with no recheck).

use crate::decision::{ContKind, ContinuationSpec, Decision, StaleKind, Verdict};
use crate::effects::{Effect, EventKind, PerfComplete, PerfOutcome, PerfStage, Placement as At};
use crate::ids::Now;
use crate::inputs::{CancelCaptureResultInput, CancelInput, CancelOrigin};
use crate::lifecycle::{
    clear_context, decision, emit_event, emit_status, nothing, placed, set_status, terminal,
};
use crate::replies::{ErrText, Reply, ReplyType};
use crate::state::{DictationCore, LockFreeFacts};
use crate::status::{AudioCancelReason, ErrCode, Status, TerminalOutcome};

fn with_reply(mut decided: Decision, reply: Reply) -> Decision {
    decided.reply = Some(reply);
    decided
}

/// Site 205, `decide_cancel_request` (R205.1-.5). Status and id come from the
/// core view the shell built under D; `input.expected` is the caller's owner.
///
/// * R205.1 stale: `expected = Some(e)` with `e != current_id`, any status
///   (**PD-12**: no terminal). No effect; reply `Ok(())`.
/// * R205.2 `Idle`, R205.3 `Recovering`: no effect; reply `Ok(())`.
/// * R205.4 `Starting` / `Recording`: `PT:CC(rid, User)` (after T is released;
///   blocking up to 2 s); `K:CaptureCancel`; no status write: `Recovering{User}`
///   arrives from the supervisor (R209). The capture cancel's result is site 206.
/// * R205.5 `Processing`: `In:ST→Idle`; `PDT:CX-`, `MC`, `TERM(UserCancelledProcessing /
///   CancelledProcessing)`, `KB(false)`, `EM(idle)`, `EV(Cancelled)`; `PT:PC(Cancelled,
///   InferenceDecode)`. `pipelines[rid]` is left in place: it is draining until
///   R204a (**PD-16**: the post-D clears and emits run with no recheck).
pub fn decide_cancel_request(core: &DictationCore, input: CancelInput, _now: Now) -> Decision {
    let rid = core.current_id;
    let reply_ok = |decided: Decision| with_reply(decided, Reply::Ok);
    if input.expected.is_some_and(|expected| expected != rid) {
        return reply_ok(nothing(Verdict::Stale(StaleKind::StaleRecording), "R205.1"));
    }
    match core.status {
        Status::Idle => reply_ok(nothing(Verdict::Continue, "R205.2")),
        Status::Recovering => reply_ok(nothing(Verdict::Continue, "R205.3")),
        Status::Starting | Status::Recording => {
            let mut decided = reply_ok(decision(
                Verdict::Applied,
                None,
                vec![placed(
                    At::PostT,
                    Effect::CaptureCancel {
                        id: rid,
                        reason: AudioCancelReason::User,
                    },
                )],
                "R205.4",
            ));
            decided.continuation = Some(ContinuationSpec {
                recording_id: rid,
                kind: ContKind::CaptureCancel,
            });
            decided
        }
        Status::Processing => reply_ok(decision(
            Verdict::Applied,
            Some((Status::Processing, Status::Idle)),
            vec![
                set_status(Status::Idle),
                clear_context(At::PostDT, rid),
                placed(At::PostDT, Effect::MarkCancelled(rid)),
                terminal(
                    At::PostDT,
                    TerminalOutcome::UserCancelledProcessing,
                    ErrCode::CancelledProcessing,
                ),
                placed(At::PostDT, Effect::KeyboardProcessing(false)),
                emit_status(At::PostDT, Status::Idle),
                emit_event(At::PostDT, EventKind::Cancelled),
                placed(
                    At::PostT,
                    Effect::PerfComplete(PerfComplete {
                        outcome: PerfOutcome::Cancelled,
                        stage: PerfStage::InferenceDecode,
                        cause: None,
                        with_audio_duration: false,
                    }),
                ),
            ],
            "R205.5",
        )),
    }
}

/// Site 206, `decide_cancel_capture_result_nolock` (R206.1-.2), D-less: the result
/// of the capture cancel the cancel command (R205.4) or the stop command
/// (R201.4) issued after releasing T. Nothing is read: the facts are unused, and
/// the status is never looked at.
///
/// * R206.1 `Ok(_)`: no effect ([`Verdict::Continue`]). Reply: stop
///   `recording_cancelled` / `recovering`, cancel `Ok(())`. The supervisor emits the
///   cancel terminal (R209).
/// * R206.2 `Err`: no effect, reply `Err(<the failed call's text>)` ([`ErrText::Shell`]).
///   A failed or timed-out capture cancel does not end the recording: the supervisor may
///   still hold the queued `Cancel{User}` (the client's 2 s receive expires first), or a
///   stop that took the capture goes on. So the failure claims no terminal and leaves it
///   to the path that really ends the recording (a stop's rows, or the supervisor's
///   `Recovering` arm, R209.3-.5), which also completes performance. It used to claim
///   `PipelineFailure / RuntimeFailure` for a recording that then succeeded or was
///   cancelled, so telemetry and the performance record disagreed (**PD-21**, fixed).
pub fn decide_cancel_capture_result_nolock(
    _facts: LockFreeFacts,
    input: CancelCaptureResultInput,
    _now: Now,
) -> Decision {
    match input.result {
        Ok(_) => with_reply(
            nothing(Verdict::Continue, "R206.1"),
            match input.origin {
                CancelOrigin::Stop => {
                    Reply::typed(ReplyType::RecordingCancelled, Status::Recovering)
                }
                CancelOrigin::Cancel => Reply::Ok,
            },
        ),
        // `Applied` with no effect: the shell reads the verdict to return the error.
        Err(()) => with_reply(
            decision(Verdict::Applied, None, Vec::new(), "R206.2"),
            Reply::Err(ErrText::Shell),
        ),
    }
}

#[cfg(test)]
mod tests;
