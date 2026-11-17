//! One test per C3.7 row (R301.1-R301.5, R302.1, R302.2), for every status that
//! reaches the row: guard, observation, mutation, ordered placed effects,
//! continuation and reply. Expected effect lists are written as the appendix prints
//! them (`In:ID+`, `PT:KB(true)`), not through the builders `process_audio.rs` uses.

use super::*;
use crate::admission::Occupancy;
use crate::decision::{RowId, SiteId};
use crate::effects::{Cond, Effect, Placed};
use crate::ids::RecordingId;
use crate::rows::{rows_of_site, C3_ROWS};
use crate::state::LiveObs;

const NOW: Now = Now(0);
const RID: RecordingId = RecordingId(7);
const NEWER: RecordingId = RecordingId(9);

fn core(status: Status, current: RecordingId) -> DictationCore {
    DictationCore::project(status, current, LiveObs::default())
}

fn tag(placed: &Placed) -> String {
    let at = placed.at.label();
    let body = match &placed.effect {
        Effect::AllocId => "ID+".to_string(),
        Effect::SetStatus(to) => format!("ST→{to:?}"),
        Effect::AxSnapshot(mode) => format!("AX({mode:?})"),
        Effect::KeyboardProcessing(on) => format!("KB({on})"),
        Effect::EmitEvent(kind) => format!("EV({kind:?})"),
        Effect::EmitStatus { status, cond } => {
            assert_eq!(*cond, Cond::Always);
            format!("EM({})", status.as_str())
        }
        Effect::ClearContext(id) => format!("CX-({})", id.0),
        other => panic!("not a process_audio effect: {other:?}"),
    };
    format!("{at}:{body}")
}

fn tags(decided: &Decision) -> Vec<String> {
    decided.effects.iter().map(tag).collect()
}

fn input(
    meeting: bool,
    inference: bool,
    file: bool,
    benchmark: Option<bool>,
) -> ProcessAudioStartInput {
    ProcessAudioStartInput {
        occupancy: Occupancy {
            meeting_active: meeting,
            meeting_inference_active: inference,
            file_transcribing: file,
            ..Occupancy::default()
        },
        benchmark_running: benchmark,
    }
}

fn start(status: Status, input: ProcessAudioStartInput) -> Decision {
    decide_process_audio_start(&core(status, RID), input, NOW)
}

fn assert_pure_refusal(decided: &Decision, row: &'static str, kind: ProcessAudioRefusal) {
    assert_eq!(decided.row, RowId(row));
    assert_eq!(
        decided.verdict,
        Verdict::Refused(Refusal::ProcessAudio(kind))
    );
    assert!(decided.effects.is_empty(), "{row}: a refusal runs nothing");
    assert_eq!(decided.transition, None);
    assert_eq!(decided.continuation, None);
    assert_eq!(decided.reply, Some(Reply::Err(ErrText::ProcessAudio(kind))));
    assert_eq!(decided.validate(), Ok(()));
}

// ------------------------------------------------------------------ 301

#[test]
fn r301_1_a_meeting_refuses_in_every_status_before_anything_else() {
    for status in Status::ALL {
        // Either meeting flag blocks (`meeting_blocks_asr`), and the file and
        // benchmark facts are not consulted: even a benchmark that was never
        // read does not turn the answer into a `NeedObservation`.
        for (meeting, inference) in [(true, false), (false, true), (true, true)] {
            let decided = start(status, input(meeting, inference, true, Some(true)));
            assert_pure_refusal(&decided, "R301.1", ProcessAudioRefusal::MeetingActive);
            let unread = start(status, input(meeting, inference, false, None));
            assert_pure_refusal(&unread, "R301.1", ProcessAudioRefusal::MeetingActive);
        }
    }
    assert_eq!(
        ProcessAudioRefusal::MeetingActive.message(),
        "Cannot process audio while a meeting transcript is active."
    );
}

