//! One test per C3.6 row (R207.1 - R216.5): guard, observations consumed,
//! mutation, ordered placed effects, continuation and reply, for every status
//! that reaches the row, plus the cross-cutting properties of the family.
//!
//! The expected effect lists are written as the appendix prints them
//! (`In:ST→Recording`, `PDT:EV(InitStalled)`), through [`tags`], not through the
//! builders `lifecycle.rs` uses, so a reordered or re-placed effect fails here.

use super::*;
use crate::decision::SiteId;
use crate::effects::TakeMode;
use crate::rows::rows_of_site;
use crate::state::LiveObs;

const NOW: Now = Now(0);
const RID: RecordingId = RecordingId(7);
const NEWER: RecordingId = RecordingId(8);

fn core(status: Status, current: RecordingId) -> DictationCore {
    DictationCore::project(status, current, LiveObs::default())
}

fn facts(current: RecordingId) -> LockFreeFacts {
    LockFreeFacts {
        current_id: current,
        cancelled_through: RecordingId::NONE,
    }
}

/// An effect as the appendix writes it. A conditional `PRES` also names the
/// index of the emit it pairs with.
fn tag(placed: &Placed) -> String {
    let at = placed.at.label();
    let body = match &placed.effect {
        Effect::SetStatus(to) => format!("ST→{to:?}"),
        Effect::EmitStatus { status, cond } => {
            assert_eq!(
                *cond,
                Cond::Always,
                "EM({status:?}) carries no predicate here"
            );
            format!("EM({})", status.as_str())
        }
        Effect::EmitEvent(EventKind::Interrupted { auto_transcribe }) => {
            format!("EV(Interrupted{{autoTranscribe:{auto_transcribe}}})")
        }
        Effect::EmitEvent(kind) => format!("EV({kind:?})"),
        Effect::Presentation { kind, cond } => match cond {
            Cond::EmitOk(paired) => format!("PRES({kind:?})<-#{paired}"),
            other => panic!("PRES must pair with an emit, got {other:?}"),
        },
        Effect::Terminal(t) => format!("TERM({:?}/{:?})", t.outcome, t.code),
        Effect::PerfComplete(pc) => format!(
            "PC({:?},{:?},{:?},duration={}{}{})",
            pc.outcome,
            pc.stage,
            pc.cause,
            pc.with_audio_duration,
            // PD-24: only the arm out of `Starting` completes only a run that exists.
            if pc.only_if_run { ",only-if-run" } else { "" },
            // PD-20: the cancel's completion is the second half of its terminal claim.
            if pc.follows_terminal {
                ",follows-terminal"
            } else {
                ""
            }
        ),
        Effect::ClearContext(id) => format!("CX-({})", id.0),
        Effect::KeyboardProcessing(on) => format!("KB({on})"),
        Effect::TouchLastTranscription => "TL".into(),
        Effect::AcceptTelemetry => "ACC".into(),
        Effect::StartSideTasks => "SD".into(),
        Effect::ProductionCaptureFailure(_) => "PCF".into(),
        Effect::TakeInterrupted { id, mode } => {
            format!(
                "TI({}, {})",
                id.0,
                if *mode == TakeMode::Dispose { "d" } else { "u" }
            )
        }
        Effect::SelfStop { id, trigger } => format!("RS({}, {trigger:?})", id.0),
        other => panic!("unexpected effect {other:?}"),
    };
    format!("{at}:{body}")
}

fn tags(decision: &Decision) -> Vec<String> {
    decision.effects.iter().map(tag).collect()
}

/// Assert one whole row: id, verdict, mutation, ordered placed effects, and that
/// no continuation or reply is produced.
#[track_caller]
fn row(
    decision: &Decision,
    id: &str,
    verdict: Verdict,
    transition: Option<(Status, Status)>,
    effects: &[&str],
) {
    assert_eq!(decision.row, RowId(match_static(id)), "row id");
    assert_eq!(decision.verdict, verdict, "{id} verdict");
    assert_eq!(decision.transition, transition, "{id} mutation");
    assert_eq!(tags(decision), effects, "{id} ordered placed effects");
    assert_eq!(decision.continuation, None, "{id} continuation");
    assert_eq!(decision.reply, None, "{id} reply");
    assert_eq!(decision.validate(), Ok(()), "{id} structure");
    assert!(crate::rows::site_of_row(id).is_some(), "{id} is a C3 row");
}

