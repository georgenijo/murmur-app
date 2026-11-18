//! `DictationCore::handle` routes every input to the decide function that owns it
//! and adds nothing of its own.

use super::*;
use crate::admission::Occupancy;
use crate::decision::{ContKind, Continuation, OpStatus, SiteId, Verdict};
use crate::ids::{Handle, OperationId, RecordingId};
use crate::inputs::{
    CancelCaptureResultInput, CancelInput, CancelOrigin, CaptureErr, CaptureResult,
    CaptureResultInput, DestinationInput, DeviceSel, GestureObs, GuardFinishInput, GuardOrigin,
    InterruptedMeta, Origin, PipelineIdleEmitInput, PipelineResult, PipelineTerminal,
    ProcessAudioFinishInput, ProcessAudioFinishKind, ProcessAudioStartInput, ReadyPublishInput,
    SpeculativeObs, StartContextInput, StartFlightEndInput, StartInput, StopCaptureResult,
    StopCaptureResultInput, StopInput,
};
use crate::lifecycle::{decide_sup_accepted_nolock, decide_sup_ready, SupReadyInput};
use crate::rows::site_of_row;
use crate::state::{LiveObs, PersistedCore};
use crate::status::{AudioCancelReason, ErrCode, Status, StopTrigger};

const NOW: Now = Now(0);
const RID: RecordingId = RecordingId(7);

fn view(status: Status, current: RecordingId) -> DictationCore {
    DictationCore::new(
        PersistedCore::new(status, current),
        LiveObs {
            cancelled_through: RecordingId(3),
            context_for: Some(RID),
        },
    )
}

fn token(kind: ContKind) -> Continuation {
    Continuation {
        recording_id: RID,
        op: OperationId(11),
        kind,
    }
}

fn start(device: DeviceSel, app_disabled: bool) -> StartInput {
    StartInput {
        origin: Origin::Hold,
        device,
        gesture: GestureObs::default(),
        speculative: SpeculativeObs::default(),
        kept_preview: false,
        app_disabled,
        fixed_and_smart: false,
        occupancy: Occupancy::default(),
        cleanup_pending: Some(false),
        smart_auto_result: None,
    }
}