#[test]
fn r301_1_is_decided_with_no_d_from_the_input_alone() {
    let blocked = decide_process_audio_start_pre(&input(true, false, false, None), NOW)
        .expect("a meeting blocks");
    assert_eq!(blocked.row, RowId("R301.1"));
    assert!(decide_process_audio_start_pre(&input(false, false, true, Some(true)), NOW).is_none());
}

#[test]
fn r301_2_a_file_transcription_refuses_in_every_status_without_reading_the_benchmark() {
    for status in Status::ALL {
        // `benchmark_running` is still `None`: the coordinator mutex is not
        // read once the file check refused (D-S1-2).
        let decided = start(status, input(false, false, true, None));
        assert_pure_refusal(&decided, "R301.2", ProcessAudioRefusal::FileTranscription);
        let with_benchmark = start(status, input(false, false, true, Some(true)));
        assert_pure_refusal(
            &with_benchmark,
            "R301.2",
            ProcessAudioRefusal::FileTranscription,
        );
    }
}

#[test]
fn the_benchmark_is_read_only_through_need_observation_after_the_file_check() {
    for status in Status::ALL {
        let decided = start(status, input(false, false, false, None));
        assert_eq!(
            decided.verdict,
            Verdict::NeedObservation(ObsKind::BenchmarkRunning),
            "{status:?}"
        );
        // Asked before any mutation: no effect, no continuation, no reply.
        assert!(decided.effects.is_empty());
        assert_eq!(decided.transition, None);
        assert_eq!(decided.continuation, None);
        assert_eq!(decided.reply, None);
        // Even a status that will refuse asks first: today's code reads the
        // benchmark before the status.
    }
}

#[test]
fn r301_3_a_running_benchmark_refuses_in_every_status() {
    for status in Status::ALL {
        let decided = start(status, input(false, false, false, Some(true)));
        assert_pure_refusal(&decided, "R301.3", ProcessAudioRefusal::Benchmark);
    }
}

#[test]
fn r301_4_live_dictation_refuses_in_every_non_idle_status() {
    for status in [
        Status::Starting,
        Status::Recording,
        Status::Recovering,
        Status::Processing,
    ] {
        let decided = start(status, input(false, false, false, Some(false)));
        assert_pure_refusal(&decided, "R301.4", ProcessAudioRefusal::LiveDictation);
    }
}

#[test]
fn r301_5_an_idle_import_is_accepted() {
    for current in [RecordingId(0), RID] {
        let decided = decide_process_audio_start(
            &core(Status::Idle, current),
            input(false, false, false, Some(false)),
            NOW,
        );
        let rid = current.next();
        assert_eq!(decided.row, RowId("R301.5"));
        assert_eq!(decided.verdict, Verdict::Applied);
        assert_eq!(decided.transition, Some((Status::Idle, Status::Processing)));
        assert_eq!(
            tags(&decided),
            [
                "In:ID+",
                "In:ST→Processing",
                "In:AX(Stop)",
                "PT:KB(true)",
                "PT:EV(GenStarted)",
                "PT:EM(processing)",
            ]
        );
        assert_eq!(
            decided.continuation,
            Some(ContinuationSpec {
                recording_id: rid,
                kind: ContKind::Pipeline
            }),
            "the flight registers K:Pipeline for the id ID+ allocates"
        );
        assert_eq!(decided.reply, None, "the pipeline continues in the caller");
        assert_eq!(decided.validate(), Ok(()));
    }
}