fn match_static(id: &str) -> &'static str {
    crate::rows::C3_ROWS
        .iter()
        .map(|(row, _)| *row)
        .find(|row| *row == id)
        .unwrap_or_else(|| panic!("{id} is not a C3 row"))
}

const STALE: Verdict = Verdict::Stale(StaleKind::StaleRecording);

// ------------------------------------------------------------------- 207

fn ready(context_present: Option<bool>) -> SupReadyInput {
    SupReadyInput {
        rid: RID,
        context_present,
        ready_received: NOW,
    }
}

#[test]
fn r207_1_ready_for_a_non_current_recording_is_superseded_under_d() {
    for status in Status::ALL {
        for context in [None, Some(false), Some(true)] {
            let d = decide_sup_ready(&core(status, NEWER), ready(context), NOW);
            row(
                &d,
                "R207.1",
                STALE,
                None,
                &["In:TERM(Superseded/StaleOwner)", "PDT:SD"],
            );
        }
    }
}

#[test]
fn r207_2_ready_while_idle_recovering_or_processing_is_superseded() {
    for status in [Status::Idle, Status::Recovering, Status::Processing] {
        let d = decide_sup_ready(&core(status, RID), ready(None), NOW);
        row(
            &d,
            "R207.2",
            Verdict::Applied,
            None,
            &["In:TERM(Superseded/StaleOwner)", "PDT:SD"],
        );
    }
}

#[test]
fn r207_3_a_repeat_ready_for_a_live_recording_claims_no_terminal_and_still_starts_side_tasks() {
    // PD-23: the live recording's terminal belongs to the path that ends it. The second
    // ticker attempt (PD-4's remaining half) stays.
    let d = decide_sup_ready(&core(Status::Recording, RID), ready(Some(true)), NOW);
    row(&d, "R207.3", Verdict::Applied, None, &["PDT:SD"]);
}

#[test]
fn r207_4_ready_before_the_context_only_attempts_side_tasks() {
    let d = decide_sup_ready(&core(Status::Starting, RID), ready(Some(false)), NOW);
    row(&d, "R207.4", Verdict::Continue, None, &["PDT:SD"]);
}

#[test]
fn r207_5_ready_with_context_publishes_recording() {
    let d = decide_sup_ready(&core(Status::Starting, RID), ready(Some(true)), NOW);
    row(
        &d,
        "R207.5",
        Verdict::Applied,
        Some((Status::Starting, Status::Recording)),
        &["In:ST→Recording", "In:TL", "In:EM(recording)", "PDT:SD"],
    );
}

#[test]
fn ready_reads_the_context_only_after_the_stale_check_passed() {
    // D-S1-2: the observation is requested before any mutation, and only for a
    // current recording that is `Starting`; every other case decides without it.
    for status in Status::ALL {
        for current in [RID, NEWER] {
            let d = decide_sup_ready(&core(status, current), ready(None), NOW);
            let asks = matches!(
                d.verdict,
                Verdict::NeedObservation(ObsKind::ContextPresence { id }) if id == RID
            );
            assert_eq!(asks, status == Status::Starting && current == RID);
            if asks {
                assert!(d.effects.is_empty() && d.transition.is_none());
            } else {
                assert!(!d.effects.is_empty(), "{status:?}/{current:?} decides");
            }
        }
    }
}

// ------------------------------------------------------------------- 208

#[test]
fn r208_1_still_connecting_while_starting_emits_the_stall_under_d() {
    let d = decide_sup_still_connecting(&core(Status::Starting, RID), RID, NOW);
    row(
        &d,
        "R208.1",
        Verdict::Applied,
        None,
        &["In:EV(InitStalled)"],
    );
}

#[test]
fn r208_2_still_connecting_is_silent_otherwise() {
    for status in [
        Status::Idle,
        Status::Recording,
        Status::Recovering,
        Status::Processing,
    ] {
        let d = decide_sup_still_connecting(&core(status, RID), RID, NOW);
        row(&d, "R208.2", Verdict::Continue, None, &[]);
    }
    for status in Status::ALL {
        let d = decide_sup_still_connecting(&core(status, NEWER), RID, NOW);
        row(&d, "R208.2", STALE, None, &[]);
    }
}

// ------------------------------------------------------------------- 209

fn recovering(reason: AudioCancelReason) -> SupRecoveringInput {
    SupRecoveringInput { rid: RID, reason }
}