/// At least one input per decide function, covering both entries of the sites
/// that have two (a D entry and a D-less one).
fn inputs() -> Vec<Input> {
    let rid = RID;
    let sup = |event| Input::Supervisor(event);
    let stop_ctx = |context_present| {
        Input::StopContext(crate::inputs::StopContextInput {
            token: token(ContKind::StopContext),
            op_status: OpStatus::Outstanding,
            rid,
            context_present,
            interrupted_error: None,
        })
    };
    let capture_result = |result| {
        Input::StopCaptureResult(StopCaptureResultInput {
            token: token(ContKind::CaptureStop),
            op_status: OpStatus::Outstanding,
            rid,
            result,
        })
    };
    let idle_emit = |kind, result| {
        Input::PipelineIdleEmit(PipelineIdleEmitInput {
            rid,
            kind,
            result,
            interrupted: None,
            char_count: 4,
        })
    };
    vec![
        Input::StartRequest(start(DeviceSel::SystemDefault, false)),
        Input::StartRequest(start(DeviceSel::SystemDefault, true)),
        Input::StartCaptureResult(CaptureResultInput {
            token: token(ContKind::CaptureStart),
            op_status: OpStatus::Outstanding,
            result: CaptureResult::Ok,
        }),
        Input::StartCaptureResult(CaptureResultInput {
            token: token(ContKind::CaptureStart),
            op_status: OpStatus::Outstanding,
            result: CaptureResult::Err(CaptureErr::Other),
        }),
        Input::StartContext(StartContextInput {
            token: token(ContKind::ResolveContext),
            op_status: OpStatus::Outstanding,
        }),
        Input::ReadyPublish(ReadyPublishInput {
            token: token(ContKind::ReadyPublish),
            op_status: OpStatus::Outstanding,
            rid,
            audio_ready: true,
            context_present: Some(true),
            context_ready_at: Now(5),
        }),
        Input::ReadyPublish(ReadyPublishInput {
            token: token(ContKind::ReadyPublish),
            op_status: OpStatus::Outstanding,
            rid,
            audio_ready: false,
            context_present: None,
            context_ready_at: Now(5),
        }),
        Input::StopRequest(StopInput {
            expected: Some(rid),
            trigger: StopTrigger::User,
            observed_current_id: rid,
            interrupted: Some(Some(InterruptedMeta {
                samples: 100,
                reason_code: ErrCode::RuntimeFailure,
                duration_ms: 5,
            })),
            entered: Now(1),
        }),
        Input::StopRequest(StopInput {
            expected: Some(RecordingId(2)),
            trigger: StopTrigger::User,
            observed_current_id: rid,
            interrupted: Some(None),
            entered: Now(1),
        }),
        stop_ctx(false),
        stop_ctx(true),
        capture_result(StopCaptureResult::Err),
        capture_result(StopCaptureResult::Samples {
            n: 100,
            interrupted: None,
        }),
        capture_result(StopCaptureResult::Samples {
            n: 50_000,
            interrupted: None,
        }),
        Input::GuardFinish(GuardFinishInput {
            rid,
            origin: GuardOrigin::PipelineExit,
            closure_emit: true,
        }),
        idle_emit(crate::inputs::PostKind::StopPost, None),
        idle_emit(
            crate::inputs::PostKind::StopPost,
            Some(PipelineResult::Ok(PipelineTerminal::Success)),
        ),
        idle_emit(crate::inputs::PostKind::LegacyPost, None),
        Input::CancelRequest(CancelInput { expected: None }),
        Input::CancelCaptureResult(CancelCaptureResultInput {
            token: token(ContKind::CaptureCancel),
            op_status: OpStatus::Outstanding,
            rid,
            result: Ok(true),
            origin: CancelOrigin::Cancel,
        }),
        sup(SupervisorEvent::Accepted { rid }),
        sup(SupervisorEvent::Ready {
            rid,
            context_present: true,
            ready_received: Now(2),
        }),
        sup(SupervisorEvent::StillConnecting { rid }),
        sup(SupervisorEvent::Recovering {
            rid,
            reason: AudioCancelReason::User,
        }),
        sup(SupervisorEvent::CleanupFinished { rid }),
        sup(SupervisorEvent::Idle { rid }),
        sup(SupervisorEvent::InitializationFailed {
            rid,
            kind: Handle(1),
            error_code: ErrCode::PermissionDenied,
            recovery_reason: None,
        }),
        sup(SupervisorEvent::RecoveryStalled { rid }),
        sup(SupervisorEvent::Interrupted {
            rid,
            delivered_samples: 100,
            duration_ms: 10,
            reason: AudioCancelReason::RuntimeFailure,
        }),
        sup(SupervisorEvent::Interrupted {
            rid,
            delivered_samples: 9_000,
            duration_ms: 10,
            reason: AudioCancelReason::RuntimeFailure,
        }),
        sup(SupervisorEvent::InterruptedShortIdle { rid }),
        Input::ProcessAudioStart(ProcessAudioStartInput {
            occupancy: Occupancy::default(),
            benchmark_running: Some(false),
        }),
        Input::ProcessAudioFinish(ProcessAudioFinishInput {
            rid,
            kind: ProcessAudioFinishKind::ParseFailed,
            closure_emit: true,
        }),
        Input::ProcessAudioFinish(ProcessAudioFinishInput {
            rid,
            kind: ProcessAudioFinishKind::PipelineDone,
            closure_emit: true,
        }),
        Input::StartDestination(DestinationInput {
            rid,
            smart_enabled: true,
        }),
        Input::StartFlightEnd(StartFlightEndInput { rid }),
        Input::ProcessAudioDestination(DestinationInput {
            rid,
            smart_enabled: true,
        }),
    ]
}

fn grid() -> Vec<DictationCore> {
    let mut cores = Vec::new();
    for status in Status::ALL {
        for current in [RID, RecordingId(2), RecordingId(8), RecordingId::NONE] {
            cores.push(view(status, current));
        }
    }
    cores
}

#[test]
fn every_row_a_handle_returns_belongs_to_the_site_of_the_input() {
    for input in inputs() {
        for core in grid() {
            let decided = core.handle(input.clone(), NOW);
            assert_eq!(
                site_of_row(decided.row.0),
                Some(input.site()),
                "{input:?} on {core:?} returned {}",
                decided.row.0
            );
        }
    }
}

#[test]
fn every_decision_a_handle_returns_is_structurally_valid() {
    for input in inputs() {
        for core in grid() {
            let decided = core.handle(input.clone(), NOW);
            assert_eq!(
                decided.validate(),
                Ok(()),
                "{input:?} on {core:?} returned {decided:?}"
            );
        }
    }
}