#[test]
fn no_process_audio_decision_ever_claims_a_terminal() {
    let mut decided = vec![decide_process_audio_start(
        &core(Status::Idle, RID),
        input(false, false, false, Some(false)),
        NOW,
    )];
    for status in Status::ALL {
        for current in [RID, NEWER] {
            for kind in [
                ProcessAudioFinishKind::ParseFailed,
                ProcessAudioFinishKind::PipelineDone,
            ] {
                decided.push(decide_process_audio_finish(
                    &core(status, current),
                    ProcessAudioFinishInput {
                        rid: RID,
                        kind,
                        closure_emit: true,
                    },
                    NOW,
                ));
            }
        }
    }
    for decision in decided {
        assert!(
            decision
                .effects
                .iter()
                .all(|p| !matches!(p.effect, Effect::Terminal(_))),
            "{:?}: no TERM is ever emitted by process_audio (A0 C2)",
            decision.row
        );
    }
}

#[test]
fn the_start_decision_is_pure() {
    let core = core(Status::Idle, RID);
    let one = decide_process_audio_start(&core, input(false, false, false, Some(false)), NOW);
    let two = decide_process_audio_start(&core, input(false, false, false, Some(false)), NOW);
    assert_eq!(one, two);
    assert_eq!(core.status, Status::Idle);
    assert_eq!(core.current_id, RID);
}

// ------------------------------------------------------------------ 302

fn finish(
    status: Status,
    current: RecordingId,
    kind: ProcessAudioFinishKind,
    emit: bool,
) -> Decision {
    decide_process_audio_finish(
        &core(status, current),
        ProcessAudioFinishInput {
            rid: RID,
            kind,
            closure_emit: emit,
        },
        NOW,
    )
}

#[test]
fn r302_1_a_parse_failure_is_exactly_the_204a_legacy_parse_rows() {
    // (status, row) for the guard's own id, then for a newer id.
    let current = [
        (Status::Processing, "R204a.1"),
        (Status::Idle, "R204a.2"),
        (Status::Recovering, "R204a.3"),
    ];
    for (status, row) in current {
        let decided = finish(status, RID, ProcessAudioFinishKind::ParseFailed, true);
        assert_eq!(decided.row, RowId(row), "{status:?}");
        assert_eq!(decided.verdict, Verdict::Applied);
        assert_eq!(decided.transition, Some((status, Status::Idle)));
        assert_eq!(
            tags(&decided),
            ["Pre:CX-(7)", "In:ST→Idle", "In:KB(false)", "In:EM(idle)"]
        );
        assert_eq!(decided.reply, Some(Reply::Err(ErrText::Shell)));
        assert_eq!(decided.continuation, None);
        assert_eq!(decided.validate(), Ok(()));
    }
    let newer = [
        (Status::Idle, "R204a.5a"),
        (Status::Starting, "R204a.5b"),
        (Status::Recording, "R204a.5c"),
        (Status::Recovering, "R204a.5d"),
        (Status::Processing, "R204a.5e"),
    ];
    for (status, row) in newer {
        let decided = finish(status, NEWER, ProcessAudioFinishKind::ParseFailed, true);
        assert_eq!(decided.row, RowId(row), "{status:?}");
        assert_eq!(
            decided.verdict,
            Verdict::Stale(crate::decision::StaleKind::StaleRecording)
        );
        assert_eq!(
            decided.transition, None,
            "a newer recording's status is not touched"
        );
        // `CX-` clears the slot only if it still holds the guard's id; no `ST`,
        // `KB` or `EM` for a stale guard.
        assert_eq!(tags(&decided), ["Pre:CX-(7)"]);
        assert_eq!(decided.reply, Some(Reply::Err(ErrText::Shell)));
    }
}

#[test]
fn r302_1_a_deferred_parse_finish_has_no_closure_to_emit_idle() {
    let decided = finish(
        Status::Processing,
        RID,
        ProcessAudioFinishKind::ParseFailed,
        false,
    );
    assert_eq!(tags(&decided), ["Pre:CX-(7)", "In:ST→Idle", "In:KB(false)"]);
}