#[test]
fn r209_1_recovering_for_a_non_current_recording_is_superseded() {
    for status in Status::ALL {
        let d = decide_sup_recovering(
            &core(status, NEWER),
            recovering(AudioCancelReason::User),
            NOW,
        );
        row(
            &d,
            "R209.1",
            STALE,
            None,
            &["In:TERM(Superseded/StaleOwner)"],
        );
    }
}

#[test]
fn r209_2_recovering_over_idle_recovering_or_processing_is_superseded() {
    // PD-5: a `HardDeadline` while `Processing` supersedes a current recording.
    for status in [Status::Idle, Status::Recovering, Status::Processing] {
        for reason in [
            AudioCancelReason::User,
            AudioCancelReason::HardDeadline,
            AudioCancelReason::RuntimeFailure,
        ] {
            let d = decide_sup_recovering(&core(status, RID), recovering(reason), NOW);
            row(
                &d,
                "R209.2",
                Verdict::Applied,
                None,
                &["In:TERM(Superseded/StaleOwner)"],
            );
        }
    }
}

#[test]
fn r209_3_user_cancel_while_starting() {
    let d = decide_sup_recovering(
        &core(Status::Starting, RID),
        recovering(AudioCancelReason::User),
        NOW,
    );
    row(
        &d,
        "R209.3",
        Verdict::Applied,
        Some((Status::Starting, Status::Recovering)),
        &[
            "In:ST→Recovering",
            "PDT:CX-(7)",
            "PDT:EM(recovering)",
            "PDT:PRES(CleanupInProgress)<-#2",
            "PDT:EV(RecoveryStarted)",
            "PDT:PC(Cancelled,CaptureFinalization,None,duration=false,only-if-run)",
            "PDT:TERM(UserCancelledStarting/CancelledStarting)",
            "PDT:EV(Cancelled)",
        ],
    );
}

#[test]
fn r209_4_user_cancel_while_recording() {
    let d = decide_sup_recovering(
        &core(Status::Recording, RID),
        recovering(AudioCancelReason::User),
        NOW,
    );
    row(
        &d,
        "R209.4",
        Verdict::Applied,
        Some((Status::Recording, Status::Recovering)),
        &[
            "In:ST→Recovering",
            "PDT:CX-(7)",
            "PDT:EM(recovering)",
            "PDT:PRES(CleanupInProgress)<-#2",
            "PDT:EV(RecoveryStarted)",
            "PDT:PC(Cancelled,CaptureFinalization,None,duration=false)",
            "PDT:TERM(UserCancelledRecording/CancelledRecording)",
            "PDT:EV(Cancelled)",
        ],
    );
}

#[test]
fn r209_5_device_or_sleep_recovery_is_a_runtime_interruption() {
    for (reason, code) in [
        (AudioCancelReason::DeviceChanged, "DeviceChanged"),
        (AudioCancelReason::SystemSleep, "SystemSleep"),
        (AudioCancelReason::SystemWake, "SystemWake"),
    ] {
        for status in [Status::Starting, Status::Recording] {
            let d = decide_sup_recovering(&core(status, RID), recovering(reason), NOW);
            let terminal = format!("PDT:TERM(RuntimeInterruption/{code})");
            // PD-24: out of `Starting` the completion needs a run; out of `Recording` the
            // run was begun, so the claim is the run's own.
            let completion = if status == Status::Starting {
                "PDT:PC(Cancelled,CaptureFinalization,None,duration=false,only-if-run)"
            } else {
                "PDT:PC(Cancelled,CaptureFinalization,None,duration=false)"
            };
            row(
                &d,
                "R209.5",
                Verdict::Applied,
                Some((status, Status::Recovering)),
                &[
                    "In:ST→Recovering",
                    "PDT:CX-(7)",
                    "PDT:EM(recovering)",
                    "PDT:PRES(CleanupInProgress)<-#2",
                    "PDT:EV(RecoveryStarted)",
                    completion,
                    &terminal,
                ],
            );
        }
    }
}

#[test]
fn r209_6_runtime_failure_keeps_the_context_and_claims_no_terminal() {
    // The terminal comes later from `Interrupted`, `Idle` or `InitializationFailed`.
    for status in [Status::Starting, Status::Recording] {
        let d = decide_sup_recovering(
            &core(status, RID),
            recovering(AudioCancelReason::RuntimeFailure),
            NOW,
        );
        row(
            &d,
            "R209.6",
            Verdict::Applied,
            Some((status, Status::Recovering)),
            &[
                "In:ST→Recovering",
                "PDT:EM(recovering)",
                "PDT:PRES(CleanupInProgress)<-#1",
                "PDT:EV(RecoveryStarted)",
            ],
        );
    }
}