#[test]
fn handle_is_pure_and_total() {
    for input in inputs() {
        for core in grid() {
            let before = core;
            let first = core.handle(input.clone(), NOW);
            let second = core.handle(input.clone(), Now(999_999));
            assert_eq!(first, second, "`now` must not change a decision: {input:?}");
            assert_eq!(core, before, "a decision never mutates its view");
        }
    }
}

#[test]
fn handle_covers_every_site_the_input_vocabulary_names() {
    let mut reached: Vec<u16> = inputs().iter().map(|i| i.site().code()).collect();
    reached.sort_unstable();
    reached.dedup();
    let mut every: Vec<u16> = SiteId::ALL.iter().map(|s| s.code()).collect();
    every.sort_unstable();
    assert_eq!(reached, every, "the routing table reaches every C0.3 site");
}

#[test]
fn nolock_entries_get_the_lock_free_facts_of_the_view() {
    let core = view(Status::Recording, RID);
    let direct = decide_sup_accepted_nolock(core.lock_free_facts(), RecordingId(2), NOW);
    let routed = core.handle(
        Input::Supervisor(SupervisorEvent::Accepted {
            rid: RecordingId(2),
        }),
        NOW,
    );
    assert_eq!(routed, direct);
    assert_eq!(routed.row.0, "R212.1");
    assert!(
        routed.effects.len() == 2,
        "a stale Accepted adds the Superseded terminal: {routed:?}"
    );
}

#[test]
fn an_answered_observation_is_the_decision_and_an_unanswered_one_is_returned() {
    let core = view(Status::Starting, RID);
    let ready = |context_present| SupReadyInput {
        rid: RID,
        context_present,
        ready_received: Now(2),
    };
    // The supervisor `Ready` input carries the answer (`context_present: bool`).
    let routed = core.handle(
        Input::Supervisor(SupervisorEvent::Ready {
            rid: RID,
            context_present: true,
            ready_received: Now(2),
        }),
        NOW,
    );
    assert_eq!(routed, decide_sup_ready(&core, ready(Some(true)), NOW));
    assert_eq!(routed.row.0, "R207.5");
    // An input whose answer is still absent comes back as `NeedObservation`,
    // untouched, for the caller to answer.
    let waiting = core.handle(
        Input::ReadyPublish(ReadyPublishInput {
            token: token(ContKind::ReadyPublish),
            op_status: OpStatus::Outstanding,
            rid: RID,
            audio_ready: true,
            context_present: None,
            context_ready_at: Now(5),
        }),
        NOW,
    );
    assert!(
        matches!(waiting.verdict, Verdict::NeedObservation(_)),
        "{waiting:?}"
    );
    assert!(waiting.effects.is_empty());
}

#[test]
fn the_two_entries_of_a_split_site_follow_the_input() {
    let core = view(Status::Processing, RID);
    let ctx = |context_present| {
        Input::StopContext(crate::inputs::StopContextInput {
            token: token(ContKind::StopContext),
            op_status: OpStatus::Outstanding,
            rid: RID,
            context_present,
            interrupted_error: None,
        })
    };
    assert_eq!(core.handle(ctx(true), NOW).row.0, "R202.1");
    assert_eq!(core.handle(ctx(false), NOW).row.0, "R202.3");
    let tail = Input::PipelineIdleEmit(PipelineIdleEmitInput {
        rid: RID,
        kind: crate::inputs::PostKind::StopPost,
        result: Some(PipelineResult::Err),
        interrupted: None,
        char_count: 0,
    });
    assert_eq!(core.handle(tail, NOW).row.0, "R204b.5");
    let in_d = Input::PipelineIdleEmit(PipelineIdleEmitInput {
        rid: RID,
        kind: crate::inputs::PostKind::StopPost,
        result: None,
        interrupted: None,
        char_count: 0,
    });
    assert_eq!(core.handle(in_d, NOW).row.0, "R204b.3");
}

#[test]
fn project_and_new_build_the_same_view() {
    let obs = LiveObs {
        cancelled_through: RecordingId(1),
        context_for: None,
    };
    for status in Status::ALL {
        assert_eq!(
            DictationCore::project(status, RID, obs),
            DictationCore::new(PersistedCore::new(status, RID), obs)
        );
    }
}
