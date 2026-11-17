//! Stage 2a-5: the legacy `process_audio` decide functions (appendix C2.3 sites
//! 301 and 302; rows of C3.7, kept by C11.2).
//!
//! `process_audio` has no frontend caller and emits no terminal (no `TERM` is
//! ever decided here). Its two D sections are decided by pure functions that
//! mutate nothing: fields commit at their effects (C1.4.2) and H1/H2 are the
//! shell's bookkeeping right after the decision (C1.4.1).
//!
//! * [`decide_process_audio_start`] (301, rows R301.1-R301.5): the refusals in
//!   today's order (meeting before D; file transcription, benchmark and status
//!   under D), then R301.5, the accepted import: `In:ID+`, `In:ST→Processing`,
//!   `In:AX(s)`, `PT:KB(true)`, `PT:EV(GenStarted)`, `PT:EM(processing)` and the
//!   continuation `K:Pipeline` (the shell registers H1 and H2 together).
//! * [`decide_process_audio_finish`] (302, rows R302.1-R302.2): **exactly** the 204a
//!   rows with `origin = LegacyParse` (the parse failure) and the 204b rows with
//!   `kind = LegacyPost` (the tail after the pipeline), by C3.7. They are
//!   delegations: the returned [`Decision`] carries the R204a / R204b row id, so a
//!   fix to the shared guard rows reaches `process_audio` automatically.
//!
//! Code wins over the appendix in three places (listed in the README):
//! the meeting check (R301.1) is a pure function of the input and runs with no D
//! ([`decide_process_audio_start_pre`], the `decide_start_request_pre` precedent);
//! `benchmark.is_running()` is read through `NeedObservation(BenchmarkRunning)`
//! because it takes the coordinator mutex and today's code reads it only after the
//! file check passed (D-S1-2); and `process_audio`'s success reply has no `state`
//! key, which the shell keeps.

use crate::decision::{ContKind, ContinuationSpec, Decision, ObsKind, Refusal, Verdict};
use crate::effects::{AxMode, EventKind, Placement as At};
use crate::ids::Now;
use crate::inputs::{
    GuardFinishInput, GuardOrigin, PipelineIdleEmitInput, PostKind, ProcessAudioFinishInput,
    ProcessAudioFinishKind, ProcessAudioStartInput,
};
use crate::lifecycle::{decision, emit_event, emit_status, nothing, placed, set_status};
use crate::replies::{ErrText, ProcessAudioRefusal, Reply, TranscriptText};
use crate::state::DictationCore;
use crate::status::Status;
use crate::stop::{decide_guard_finish, decide_pipeline_idle_emit};

fn refused(kind: ProcessAudioRefusal, row: &'static str) -> Decision {
    let mut decided = nothing(Verdict::Refused(Refusal::ProcessAudio(kind)), row);
    decided.reply = Some(Reply::Err(ErrText::ProcessAudio(kind)));
    decided
}

/// R301.1 (3684): a meeting (or its transcript recovery) owns ASR. Decided with no
/// D, from the input alone. `None` when the meeting does not block.
pub fn decide_process_audio_start_pre(
    input: &ProcessAudioStartInput,
    _now: Now,
) -> Option<Decision> {
    input
        .occupancy
        .meeting_blocks_asr()
        .then(|| refused(ProcessAudioRefusal::MeetingActive, "R301.1"))
}