#[test]
fn r209_7_hard_deadline_clears_the_context_but_claims_no_terminal_and_no_perf() {
    for status in [Status::Starting, Status::Recording] {
        let d = decide_sup_recovering(
            &core(status, RID),
            recovering(AudioCancelReason::HardDeadline),
            NOW,
        );
        row(
            &d,
            "R209.7",
            Verdict::Applied,
            Some((status, Status::Recovering)),
            &[
                "In:ST→Recovering",
                "PDT:CX-(7)",
                "PDT:EM(recovering)",
                "PDT:PRES(CleanupInProgress)<-#2",
                "PDT:EV(RecoveryStarted)",
            ],
        );
    }
}

// ------------------------------------------------------------------- 210

#[test]
fn r210_1_cleanup_for_a_non_current_recording_does_nothing() {
    for status in Status::ALL {
        let d = decide_sup_cleanup_finished(&core(status, NEWER), RID, NOW);
        row(&d, "R210.1", STALE, None, &[]);
    }
}

#[test]
fn r210_2_cleanup_while_idle_observes_the_self_loop_and_publishes() {
    let d = decide_sup_cleanup_finished(&core(Status::Idle, RID), RID, NOW);
    row(
        &d,
        "R210.2",
        Verdict::Applied,
        Some((Status::Idle, Status::Idle)),
        &["In:ST→Idle", "In:EV(CleanupFinished)", "In:EM(idle)"],
    );
}

#[test]
fn r210_3_cleanup_while_recovering_restores_idle() {
    let d = decide_sup_cleanup_finished(&core(Status::Recovering, RID), RID, NOW);
    row(
        &d,
        "R210.3",
        Verdict::Applied,
        Some((Status::Recovering, Status::Idle)),
        &["In:ST→Idle", "In:EV(CleanupFinished)", "In:EM(idle)"],
    );
}

#[test]
fn r210_4_cleanup_during_a_live_status_only_publishes_the_event() {
    // Sealed delivery during Processing: the capture finished, the pipeline owns the id.
    for status in [Status::Starting, Status::Recording, Status::Processing] {
        let d = decide_sup_cleanup_finished(&core(status, RID), RID, NOW);
        row(
            &d,
            "R210.4",
            Verdict::Applied,
            None,
            &["In:EV(CleanupFinished)"],
        );
    }
}

// ------------------------------------------------------------------- 211

#[test]
fn r211_1_idle_for_a_non_current_recording_is_superseded() {
    for status in Status::ALL {
        let d = decide_sup_idle(&core(status, NEWER), RID, NOW);
        row(
            &d,
            "R211.1",
            STALE,
            None,
            &["In:TERM(Superseded/StaleOwner)"],
        );
    }
}

#[test]
fn r211_2_idle_while_already_idle_is_superseded_without_a_keyboard_reset() {
    // PD-2 / PD-18 (b): no KB reset, the terminal only wins if none was claimed.
    let d = decide_sup_idle(&core(Status::Idle, RID), RID, NOW);
    row(
        &d,
        "R211.2",
        Verdict::Applied,
        None,
        &["In:TERM(Superseded/StaleOwner)"],
    );
}

const FORCED_IDLE: [&str; 6] = [
    "In:TERM(PipelineFailure/RuntimeFailure)",
    "In:PC(Failed,CaptureFinalization,Some(AudioCaptureFailed),duration=false)",
    "In:ST→Idle",
    "In:CX-(7)",
    "In:KB(false)",
    "In:EM(idle)",
];

#[test]
fn r211_3_to_6_idle_forces_idle_over_any_live_status() {
    // PD-7 (Processing: the supervisor forces Idle over a running pipeline) and
    // PD-18 (b) (Recovering) keep their literal behaviour.
    for (status, id) in [
        (Status::Processing, "R211.3"),
        (Status::Starting, "R211.4"),
        (Status::Recording, "R211.5"),
        (Status::Recovering, "R211.6"),
    ] {
        let d = decide_sup_idle(&core(status, RID), RID, NOW);
        row(
            &d,
            id,
            Verdict::Applied,
            Some((status, Status::Idle)),
            &FORCED_IDLE,
        );
    }
}