#[test]
fn r302_1_equals_the_guard_decision_with_a_reply() {
    for status in Status::ALL {
        for current in [RID, NEWER] {
            let guard = decide_guard_finish(
                &core(status, current),
                GuardFinishInput {
                    rid: RID,
                    origin: GuardOrigin::LegacyParse,
                    closure_emit: true,
                },
                NOW,
            );
            let mut legacy = finish(status, current, ProcessAudioFinishKind::ParseFailed, true);
            assert_eq!(legacy.reply.take(), Some(Reply::Err(ErrText::Shell)));
            assert_eq!(legacy, guard, "{status:?} {current:?}");
        }
    }
}

#[test]
fn r302_2_the_tail_after_the_pipeline_is_exactly_the_204b_legacy_post_rows() {
    let current = [(Status::Idle, "R204b.1"), (Status::Recovering, "R204b.2")];
    for (status, row) in current {
        let decided = finish(status, RID, ProcessAudioFinishKind::PipelineDone, true);
        assert_eq!(decided.row, RowId(row), "{status:?}");
        assert_eq!(decided.verdict, Verdict::Applied);
        assert_eq!(decided.transition, None, "204b never writes status");
        assert_eq!(tags(&decided), ["In:KB(false)", "In:EM(idle)"]);
        assert_eq!(
            decided.reply,
            Some(Reply::Transcription {
                text: TranscriptText::FromPipeline,
                state: Status::Idle
            })
        );
    }
    let newer = [
        (Status::Idle, "R204b.4a"),
        (Status::Starting, "R204b.4b"),
        (Status::Recording, "R204b.4c"),
        (Status::Recovering, "R204b.4d"),
        (Status::Processing, "R204b.4e"),
    ];
    for (status, row) in newer {
        let decided = finish(status, NEWER, ProcessAudioFinishKind::PipelineDone, true);
        assert_eq!(decided.row, RowId(row), "{status:?}");
        assert_eq!(decided.transition, None);
        assert!(
            decided.effects.is_empty(),
            "a newer start owns idle: no KB, no EM"
        );
    }
    // `closure_emit` is a parse-failure fact: the tail ignores it.
    assert_eq!(
        finish(
            Status::Idle,
            RID,
            ProcessAudioFinishKind::PipelineDone,
            false
        ),
        finish(
            Status::Idle,
            RID,
            ProcessAudioFinishKind::PipelineDone,
            true
        )
    );
}

#[test]
fn r302_2_equals_the_stop_tail_decision_with_a_reply() {
    for status in Status::ALL {
        for current in [RID, NEWER] {
            let tail = decide_pipeline_idle_emit(
                &core(status, current),
                PipelineIdleEmitInput {
                    rid: RID,
                    kind: PostKind::LegacyPost,
                    result: None,
                    interrupted: None,
                    char_count: 0,
                },
                NOW,
            );
            let mut legacy = finish(status, current, ProcessAudioFinishKind::PipelineDone, true);
            assert!(legacy.reply.take().is_some());
            assert_eq!(legacy, tail, "{status:?} {current:?}");
        }
    }
}

// ------------------------------------------------------------ registry

#[test]
fn every_c3_7_row_is_covered_by_a_test_here_and_registered() {
    let c37: Vec<&str> = rows_of_site(SiteId::ProcessAudioStart)
        .chain(rows_of_site(SiteId::ProcessAudioFinish))
        .collect();
    assert_eq!(
        c37,
        ["R301.1", "R301.2", "R301.3", "R301.4", "R301.5", "R302.1", "R302.2"]
    );
    // R301.1-.5 are decided by name above; R302.1 / R302.2 are the 204a / 204b
    // rows listed in C3.7, which must exist in the registry.
    for row in [
        "R204a.1", "R204a.2", "R204a.3", "R204a.5a", "R204a.5b", "R204a.5c", "R204a.5d",
        "R204a.5e", "R204b.1", "R204b.2", "R204b.4a", "R204b.4b", "R204b.4c", "R204b.4d",
        "R204b.4e",
    ] {
        assert!(C3_ROWS.iter().any(|(id, _)| *id == row), "{row}");
    }
}