/// Site 301, `decide_process_audio_start` (R301.1-R301.5; T held, D section
/// 3691-3730 in today's numbering).
///
/// Order, as today's code evaluates it:
///
/// 1. R301.1 meeting blocks ASR (no D): `Err("Cannot process audio while a meeting
///    transcript is active.")`.
/// 2. R301.2 `file_transcribing`: `Err("... a file transcription is in progress.")`.
/// 3. `benchmark_running` unread: `NeedObservation(BenchmarkRunning)`, before any
///    mutation. R301.3 running: `Err("... a benchmark is in progress.")`.
/// 4. R301.4 status is not `Idle` (`Starting`, `Recording`, `Recovering`,
///    `Processing`): `Err("Cannot process audio while live dictation is active.")`.
/// 5. R301.5 `Idle`, no blocker: `current_id + 1`, `Idle→Processing`; `In:ID+`
///    (first, before `AX`), `In:ST→Processing`, `In:AX(s)`; `PT:KB(true)`,
///    `PT:EV(GenStarted)`, `PT:EM(processing)` (T was dropped between D and them).
///    Continuation `K:Pipeline(rid)`; the pipeline runs in the caller, and no
///    terminal is emitted anywhere in the command. No reply.
///
/// Refusals are pure: no mutation, no effect. The cleanup-pending check of the
/// native start is absent here by design (R216.5's note in C3.6).
pub fn decide_process_audio_start(
    core: &DictationCore,
    input: ProcessAudioStartInput,
    now: Now,
) -> Decision {
    if let Some(pre) = decide_process_audio_start_pre(&input, now) {
        return pre;
    }
    if input.occupancy.file_transcribing {
        return refused(ProcessAudioRefusal::FileTranscription, "R301.2");
    }
    match input.benchmark_running {
        None => {
            return nothing(
                Verdict::NeedObservation(ObsKind::BenchmarkRunning),
                "R301.3",
            )
        }
        Some(true) => return refused(ProcessAudioRefusal::Benchmark, "R301.3"),
        Some(false) => {}
    }
    match core.status {
        Status::Idle => {}
        Status::Starting | Status::Recording | Status::Recovering | Status::Processing => {
            return refused(ProcessAudioRefusal::LiveDictation, "R301.4");
        }
    }
    // `ID+` allocates `current_id + 1`, a pure function of the core (C0.2).
    let rid = core.next_id();
    let mut decided = decision(
        Verdict::Applied,
        Some((Status::Idle, Status::Processing)),
        vec![
            placed(At::In, crate::effects::Effect::AllocId),
            set_status(Status::Processing),
            placed(At::In, crate::effects::Effect::AxSnapshot(AxMode::Stop)),
            placed(At::PostT, crate::effects::Effect::KeyboardProcessing(true)),
            emit_event(At::PostT, EventKind::GenStarted),
            emit_status(At::PostT, Status::Processing),
        ],
        "R301.5",
    );
    decided.continuation = Some(ContinuationSpec {
        recording_id: rid,
        kind: ContKind::Pipeline,
    });
    decided
}

/// Site 302, `decide_process_audio_finish` (R302.1-R302.2).
///
/// * [`ProcessAudioFinishKind::ParseFailed`] (R302.1, 3780 / 3786): **exactly** the
///   204a rows with `origin = LegacyParse` (R204a.1 Processing, .2 Idle, .3
///   Recovering, .5a-e a newer id), `closure_emit` as the guard has it. The reply is
///   `Err(<the parse error>)` (shell text). The shell retires `H2 rid` and `H1
///   op_pl` after this returns and before the `In` effects (C3.9).
/// * [`ProcessAudioFinishKind::PipelineDone`] (R302.2, 3817-3821): **exactly** the
///   204b rows with `kind = LegacyPost` (R204b.1 Idle, .2 Recovering, .4a-e a newer
///   id). The pipeline's own guard (204a `PipelineExit`) already ran and retired
///   `H2` / `H1`. The reply is the `transcription` JSON, whose text is the
///   pipeline's.
pub fn decide_process_audio_finish(
    core: &DictationCore,
    input: ProcessAudioFinishInput,
    now: Now,
) -> Decision {
    match input.kind {
        ProcessAudioFinishKind::ParseFailed => {
            let mut decided = decide_guard_finish(
                core,
                GuardFinishInput {
                    rid: input.rid,
                    origin: GuardOrigin::LegacyParse,
                    closure_emit: input.closure_emit,
                },
                now,
            );
            decided.reply = Some(Reply::Err(ErrText::Shell));
            decided
        }
        ProcessAudioFinishKind::PipelineDone => {
            let mut decided = decide_pipeline_idle_emit(
                core,
                PipelineIdleEmitInput {
                    rid: input.rid,
                    kind: PostKind::LegacyPost,
                    result: None,
                    interrupted: None,
                    char_count: 0,
                },
                now,
            );
            decided.reply = Some(Reply::Transcription {
                text: TranscriptText::FromPipeline,
                state: Status::Idle,
            });
            decided
        }
    }
}

#[cfg(test)]
mod tests;