// ------------------------------------------------------------------- 212

#[test]
fn r212_1_accepted_claims_telemetry_and_supersedes_a_non_current_recording() {
    let current = decide_sup_accepted_nolock(facts(RID), RID, NOW);
    row(&current, "R212.1", Verdict::Applied, None, &["PDT:ACC"]);
    let stale = decide_sup_accepted_nolock(facts(NEWER), RID, NOW);
    row(
        &stale,
        "R212.1",
        Verdict::Applied,
        None,
        &["PDT:ACC", "PDT:TERM(Superseded/StaleOwner)"],
    );
}

// ------------------------------------------------------------------- 213

fn init_failed(recovery_reason: Option<AudioCancelReason>) -> SupInitFailedInput {
    SupInitFailedInput {
        rid: RID,
        kind: Handle(3),
        error_code: ErrCode::DeviceUnavailable,
        recovery_reason,
    }
}

#[test]
fn r213_1_init_failure_for_a_non_current_recording_records_the_failure_then_supersedes() {
    let d = decide_sup_init_failed_nolock(facts(NEWER), init_failed(None), NOW);
    row(
        &d,
        "R213.1",
        STALE,
        None,
        &["PDT:PCF", "PDT:TERM(Superseded/StaleOwner)"],
    );
}

#[test]
fn r213_2_init_failure_claims_the_typed_failure_and_never_writes_status() {
    let d = decide_sup_init_failed_nolock(facts(RID), init_failed(None), NOW);
    row(
        &d,
        "R213.2",
        Verdict::Applied,
        None,
        &[
            "PDT:PCF",
            "PDT:TERM(CaptureInitFailure/DeviceUnavailable)",
            "PDT:CX-(7)",
            "PDT:PC(Failed,CaptureFinalization,Some(AudioCaptureFailed),duration=false)",
            "PDT:EV(InitFailed)",
            "PDT:PRES(InitFailed)<-#4",
        ],
    );
    // A runtime-failure recovery claims `RuntimeInterruption`, same code.
    let recovery = decide_sup_init_failed_nolock(
        facts(RID),
        init_failed(Some(AudioCancelReason::RuntimeFailure)),
        NOW,
    );
    assert_eq!(
        tags(&recovery)[1],
        "PDT:TERM(RuntimeInterruption/DeviceUnavailable)"
    );
    // Any other recovery reason is still a capture init failure.
    let other = decide_sup_init_failed_nolock(
        facts(RID),
        init_failed(Some(AudioCancelReason::HardDeadline)),
        NOW,
    );
    assert_eq!(
        tags(&other)[1],
        "PDT:TERM(CaptureInitFailure/DeviceUnavailable)"
    );
}

// ------------------------------------------------------------------- 214

#[test]
fn r214_1_recovery_stall_for_the_current_recording_is_published() {
    let d = decide_sup_stalled_nolock(facts(RID), RID, NOW);
    row(
        &d,
        "R214.1",
        Verdict::Applied,
        None,
        &["PDT:EV(RecoveryStalled)", "PDT:PRES(RecoveryStalled)<-#0"],
    );
}

#[test]
fn r214_2_recovery_stall_for_a_non_current_recording_is_silent() {
    let d = decide_sup_stalled_nolock(facts(NEWER), RID, NOW);
    row(&d, "R214.2", STALE, None, &[]);
}

// ------------------------------------------------------------------- 215

fn interrupted(delivered_samples: usize) -> SupInterruptedInput {
    SupInterruptedInput {
        rid: RID,
        delivered_samples,
        duration_ms: 1_500,
        reason: AudioCancelReason::DeviceChanged,
    }
}

#[test]
fn r215_1_interruption_of_a_non_current_recording_disposes_and_supersedes() {
    for samples in [0, 7_999, 8_000, 100_000] {
        let d = decide_sup_interrupted_nolock(facts(NEWER), interrupted(samples), NOW);
        row(
            &d,
            "R215.1",
            STALE,
            None,
            &[
                "PDT:TI(7, d)",
                "PDT:TERM(Superseded/StaleOwner)",
                "PDT:PC(Failed,CaptureFinalization,Some(AudioCaptureFailed),duration=true)",
            ],
        );
    }
}

#[test]
fn r215_2_a_long_interrupted_capture_auto_transcribes_through_a_self_stop() {
    for samples in [8_000, 8_001, 1_000_000] {
        let d = decide_sup_interrupted_nolock(facts(RID), interrupted(samples), NOW);
        assert_eq!(d.row, RowId("R215.2"));
        assert_eq!(d.verdict, Verdict::Applied);
        assert_eq!(d.transition, None);
        assert_eq!(
            tags(&d),
            [
                "PDT:EV(Interrupted{autoTranscribe:true})",
                "PDT:PRES(Interrupted)<-#0",
                "PDT:RS(7, Other)",
            ]
        );
        // K:AutoStop(rid, op_auto): the shell registers it, the spawned stop's
        // R201 consumes it. No reply.
        assert_eq!(
            d.continuation,
            Some(ContinuationSpec {
                recording_id: RID,
                kind: ContKind::AutoStop
            })
        );
        assert_eq!(d.reply, None);
        assert_eq!(d.validate(), Ok(()));
    }
}

#[test]
fn r215_3_a_short_interrupted_capture_hands_over_to_site_216() {
    for samples in [0, 1, 7_999] {
        let d = decide_sup_interrupted_nolock(facts(RID), interrupted(samples), NOW);
        row(
            &d,
            "R215.3",
            Verdict::Applied,
            None,
            &[
                "PDT:EV(Interrupted{autoTranscribe:false})",
                "PDT:PRES(Interrupted)<-#0",
            ],
        );
        assert_eq!(d.row, R215_3);
    }
}

// ------------------------------------------------------------------- 216

fn short_idle_effects() -> Vec<&'static str> {
    vec![
        "Pre:TI(7, d)",
        "Pre:TERM(RuntimeInterruption/RuntimeFailure)",
        "Pre:CX-(7)",
        "Pre:PC(Failed,CaptureFinalization,Some(AudioCaptureFailed),duration=true)",
        "In:ST→Idle",
        "PDT:KB(false)",
        "PDT:EM(idle)",
    ]
}

#[test]
fn r216_1_to_4_short_idle_has_no_status_guard_and_writes_idle_over_any_status() {
    // PD-1 (no guard), PD-15 (TI is destructive), PD-7 (Processing), PD-18 (a)
    // (Recovering): the literal behaviour is kept.
    for (status, id) in [
        (Status::Recovering, "R216.1"),
        (Status::Idle, "R216.2"),
        (Status::Starting, "R216.3"),
        (Status::Recording, "R216.3"),
        (Status::Processing, "R216.4"),
    ] {
        let d = decide_sup_interrupted_short_idle(&core(status, RID), RID, NOW);
        row(
            &d,
            id,
            Verdict::Applied,
            Some((status, Status::Idle)),
            &short_idle_effects(),
        );
    }
}

#[test]
fn r216_5_short_idle_writes_idle_over_a_newer_recording_too() {
    // X-prod (D3): no production schedule reaches it, but the core is total and
    // PD-1 covers the write itself.
    for status in Status::ALL {
        let d = decide_sup_interrupted_short_idle(&core(status, NEWER), RID, NOW);
        row(
            &d,
            "R216.5",
            Verdict::Applied,
            Some((status, Status::Idle)),
            &short_idle_effects(),
        );
    }
}

#[test]
fn short_idle_pre_effects_are_the_decisions_leading_pre_effects() {
    // The shell runs `pre_effects()` before D, then the decide's remainder.
    let pre = short_idle_pre_effects(RID);
    assert!(pre.iter().all(|p| p.at == Placement::Pre));
    let d = decide_sup_interrupted_short_idle(&core(Status::Recovering, RID), RID, NOW);
    assert_eq!(&d.effects[..pre.len()], &pre[..]);
    let rest = d.clone().after_pre();
    assert_eq!(
        rest.effects.iter().map(tag).collect::<Vec<_>>(),
        ["In:ST→Idle", "PDT:KB(false)", "PDT:EM(idle)"]
    );
    assert_eq!(rest.transition, d.transition);
    assert_eq!(rest.validate(), Ok(()));
    let only = Decision::pre_only(pre, SHORT_IDLE_PRE_ROW);
    assert_eq!(only.validate(), Ok(()));
    assert!(only.effects.iter().all(|p| p.at == Placement::Pre));
}

// ------------------------------------------------------- cross-cutting

/// Every decision of the family over the whole status x id x reason grid.
fn grid(view: impl Fn(Status, RecordingId) -> DictationCore) -> Vec<Decision> {
    let mut out = Vec::new();
    let reasons = [
        AudioCancelReason::User,
        AudioCancelReason::DeviceChanged,
        AudioCancelReason::SystemSleep,
        AudioCancelReason::SystemWake,
        AudioCancelReason::RuntimeFailure,
        AudioCancelReason::HardDeadline,
    ];
    for status in Status::ALL {
        for current in [RID, NEWER] {
            let core = view(status, current);
            let lock_free = core.lock_free_facts();
            for context in [None, Some(false), Some(true)] {
                out.push(decide_sup_ready(&core, ready(context), NOW));
            }
            out.push(decide_sup_still_connecting(&core, RID, NOW));
            for reason in reasons {
                out.push(decide_sup_recovering(&core, recovering(reason), NOW));
                out.push(decide_sup_init_failed_nolock(
                    lock_free,
                    init_failed(Some(reason)),
                    NOW,
                ));
            }
            out.push(decide_sup_cleanup_finished(&core, RID, NOW));
            out.push(decide_sup_idle(&core, RID, NOW));
            out.push(decide_sup_accepted_nolock(lock_free, RID, NOW));
            out.push(decide_sup_init_failed_nolock(
                lock_free,
                init_failed(None),
                NOW,
            ));
            out.push(decide_sup_stalled_nolock(lock_free, RID, NOW));
            for samples in [0, 7_999, 8_000] {
                out.push(decide_sup_interrupted_nolock(
                    lock_free,
                    interrupted(samples),
                    NOW,
                ));
            }
            out.push(decide_sup_interrupted_short_idle(&core, RID, NOW));
        }
    }
    out
}

#[test]
fn every_c3_6_row_is_produced_and_every_decision_is_well_formed() {
    let decisions = grid(core);
    let mut produced: Vec<&str> = decisions.iter().map(|d| d.row.0).collect();
    produced.sort_unstable();
    produced.dedup();
    let mut expected: Vec<&str> = [
        SiteId::SupReady,
        SiteId::SupStillConnecting,
        SiteId::SupRecovering,
        SiteId::SupCleanupFinished,
        SiteId::SupIdle,
        SiteId::SupAccepted,
        SiteId::SupInitFailed,
        SiteId::SupStalled,
        SiteId::SupInterrupted,
        SiteId::SupInterruptedShortIdle,
    ]
    .into_iter()
    .flat_map(rows_of_site)
    .collect();
    expected.sort_unstable();
    assert_eq!(
        produced, expected,
        "the family covers exactly its 37 C3.6 rows"
    );
    assert_eq!(expected.len(), 37);
    for d in &decisions {
        assert_eq!(d.validate(), Ok(()), "{}", d.row.0);
        assert_eq!(d.reply, None, "{}: no lifecycle row replies", d.row.0);
        let site = crate::rows::site_of_row(d.row.0).expect("a C3 row");
        assert!(rows_of_site(site).any(|r| r == d.row.0));
    }
}

#[test]
fn no_lifecycle_decision_reads_m3_or_m4() {
    // M3 (`cancelled_through`) and M4 (`context_for`) are live-only observations
    // and the family reads neither: `Ready` asks for the context through
    // `NeedObservation`. Varying them changes no decision.
    let bare = grid(core);
    let observed = grid(|status, current| {
        DictationCore::project(
            status,
            current,
            LiveObs {
                cancelled_through: RecordingId(1_000),
                context_for: Some(RecordingId(7)),
            },
        )
    });
    assert_eq!(bare, observed);
}

/// Sites 212 and 213 run their mutex-taking leading effect **before** the shell
/// reads the current id (the base order). The decision still returns it first,
/// and stripping the prefix leaves exactly what the facts decide.
#[test]
fn the_prefix_is_the_leading_effect_and_stripping_it_leaves_the_rest() {
    let kind = Handle(0);
    let accepted_prefix = accepted_prefix();
    let init_prefix = init_failed_prefix(kind);
    assert_eq!(
        accepted_prefix.iter().map(tag).collect::<Vec<_>>(),
        ["PDT:ACC"]
    );
    assert_eq!(init_prefix.iter().map(tag).collect::<Vec<_>>(), ["PDT:PCF"]);
    let input = |rid| SupInitFailedInput {
        rid,
        kind,
        error_code: ErrCode::StreamInvalidated,
        recovery_reason: None,
    };
    for current in [RID, NEWER] {
        let accepted = decide_sup_accepted_nolock(facts(current), RID, NOW);
        assert!(accepted.effects.starts_with(&accepted_prefix));
        let rest = accepted.after_prefix(&accepted_prefix);
        assert_eq!(
            tags(&rest),
            if current == RID {
                vec![]
            } else {
                vec!["PDT:TERM(Superseded/StaleOwner)".to_string()]
            }
        );
        let failed = decide_sup_init_failed_nolock(facts(current), input(RID), NOW);
        assert!(failed.effects.starts_with(&init_prefix));
        let rest = failed.after_prefix(&init_prefix);
        assert!(
            !rest
                .effects
                .iter()
                .any(|p| matches!(p.effect, Effect::ProductionCaptureFailure(_))),
            "PCF runs once"
        );
        assert!(!rest.effects.is_empty());
        assert_eq!(rest.validate(), Ok(()), "the stripped list is still valid");
    }
    // The paired presentation follows its emit by index after the strip.
    let current =
        decide_sup_init_failed_nolock(facts(RID), input(RID), NOW).after_prefix(&init_prefix);
    assert_eq!(
        tags(&current),
        [
            "PDT:TERM(CaptureInitFailure/StreamInvalidated)",
            "PDT:CX-(7)",
            "PDT:PC(Failed,CaptureFinalization,Some(AudioCaptureFailed),duration=false)",
            "PDT:EV(InitFailed)",
            "PDT:PRES(InitFailed)<-#3",
        ]
    );
}

/// The prefix runs before the facts are read, so the decision it is stripped
/// from is the one the **later** current id gives: a generation that advanced
/// during `ACC` / `PCF` turns an applied row into the stale row.
#[test]
fn a_generation_advanced_during_the_prefix_gives_the_stale_row() {
    let kind = Handle(0);
    let input = SupInitFailedInput {
        rid: RID,
        kind,
        error_code: ErrCode::StreamInvalidated,
        recovery_reason: None,
    };
    // Snapshot before the prefix would have said R213.2; after it, R213.1.
    assert_eq!(
        decide_sup_init_failed_nolock(facts(RID), input, NOW).row,
        RowId("R213.2")
    );
    let late = decide_sup_init_failed_nolock(facts(NEWER), input, NOW)
        .after_prefix(&init_failed_prefix(kind));
    assert_eq!(late.row, RowId("R213.1"));
    assert_eq!(tags(&late), ["PDT:TERM(Superseded/StaleOwner)"]);
}

#[test]
fn only_the_continuing_row_returns_a_continuation() {
    for d in grid(core) {
        assert_eq!(
            d.continuation.is_some(),
            d.row == RowId("R215.2"),
            "{}",
            d.row.0
        );
    }
}

#[test]
fn st_is_in_and_in_effects_only_exist_at_d_sites() {
    for d in grid(core) {
        let site = crate::rows::site_of_row(d.row.0).expect("row");
        let d_less = matches!(
            site,
            SiteId::SupAccepted
                | SiteId::SupInitFailed
                | SiteId::SupStalled
                | SiteId::SupInterrupted
        );
        for placed in &d.effects {
            if d_less {
                assert_eq!(
                    placed.at,
                    Placement::PostDT,
                    "{}: a D-less entry puts every effect in PDT",
                    d.row.0
                );
            }
            if placed.effect.is_set_status() {
                assert_eq!(placed.at, Placement::In, "{}", d.row.0);
            }
        }
        // Only site 216 has `Pre` effects.
        let has_pre = d.effects.iter().any(|p| p.at == Placement::Pre);
        assert_eq!(
            has_pre,
            site == SiteId::SupInterruptedShortIdle,
            "{}",
            d.row.0
        );
    }
}

#[test]
fn the_defect_rows_keep_their_literal_behaviour() {
    // PD-1: no guard on the 216 write; PD-7: Idle over a running pipeline;
    // PD-5: HardDeadline supersedes a Processing recording.
    let pd1 = decide_sup_interrupted_short_idle(&core(Status::Processing, RID), RID, NOW);
    assert_eq!(pd1.transition, Some((Status::Processing, Status::Idle)));
    let pd7 = decide_sup_idle(&core(Status::Processing, RID), RID, NOW);
    assert_eq!(pd7.transition, Some((Status::Processing, Status::Idle)));
    let pd5 = decide_sup_recovering(
        &core(Status::Processing, RID),
        recovering(AudioCancelReason::HardDeadline),
        NOW,
    );
    assert_eq!(tags(&pd5), ["In:TERM(Superseded/StaleOwner)"]);
}
