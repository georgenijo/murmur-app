//! Stage 1b shell wiring for `murmur_dictation_core::admission::admit`.
//!
//! Every admission predicate in the app is evaluated by `admit()` /
//! `admit_after()` at the call site that read the facts before, under the lock
//! that call site already holds (appendix C6.4). This module only **gathers**
//! facts and **maps** results; it never claims, locks (beyond the dictation
//! lock the old gate already took) or releases a resource, and it does not make
//! admission atomic.
//!
//! # How a call site reads
//!
//! `admit` is a pure function of an [`Occupancy`]. Old predicates read their
//! facts lazily and in order: a refusal returned before the later facts were
//! read, and several of those reads are not free (`audio_lifecycle::is_audio_active`
//! constructs the global supervisor on first use, the supervisor ports are seams
//! the Stage 0a contract tests count, and a mutex read under the dictation lock
//! is a lock acquisition). [`Staged`] keeps that: facts are read one group at a
//! time, in the old order, and `admit` is evaluated after each group. Facts not
//! read yet are `Default` (quiet), which no check treats as a refusal, so a
//! partial occupancy admits exactly when every check before the unread facts
//! passes. The first non-`Admit` result ends the call, so a fact after a refusal
//! is never read, exactly as before.
//!
//! A retry after a wait/retire resumes with `admit_after`
//! ([`Staged::resuming`]): the retire happens where it did, unconditionally and
//! before the reads, and evaluation continues at the check after the wait.

use murmur_dictation_core::admission::{
    admit, admit_after, Admission, KeyDownShellFacts, ModelPrepVariant, Occupancy, RefusalCode,
    Request, Retry, SmartAutoObservation,
};
use murmur_dictation_core::ids::RecordingId;
use murmur_dictation_core::status::Status;
use std::sync::atomic::Ordering;

use crate::state::{AppState, DictationStatus};
use crate::{audio_lifecycle, MutexExt, State};

/// The core's view of the app's dictation status (same five states).
pub(crate) fn core_status(status: DictationStatus) -> Status {
    match status {
        DictationStatus::Idle => Status::Idle,
        DictationStatus::Starting => Status::Starting,
        DictationStatus::Recording => Status::Recording,
        DictationStatus::Recovering => Status::Recovering,
        DictationStatus::Processing => Status::Processing,
    }
}

/// A point where the shell has already done a wait/retire and evaluation
/// resumes after it (`admit_after`). The set is closed: each variant names the
/// request and the wait it resumes after, and
/// `every_resume_point_is_a_wait_of_its_request` proves `admit_after` accepts
/// each pair, so no pairing can be invalid at run time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Resume {
    /// `PreviewStart` after the unconditional `retire_standby()`.
    PreviewAfterStandby,
    /// `PreviewStart` after the bounded retire of an unconfirmed key-down capture.
    PreviewAfterKeyDownRetire,
    /// `MicStartupBenchmarkPreClaim` after the unconditional `retire_standby()`.
    MicBenchmarkAfterStandby,
}

impl Resume {
    #[cfg(test)]
    const ALL: [Resume; 3] = [
        Self::PreviewAfterStandby,
        Self::PreviewAfterKeyDownRetire,
        Self::MicBenchmarkAfterStandby,
    ];

    const fn point(self) -> (Request, Retry) {
        match self {
            Self::PreviewAfterStandby => (Request::PreviewStart, Retry::RetireStandby),
            Self::PreviewAfterKeyDownRetire => (Request::PreviewStart, Retry::RetireKeyDownCapture),
            Self::MicBenchmarkAfterStandby => {
                (Request::MicStartupBenchmarkPreClaim, Retry::RetireStandby)
            }
        }
    }
}

/// An occupancy read one fact group at a time (see the module docs).
pub(crate) struct Staged {
    request: Request,
    resume: Option<Retry>,
    occupancy: Occupancy,
}

impl Staged {
    pub(crate) fn new(request: Request) -> Self {
        Self {
            request,
            resume: None,
            occupancy: Occupancy::default(),
        }
    }

    /// Evaluate only the checks after the wait at `point`: the shell has
    /// already done the retire (`admit_after`).
    pub(crate) fn resuming(point: Resume) -> Self {
        let (request, retry) = point.point();
        Self {
            request,
            resume: Some(retry),
            occupancy: Occupancy::default(),
        }
    }

    fn evaluate(&self) -> Admission {
        match self.resume {
            None => admit(self.request, &self.occupancy),
            Some(retry) => {
                admit_after(self.request, retry, &self.occupancy)
                    .unwrap_or_else(|| {
                        unreachable!("{:?} has no {retry:?} wait to resume after", self.request)
                    })
                    .admission
            }
        }
    }

    /// Read one group of facts, at the point today's code reads them, then
    /// evaluate. `Err` carries the first result that is not `Admit`; the caller
    /// returns it without reading anything further.
    pub(crate) fn read(&mut self, gather: impl FnOnce(&mut Occupancy)) -> Result<(), Admission> {
        gather(&mut self.occupancy);
        match self.evaluate() {
            Admission::Admit(_) => Ok(()),
            other => Err(other),
        }
    }

    /// The result once every group has been read.
    pub(crate) fn finish(self) -> Admission {
        self.evaluate()
    }
}

/// Run `read_all` and return its first non-`Admit` result, or the final
/// admission when every fact group passed.
pub(crate) fn staged(
    mut session: Staged,
    read_all: impl FnOnce(&mut Staged) -> Result<(), Admission>,
) -> Admission {
    match read_all(&mut session) {
        Err(stop) => stop,
        Ok(()) => session.finish(),
    }
}

/// The refusal's text. Every caller owns a request and an occupancy for which
/// a refusal is the only possible result (a failed claim's own answer, or a
/// read that already refused); anything else is a wiring bug, not a reply.
pub(crate) fn refusal_text(admission: Admission) -> String {
    match admission {
        Admission::RefuseWith(code) => code.text().to_string(),
        Admission::Admit(_)
        | Admission::Takeover(_)
        | Admission::WaitThen(_)
        | Admission::NeedObservation(_) => {
            unreachable!("a refusal was required, got {admission:?}")
        }
    }
}

/// `Ok` when admitted, the refusal's text otherwise. The requests that use
/// this are evaluated past every wait, and none can take over a preview or
/// need an observation.
fn text_if_refused(admission: Admission) -> Result<(), String> {
    match admission {
        Admission::Admit(_) => Ok(()),
        Admission::RefuseWith(code) => Err(code.text().to_string()),
        Admission::Takeover(_) | Admission::WaitThen(_) | Admission::NeedObservation(_) => {
            unreachable!("this request cannot return {admission:?}")
        }
    }
}

/// The `&'static str` of a refusal whose callers return a static tag or
/// message. Only the two variants that carry shell text (`SmartAutoFailed`,
/// `Text`) have none; no request that reaches this function evaluates the checks
/// that produce them (`static_callers_never_see_a_payload_refusal`).
pub(crate) fn static_text(code: &RefusalCode) -> &'static str {
    match code.static_text() {
        Ok(literal) => literal,
        Err(_) => unreachable!("{code:?} carries shell text and has no static literal"),
    }
}

// ---------------------------------------------------------------------------
// Fact readers: one per fact, in the shape today's predicates read it.
// ---------------------------------------------------------------------------

mod facts {
    use super::*;

    pub(super) fn relaunch(o: &mut Occupancy) {
        o.relaunch_committed = crate::commands::permissions::relaunch_committed();
    }

    /// `AppState::meeting_blocks_asr`: a meeting or its inference recovery.
    pub(super) fn meeting(app_state: &AppState, o: &mut Occupancy) {
        o.meeting_active = app_state.meeting_active.load(Ordering::SeqCst);
        o.meeting_inference_active = app_state.meeting_inference_active.load(Ordering::SeqCst);
    }

    pub(super) fn file(app_state: &AppState, o: &mut Occupancy) {
        o.file_transcribing = app_state.file_transcribing.load(Ordering::SeqCst);
    }

    pub(super) fn corpus(state: &State, o: &mut Occupancy) {
        #[cfg(feature = "internal-benchmark")]
        {
            o.corpus_active = state.corpus.is_active();
        }
        #[cfg(not(feature = "internal-benchmark"))]
        {
            let _ = (state, o);
        }
    }

    /// `BenchmarkCoordinator::is_running` (`CoordinatorActivity::Benchmark` only).
    pub(super) fn benchmark(state: &State, o: &mut Occupancy) {
        o.benchmark_running = state.benchmark.is_running();
    }

    pub(super) fn status(status: DictationStatus, o: &mut Occupancy) {
        o.status = core_status(status);
    }

    pub(super) fn audio_active(o: &mut Occupancy) {
        o.audio_active = audio_lifecycle::is_audio_active();
    }

    pub(super) fn app_disabled(o: &mut Occupancy) {
        o.app_disabled = crate::keyboard::is_app_disabled();
    }
}

// ---------------------------------------------------------------------------
// Dictation start (recording.rs `start_native_recording_inner`)
// ---------------------------------------------------------------------------

/// `dictation_start_blocker`: relaunch → meeting → file → corpus → benchmark →
/// status, under the dictation lock the caller holds. The start's own
/// `cleanup_pending` and Smart Auto reads follow at the start site.
pub(crate) fn dictation_start_admission(state: &State, status: DictationStatus) -> Admission {
    staged(Staged::new(Request::DictationStart), |s| {
        s.read(facts::relaunch)?;
        s.read(|o| facts::meeting(&state.app_state, o))?;
        s.read(|o| facts::file(&state.app_state, o))?;
        s.read(|o| facts::corpus(state, o))?;
        s.read(|o| facts::benchmark(state, o))?;
        s.read(|o| facts::status(status, o))
    })
}

/// The start's pre-D checks (`app_disabled`, then Smart Auto with a fixed
/// device): `Ok` or the refusal, with the facts exactly as the start read them.
pub(crate) fn dictation_start_pre_admission(
    app_disabled: bool,
    smart_auto_with_fixed_device: bool,
) -> Admission {
    admit(
        Request::DictationStart,
        &Occupancy {
            app_disabled,
            fixed_and_smart_auto: smart_auto_with_fixed_device,
            ..Occupancy::default()
        },
    )
}

/// The start's retiring-capture check: `cleanup_pending(current_id)` was read
/// by the caller under the same dictation-lock hold, after the status check
/// passed (status is `Idle` here), and before the Smart Auto resolve.
pub(crate) fn dictation_start_cleanup_admission(cleanup_pending: bool) -> Admission {
    admit(
        Request::DictationStart,
        &Occupancy {
            cleanup_pending,
            ..Occupancy::default()
        },
    )
}

/// The start's Smart Auto step. `NeedObservation` is answered by the caller's
/// resolve, then the result is passed back here.
pub(crate) fn dictation_start_smart_auto_admission(observation: SmartAutoObservation) -> Admission {
    admit(
        Request::DictationStart,
        &Occupancy {
            smart_auto: observation,
            ..Occupancy::default()
        },
    )
}

// ---------------------------------------------------------------------------
// Prepare / key-down / relaunch gate (D held for the whole gate, as before)
// ---------------------------------------------------------------------------

/// `dictation_refusal` for `DictationPrepare`, `KeyDownCapture` and
/// `RelaunchGate`: the blockers → preview → `audio_active` → `cleanup_pending`,
/// all read under the dictation lock taken here.
fn dictation_gate(state: &State, request: Request) -> Admission {
    debug_assert!(matches!(
        request,
        Request::DictationPrepare | Request::KeyDownCapture | Request::RelaunchGate
    ));
    let dictation = state.app_state.dictation.lock_or_recover();
    let preview = &state.app_state.microphone_preview;
    let mut session = Staged::new(request);
    let blocked = (|| -> Result<(), Admission> {
        // The relaunch's own commitment is not activity to the relaunch gate.
        if request != Request::RelaunchGate {
            session.read(facts::relaunch)?;
        }
        session.read(|o| facts::meeting(&state.app_state, o))?;
        session.read(|o| facts::file(&state.app_state, o))?;
        session.read(|o| facts::corpus(state, o))?;
        session.read(|o| facts::benchmark(state, o))?;
        session.read(|o| facts::status(dictation.status, o))?;
        session.read(|o| o.preview_active = preview.is_active())
    })();
    match blocked {
        // A visible Settings test is taken over in place: the handoff candidate
        // was asked only after the gate returned (dictation lock released).
        Err(Admission::RefuseWith(RefusalCode::PreviewActive))
            if request == Request::KeyDownCapture =>
        {
            drop(dictation);
            match session.read(|o| o.handoff_candidate = preview.handoff_candidate()) {
                Err(takeover_or_refusal) => takeover_or_refusal,
                Ok(()) => session.finish(),
            }
        }
        Err(refusal) => refusal,
        Ok(()) => {
            let rest = (|| -> Result<(), Admission> {
                session.read(facts::audio_active)?;
                session.read(|o| {
                    o.cleanup_pending = audio_lifecycle::is_dictation_cleanup_pending(
                        state.app_state.recording_id.load(Ordering::SeqCst),
                    );
                })
            })();
            match rest {
                Err(refusal) => refusal,
                Ok(()) => session.finish(),
            }
        }
    }
}

/// The static tag of a gate's result: `None` when admitted. A gate never waits,
/// takes over (the key-down caller handles `Takeover` before this) or needs an
/// observation.
fn tag_of(admission: &Admission) -> Option<&'static str> {
    match admission {
        Admission::Admit(_) => None,
        Admission::RefuseWith(code) => Some(static_text(code)),
        Admission::Takeover(_) | Admission::WaitThen(_) | Admission::NeedObservation(_) => {
            unreachable!("a gate cannot return {admission:?} here")
        }
    }
}

/// The first content-free reason a prepared capture may not open, or `None`.
pub(crate) fn dictation_prepare_refusal(state: &State) -> Option<&'static str> {
    tag_of(&dictation_gate(state, Request::DictationPrepare))
}

/// Whether silent first-tap preparation may spawn an idle capture helper.
pub(crate) fn dictation_prepare_admitted(state: &State) -> bool {
    dictation_prepare_refusal(state).is_none()
}

/// Whether anything dictation-related is in progress, for a permission
/// relaunch. A relaunch that has itself committed does not count.
pub(crate) fn dictation_relaunch_busy(state: &State) -> Option<&'static str> {
    tag_of(&dictation_gate(state, Request::RelaunchGate))
}

/// Admission for an Fn key-down capture: `Ok(Some(preview))` when the only
/// obstacle is the visible Settings microphone test, which the capture takes
/// over in place instead of being skipped. Any other refusal is unchanged.
pub(crate) fn key_down_capture_admission(state: &State) -> Result<Option<u64>, &'static str> {
    match dictation_gate(state, Request::KeyDownCapture) {
        Admission::Takeover(preview) => Ok(Some(preview.0)),
        Admission::RefuseWith(code) => Err(static_text(&code)),
        Admission::Admit(_) => Ok(None),
        Admission::WaitThen(_) | Admission::NeedObservation(_) => {
            unreachable!("key-down admission neither waits nor needs an observation")
        }
    }
}

/// The Fn key-down shell pre-checks (`fn_capture.rs` `admission_refusal`): the
/// facts the caller gathered, evaluated in the gate's order.
pub(crate) fn key_down_shell_refusal(
    listener_matches: bool,
    processing: bool,
    disabled: bool,
    gesture_active: bool,
    microphone_permission: &str,
) -> Option<&'static str> {
    let admission = admit(
        Request::KeyDownCapture,
        &Occupancy {
            keyboard_processing: processing,
            app_disabled: disabled,
            key_down: KeyDownShellFacts {
                listener_matches,
                gesture_active,
                microphone_granted: microphone_permission == "granted",
            },
            ..Occupancy::default()
        },
    );
    tag_of(&admission)
}

// ---------------------------------------------------------------------------
// File transcription (recording.rs `transcribe_file`)
// ---------------------------------------------------------------------------

/// `FileTranscriptionPreClaim`, under the recording transition the caller took:
/// the pre-admission preview stop's error first, then the meeting check, then
/// the claim. The atomic swap and its returned prior value stay here, run only
/// after the meeting check passes (recording.rs's old order); the transition
/// is handed back on success and the caller drops it.
pub(crate) fn file_transcription_pre_claim<T>(
    app_state: &AppState,
    stopped: Result<T, String>,
) -> Result<T, String> {
    let mut session = Staged::new(Request::FileTranscriptionPreClaim);
    let outcome = (|| -> Result<(), Admission> {
        session
            .read(|o| o.preview_stop = Some(stopped.as_ref().map(|_| ()).map_err(Clone::clone)))?;
        session.read(|o| facts::meeting(app_state, o))?;
        // The claim: `file_transcribing.swap(true)`; its prior value is the
        // admission fact (a pre-claim flag load could not stand in for it).
        let prior = app_state.file_transcribing.swap(true, Ordering::SeqCst);
        session.read(|o| o.file_claim_prior = Some(prior))
    })();
    match outcome {
        Err(refusal) => Err(refusal_text(refusal)),
        // A failed stop refused above, so `stopped` is the transition here.
        Ok(()) => stopped,
    }
}

/// `FileTranscriptionPostClaim`, under the dictation lock (`status` read from
/// it) with the recording transition already dropped.
pub(crate) fn file_transcription_post_claim(
    state: &State,
    status: DictationStatus,
) -> Result<(), String> {
    text_if_refused(staged(
        Staged::new(Request::FileTranscriptionPostClaim),
        |s| {
            s.read(|o| facts::benchmark(state, o))?;
            s.read(|o| facts::corpus(state, o))?;
            s.read(|o| facts::status(status, o))
        },
    ))
}

// ---------------------------------------------------------------------------
// Meeting (commands/meeting.rs)
// ---------------------------------------------------------------------------

/// `meeting_conflict`: relaunch → mic-startup check → corpus → `meeting_active`
/// → `meeting_inference_active` → `audio_active` → file → benchmark → status
/// (the dictation lock is taken only for that last read, as before).
pub(crate) fn meeting_pre_claim(state: &State) -> Option<&'static str> {
    let app_state = &state.app_state;
    let admission = staged(Staged::new(Request::MeetingPreClaim), |s| {
        s.read(facts::relaunch)?;
        s.read(|o| {
            o.mic_startup_benchmark_active = state.microphone_startup_benchmark.is_active()
        })?;
        s.read(|o| facts::corpus(state, o))?;
        s.read(|o| o.meeting_active = app_state.meeting_active.load(Ordering::SeqCst))?;
        s.read(|o| {
            o.meeting_inference_active = app_state.meeting_inference_active.load(Ordering::SeqCst);
        })?;
        s.read(facts::audio_active)?;
        s.read(|o| facts::file(app_state, o))?;
        s.read(|o| facts::benchmark(state, o))?;
        s.read(|o| facts::status(app_state.dictation.lock_or_recover().status, o))
    });
    tag_of(&admission)
}

/// `MeetingPostClaim`: the `relaunch_committed` re-read after the claim stored
/// its flags. `Err` makes the caller undo the claim.
pub(crate) fn meeting_post_claim(relaunch_committed: bool) -> Result<(), &'static str> {
    match admit(
        Request::MeetingPostClaim,
        &Occupancy {
            relaunch_recheck: Some(relaunch_committed),
            ..Occupancy::default()
        },
    ) {
        Admission::RefuseWith(code) => Err(static_text(&code)),
        Admission::Admit(_) => Ok(()),
        Admission::Takeover(_) | Admission::WaitThen(_) | Admission::NeedObservation(_) => {
            unreachable!("the post-claim recheck returns only Admit or a refusal")
        }
    }
}

// ---------------------------------------------------------------------------
// Microphone preview start (commands/microphone_preview.rs)
// ---------------------------------------------------------------------------

/// `refuse_revoked_start`: the first read after the caller's unconditional
/// `retire_standby()`, so evaluation resumes after the `standby` wait.
pub(crate) fn preview_start_revoked_refusal(
    preview: &crate::microphone_preview::MicrophonePreviewState,
    start_generation: u64,
) -> Result<(), String> {
    text_if_refused(staged(Staged::resuming(Resume::PreviewAfterStandby), |s| {
        s.read(|o| o.preview_start_revoked = preview.start_revoked(start_generation))
    }))
}

/// The preview start's reads under the dictation lock the caller holds:
/// status → meeting → benchmark → corpus → `app_disabled` → `preview_active`.
/// The microphone-owner check that follows is [`preview_audio_admission`].
pub(crate) fn preview_start_admission(
    state: &State,
    status: DictationStatus,
) -> Result<(), String> {
    text_if_refused(staged(Staged::resuming(Resume::PreviewAfterStandby), |s| {
        s.read(|o| facts::status(status, o))?;
        s.read(|o| facts::meeting(&state.app_state, o))?;
        s.read(|o| facts::benchmark(state, o))?;
        s.read(|o| facts::corpus(state, o))?;
        s.read(facts::app_disabled)?;
        s.read(|o| o.preview_active = state.app_state.microphone_preview.is_active())
    }))
}

/// `preview_audio_blocker`: an unconfirmed Fn key-down capture is retired once
/// (`WaitThen`), then `audio_active` decides (`admit_after` resumes after the
/// wait, whether or not the capture is gone).
pub(crate) fn preview_audio_admission(
    unconfirmed_key_down: impl Fn() -> bool,
    retire_key_down: impl FnOnce(),
    audio_active: impl Fn() -> bool,
) -> Option<&'static str> {
    let mut session = Staged::resuming(Resume::PreviewAfterStandby);
    match session.read(|o| o.unconfirmed_key_down = unconfirmed_key_down()) {
        Err(Admission::WaitThen(Retry::RetireKeyDownCapture)) => {
            retire_key_down();
            // Resume after the wait: only `audio_active` remains, read fresh,
            // whether or not the capture is gone.
            let mut after = Staged::resuming(Resume::PreviewAfterKeyDownRetire);
            after
                .read(|o| o.audio_active = audio_active())
                .err()
                .as_ref()
                .and_then(tag_of)
        }
        Err(other) => tag_of(&other),
        Ok(()) => session
            .read(|o| o.audio_active = audio_active())
            .err()
            .as_ref()
            .and_then(tag_of),
    }
}

/// `PreviewStartRecheck`: `is_connecting(id)` after the claim, in the spawned
/// task. `false` means the preview is no longer connecting (silent refusal).
pub(crate) fn preview_still_connecting(
    preview: &crate::microphone_preview::MicrophonePreviewState,
    preview_id: u64,
) -> bool {
    let admission = admit(
        Request::PreviewStartRecheck,
        &Occupancy {
            preview_still_connecting: Some(preview.is_connecting(preview_id)),
            ..Occupancy::default()
        },
    );
    !matches!(
        admission,
        Admission::RefuseWith(RefusalCode::PreviewNoLongerConnecting)
    )
}

// ---------------------------------------------------------------------------
// Microphone startup benchmark (commands/microphone_startup_benchmark.rs)
// ---------------------------------------------------------------------------

/// `MicStartupBenchmarkPreClaim` up to `benchmark.try_start()`, under the
/// dictation lock (`status` read from it), after the caller's unconditional
/// `retire_standby()`: meeting → status → file → corpus → `audio_active` →
/// `app_disabled`.
pub(crate) fn microphone_benchmark_pre_claim(
    state: &State,
    status: DictationStatus,
) -> Result<(), String> {
    text_if_refused(staged(
        Staged::resuming(Resume::MicBenchmarkAfterStandby),
        |s| {
            s.read(|o| facts::meeting(&state.app_state, o))?;
            s.read(|o| facts::status(status, o))?;
            s.read(|o| facts::file(&state.app_state, o))?;
            s.read(|o| facts::corpus(state, o))?;
            s.read(facts::audio_active)?;
            s.read(facts::app_disabled)
        },
    ))
}

/// The coordinator refused `try_start()`: "already running" when a benchmark
/// holds it (`is_running`, read after the failed claim as before), otherwise a
/// shared backend change holds it.
pub(crate) fn microphone_benchmark_claim_failed(
    coordinator: &crate::benchmark::BenchmarkCoordinator,
) -> String {
    refusal_text(admit(
        Request::MicStartupBenchmarkPreClaim,
        &Occupancy {
            coordinator_idle: Some(false),
            benchmark_running: coordinator.is_running(),
            ..Occupancy::default()
        },
    ))
}

/// `MicStartupBenchmarkPostClaim`: the second claim's result. `Err` keeps the
/// claim's own text if admit did not refuse.
pub(crate) fn microphone_benchmark_second_claim(
    claimed: Result<u64, String>,
) -> Result<u64, String> {
    let admission = admit(
        Request::MicStartupBenchmarkPostClaim,
        &Occupancy {
            second_claim: Some(claimed.is_ok()),
            ..Occupancy::default()
        },
    );
    match (admission, claimed) {
        (Admission::RefuseWith(code), _) => Err(code.text().to_string()),
        (_, claimed) => claimed,
    }
}

// ---------------------------------------------------------------------------
// Benchmark (commands/benchmark.rs)
// ---------------------------------------------------------------------------

/// `Benchmark`, first read: the meeting check under the transition, before the
/// dictation lock is taken.
pub(crate) fn benchmark_meeting_check(state: &State) -> Result<(), String> {
    text_if_refused(staged(Staged::new(Request::Benchmark), |s| {
        s.read(|o| facts::meeting(&state.app_state, o))
    }))
}

/// `Benchmark` under the dictation lock (`status` read from it): status → file →
/// corpus, up to the coordinator claim.
pub(crate) fn benchmark_pre_claim(state: &State, status: DictationStatus) -> Result<(), String> {
    text_if_refused(staged(Staged::new(Request::Benchmark), |s| {
        s.read(|o| facts::status(status, o))?;
        s.read(|o| facts::file(&state.app_state, o))?;
        s.read(|o| facts::corpus(state, o))
    }))
}

/// The coordinator refused `try_start()` for a benchmark.
pub(crate) fn benchmark_claim_failed(
    coordinator: &crate::benchmark::BenchmarkCoordinator,
) -> String {
    refusal_text(admit(
        Request::Benchmark,
        &Occupancy {
            coordinator_idle: Some(false),
            benchmark_running: coordinator.is_running(),
            ..Occupancy::default()
        },
    ))
}

// ---------------------------------------------------------------------------
// Corpus (commands/corpus.rs, feature `internal-benchmark`)
// ---------------------------------------------------------------------------

/// `Corpus` under the transition and dictation lock (`status` read from it):
/// meeting → status → file → benchmark, up to `corpus.claim()`.
#[cfg(feature = "internal-benchmark")]
pub(crate) fn corpus_pre_claim(state: &State, status: DictationStatus) -> Result<(), String> {
    text_if_refused(staged(Staged::new(Request::Corpus), |s| {
        s.read(|o| facts::meeting(&state.app_state, o))?;
        s.read(|o| facts::status(status, o))?;
        s.read(|o| facts::file(&state.app_state, o))?;
        s.read(|o| facts::benchmark(state, o))
    }))
}

/// `corpus.claim()` refused (a corpus recording is already active); the claim's
/// own error is kept if admit does not refuse.
#[cfg(feature = "internal-benchmark")]
pub(crate) fn corpus_claim_failed(claim_error: String) -> String {
    match admit(
        Request::Corpus,
        &Occupancy {
            corpus_active: true,
            ..Occupancy::default()
        },
    ) {
        Admission::RefuseWith(code) => code.text().to_string(),
        _ => claim_error,
    }
}

// ---------------------------------------------------------------------------
// Shared backend change, model removal, model preparation
// ---------------------------------------------------------------------------

/// `try_start_shared_backend_change()` returned `false` (model selection).
pub(crate) fn shared_backend_change_refusal() -> String {
    refusal_text(admit(
        Request::SharedBackendChange,
        &Occupancy {
            coordinator_idle: Some(false),
            ..Occupancy::default()
        },
    ))
}

/// `ModelRemoval`, first step: the recording-transition `try_lock`. The
/// attempt's outcome is the fact; the guard is handed back when admitted.
pub(crate) fn model_removal_transition_lock<G, E>(attempt: Result<G, E>) -> Result<G, String> {
    let admission = admit(
        Request::ModelRemoval,
        &Occupancy {
            transition_lock_busy: attempt.is_err(),
            ..Occupancy::default()
        },
    );
    match (admission, attempt) {
        (Admission::RefuseWith(code), _) => Err(code.text().to_string()),
        (_, Ok(guard)) => Ok(guard),
        // A failed attempt always refuses above; keep its text regardless.
        (_, Err(_)) => Err(RefusalCode::ModelRemovalTransitionBusy.text().to_string()),
    }
}

/// `ModelRemoval`, second step: the dictation-lock `try_lock`.
pub(crate) fn model_removal_dictation_lock<G>(attempt: Option<G>) -> Result<G, String> {
    let admission = admit(
        Request::ModelRemoval,
        &Occupancy {
            dictation_lock_busy: attempt.is_none(),
            ..Occupancy::default()
        },
    );
    match (admission, attempt) {
        (Admission::RefuseWith(code), _) => Err(code.text().to_string()),
        (_, Some(guard)) => Ok(guard),
        (_, None) => Err(RefusalCode::ModelRemovalDictationBusy.text().to_string()),
    }
}

/// `ModelRemoval`, third step, with both locks held: status → file → meeting →
/// `audio_active`.
pub(crate) fn model_removal_occupancy_check(
    app_state: &AppState,
    status: DictationStatus,
) -> Result<(), String> {
    text_if_refused(staged(Staged::new(Request::ModelRemoval), |s| {
        s.read(|o| facts::status(status, o))?;
        s.read(|o| facts::file(app_state, o))?;
        s.read(|o| {
            o.meeting_active = app_state.meeting_active.load(Ordering::SeqCst);
        })?;
        s.read(|o| {
            o.meeting_inference_active = app_state.meeting_inference_active.load(Ordering::SeqCst);
        })?;
        s.read(facts::audio_active)
    }))
}

/// `try_start_shared_backend_change()` returned `false` for a model removal.
pub(crate) fn model_removal_claim_failed() -> String {
    refusal_text(admit(
        Request::ModelRemoval,
        &Occupancy {
            coordinator_idle: Some(false),
            ..Occupancy::default()
        },
    ))
}

/// `ModelPreparation`: is the worker spawned for `expected_recording_id` still
/// eligible? The generation check and the status come first; the frozen
/// context's model (`model_matches`, which takes the context lock) is read
/// only when both passed, as the old `&&` chain did. The read is requested by
/// optimistically admitting with `model_matches` set: the check is monotone in
/// it, so an admit there means the model must now be read, and a refusal there
/// is a refusal whatever the model is.
pub(crate) fn model_preparation_eligible(
    variant: ModelPrepVariant,
    status: DictationStatus,
    current_recording_id: u64,
    expected_recording_id: Option<u64>,
    model_matches: impl FnOnce() -> bool,
) -> bool {
    let mut occupancy = Occupancy {
        model_prep_variant: variant,
        status: core_status(status),
        recording_id: RecordingId(current_recording_id),
        expected_recording_id: expected_recording_id.map(RecordingId),
        model_matches: true,
        ..Occupancy::default()
    };
    if !matches!(
        admit(Request::ModelPreparation, &occupancy),
        Admission::Admit(_)
    ) {
        return false;
    }
    occupancy.model_matches = model_matches();
    matches!(
        admit(Request::ModelPreparation, &occupancy),
        Admission::Admit(_)
    )
}

/// Per-site proof that routing admission through `admit` changed no reply: each
/// test drives the old refusal paths through the production function and pins
/// the literal the old predicate returned (copied from the pre-Stage-1b code),
/// and the gate tests compare against the old predicate itself.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_lifecycle::{set_public_for_test, PublicForTest};
    use crate::commands::permissions::CommittedRelaunchForTest;
    use crate::commands::recording::{dictation_start_blocker, DictationStartBlocker};
    use crate::microphone_preview::PreviewPhase;

    fn set_status(state: &State, status: DictationStatus) {
        state.app_state.dictation.lock_or_recover().status = status;
    }

    fn blocker(kind: &'static str, state: &'static str) -> Option<DictationStartBlocker> {
        Some(DictationStartBlocker { kind, state })
    }

    // -- the old predicates: literal copies from origin/main ------------------
    //
    // `dictation_start_blocker`, `dictation_activity_blocker` and
    // `dictation_refusal` exactly as `commands/recording.rs` had them before
    // Stage 1b. Only the names carry a `legacy_` prefix (the copies sit beside
    // the new functions); no body line was edited, merged or simplified.

    fn legacy_dictation_start_blocker(
        state: &State,
        status: DictationStatus,
    ) -> Option<DictationStartBlocker> {
        // A permission relaunch that has committed to exiting refuses new work. The
        // callers hold the dictation lock, and the relaunch re-reads the activity
        // gate after it commits, so a start is either seen by that re-read or
        // refused here.
        if crate::commands::permissions::relaunch_committed() {
            return Some(DictationStartBlocker {
                kind: "busy_restarting",
                state: "idle",
            });
        }
        legacy_dictation_activity_blocker(state, status)
    }

    /// What currently occupies dictation, ignoring a committed relaunch. The
    /// relaunch itself asks this, so its own commitment cannot read as activity.
    fn legacy_dictation_activity_blocker(
        state: &State,
        status: DictationStatus,
    ) -> Option<DictationStartBlocker> {
        let blocked = |kind, state| Some(DictationStartBlocker { kind, state });
        if state.app_state.meeting_blocks_asr() {
            return blocked("busy_meeting", "idle");
        }
        // Refuse if a file transcription holds the shared Whisper backend.
        // `transcribe_file` takes the dictation lock only after claiming the flag.
        if state.app_state.file_transcribing.load(Ordering::SeqCst) {
            return blocked("busy_transcribing_file", "idle");
        }
        #[cfg(feature = "internal-benchmark")]
        if state.corpus.is_active() {
            return blocked("busy_recording_corpus", "idle");
        }
        if state.benchmark.is_running() {
            return blocked("busy_benchmarking", "idle");
        }

        match status {
            DictationStatus::Starting => blocked("already_starting", "starting"),
            DictationStatus::Recording => blocked("already_recording", "recording"),
            DictationStatus::Recovering => blocked("audio_recovering", "recovering"),
            DictationStatus::Processing => blocked("already_processing", "processing"),
            DictationStatus::Idle => None,
        }
    }

    fn legacy_dictation_refusal(
        state: &State,
        honor_committed_relaunch: bool,
    ) -> Option<&'static str> {
        let dictation = state.app_state.dictation.lock_or_recover();
        let blocker = if honor_committed_relaunch {
            legacy_dictation_start_blocker(state, dictation.status)
        } else {
            legacy_dictation_activity_blocker(state, dictation.status)
        };
        if blocker.is_some() {
            Some("dictation_busy")
        } else if state.app_state.microphone_preview.is_active() {
            Some("preview_active")
        } else if audio_lifecycle::is_audio_active() {
            Some("microphone_busy")
        } else if audio_lifecycle::is_dictation_cleanup_pending(
            state.app_state.recording_id.load(Ordering::SeqCst),
        ) {
            Some("cleanup_pending")
        } else {
            None
        }
    }

    // -- dictation start -----------------------------------------------------

    #[test]
    fn start_blockers_keep_their_reply_types_and_states() {
        let state = State::for_test();
        assert_eq!(dictation_start_blocker(&state, DictationStatus::Idle), None);

        for (status, kind, reply_state) in [
            (DictationStatus::Starting, "already_starting", "starting"),
            (DictationStatus::Recording, "already_recording", "recording"),
            (
                DictationStatus::Recovering,
                "audio_recovering",
                "recovering",
            ),
            (
                DictationStatus::Processing,
                "already_processing",
                "processing",
            ),
        ] {
            assert_eq!(
                dictation_start_blocker(&state, status),
                blocker(kind, reply_state),
                "{status:?}"
            );
        }

        state.app_state.meeting_active.store(true, Ordering::SeqCst);
        assert_eq!(
            dictation_start_blocker(&state, DictationStatus::Recording),
            blocker("busy_meeting", "idle"),
            "meeting outranks status"
        );
        state
            .app_state
            .meeting_active
            .store(false, Ordering::SeqCst);
        state
            .app_state
            .meeting_inference_active
            .store(true, Ordering::SeqCst);
        assert_eq!(
            dictation_start_blocker(&state, DictationStatus::Idle),
            blocker("busy_meeting", "idle"),
            "meeting inference recovery blocks too"
        );
        state
            .app_state
            .meeting_inference_active
            .store(false, Ordering::SeqCst);

        state
            .app_state
            .file_transcribing
            .store(true, Ordering::SeqCst);
        assert_eq!(
            dictation_start_blocker(&state, DictationStatus::Idle),
            blocker("busy_transcribing_file", "idle")
        );
        state
            .app_state
            .file_transcribing
            .store(false, Ordering::SeqCst);

        assert!(state.benchmark.try_start());
        assert_eq!(
            dictation_start_blocker(&state, DictationStatus::Idle),
            blocker("busy_benchmarking", "idle")
        );
        state.benchmark.finish();

        {
            let _committed = CommittedRelaunchForTest::raise();
            state.app_state.meeting_active.store(true, Ordering::SeqCst);
            assert_eq!(
                dictation_start_blocker(&state, DictationStatus::Idle),
                blocker("busy_restarting", "idle"),
                "a committed relaunch outranks every other blocker"
            );
            state
                .app_state
                .meeting_active
                .store(false, Ordering::SeqCst);
        }
        // A shared backend change is not a start blocker (matrix cell A).
        assert!(state.benchmark.try_start_shared_backend_change());
        assert_eq!(dictation_start_blocker(&state, DictationStatus::Idle), None);
        state.benchmark.finish_shared_backend_change();
    }

    #[test]
    fn start_pre_cleanup_and_smart_auto_steps_keep_their_replies() {
        // app_disabled first, then Smart Auto with a fixed device.
        assert_eq!(
            dictation_start_pre_admission(true, true),
            Admission::RefuseWith(RefusalCode::AppDisabled)
        );
        let Admission::RefuseWith(code) = dictation_start_pre_admission(true, false) else {
            panic!("app_disabled refuses");
        };
        assert_eq!(
            code.reply()
                .map(|(kind, state)| (kind.as_str(), state.as_str())),
            Some(("app_disabled", "idle"))
        );
        let Admission::RefuseWith(code) = dictation_start_pre_admission(false, true) else {
            panic!("a fixed device with Smart Auto refuses");
        };
        assert_eq!(
            code.text(),
            "Smart Auto cannot be combined with a fixed microphone."
        );
        assert!(matches!(
            dictation_start_pre_admission(false, false),
            Admission::Admit(_)
        ));

        // The retiring capture: `audio_recovering` and the start publishes Recovering.
        assert!(matches!(
            dictation_start_cleanup_admission(false),
            Admission::Admit(_)
        ));
        let Admission::RefuseWith(code) = dictation_start_cleanup_admission(true) else {
            panic!("a retiring capture refuses");
        };
        assert!(code.publishes_recovering());
        assert_eq!(
            code.reply()
                .map(|(kind, state)| (kind.as_str(), state.as_str())),
            Some(("audio_recovering", "recovering"))
        );

        // Smart Auto: observation first, then the resolve's own text, verbatim.
        assert!(matches!(
            dictation_start_smart_auto_admission(SmartAutoObservation::Requested(None)),
            Admission::NeedObservation(_)
        ));
        assert!(matches!(
            dictation_start_smart_auto_admission(SmartAutoObservation::Requested(Some(Ok(())))),
            Admission::Admit(_)
        ));
        let failure = "No suitable microphone: the probe timed out".to_string();
        assert_eq!(
            refusal_text(dictation_start_smart_auto_admission(
                SmartAutoObservation::Requested(Some(Err(failure.clone())))
            )),
            failure
        );
    }

    // -- prepare / key-down / relaunch gate ------------------------------------

    /// Every fact the old gates read, varied together: relaunch, meeting,
    /// meeting inference, file transcription, corpus (feature), benchmark,
    /// dictation status, preview (none / connecting / active) and the audio
    /// supervisor's public state (idle / owner-less capture / this recording
    /// retiring / another dictation retiring). The oracles are the literal old
    /// functions. Returns `(combinations, comparisons)`.
    fn compare_gates_with_the_old_predicate() -> (usize, usize) {
        let state = State::for_test();
        let preview = &state.app_state.microphone_preview;
        let recording_id = state.app_state.recording_id.load(Ordering::SeqCst);
        #[cfg(feature = "internal-benchmark")]
        let corpus_states = [false, true];
        #[cfg(not(feature = "internal-benchmark"))]
        let corpus_states = [false];
        let (mut combinations, mut comparisons) = (0, 0);
        for relaunch in [false, true] {
            let committed = relaunch.then(CommittedRelaunchForTest::raise);
            for meeting in [false, true] {
                for inference in [false, true] {
                    for file in [false, true] {
                        for corpus in corpus_states {
                            for benchmark in [false, true] {
                                for status in [
                                    DictationStatus::Idle,
                                    DictationStatus::Starting,
                                    DictationStatus::Recording,
                                    DictationStatus::Recovering,
                                    DictationStatus::Processing,
                                ] {
                                    // none / connecting / active (a handoff candidate)
                                    for preview_kind in 0..3 {
                                        for audio in [
                                            PublicForTest::Idle,
                                            PublicForTest::Active,
                                            PublicForTest::CleanupPending(recording_id),
                                            PublicForTest::CleanupPending(recording_id + 1),
                                        ] {
                                            state
                                                .app_state
                                                .meeting_active
                                                .store(meeting, Ordering::SeqCst);
                                            state
                                                .app_state
                                                .meeting_inference_active
                                                .store(inference, Ordering::SeqCst);
                                            state
                                                .app_state
                                                .file_transcribing
                                                .store(file, Ordering::SeqCst);
                                            #[cfg(feature = "internal-benchmark")]
                                            state.corpus.set_active_for_test(corpus);
                                            #[cfg(not(feature = "internal-benchmark"))]
                                            let _ = corpus;
                                            if benchmark {
                                                assert!(state.benchmark.try_start());
                                            }
                                            set_status(&state, status);
                                            let id = (preview_kind > 0)
                                                .then(|| preview.claim(50).unwrap());
                                            if let (2, Some(id)) = (preview_kind, id) {
                                                preview.set_phase_if(id, PreviewPhase::Active);
                                            }
                                            let _audio = set_public_for_test(audio);

                                            let facts = format!(
                                                "relaunch={relaunch} meeting={meeting} \
                                                 inference={inference} file={file} \
                                                 corpus={corpus} benchmark={benchmark} \
                                                 {status:?} preview={preview_kind} \
                                                 audio={audio:?}"
                                            );
                                            assert_eq!(
                                                dictation_prepare_refusal(&state),
                                                legacy_dictation_refusal(&state, true),
                                                "prepare: {facts}"
                                            );
                                            assert_eq!(
                                                dictation_relaunch_busy(&state),
                                                legacy_dictation_refusal(&state, false),
                                                "relaunch gate: {facts}"
                                            );
                                            let legacy_key_down =
                                                match legacy_dictation_refusal(&state, true) {
                                                    None => Ok(None),
                                                    Some("preview_active") => preview
                                                        .handoff_candidate()
                                                        .map(Some)
                                                        .ok_or("preview_active"),
                                                    Some(reason) => Err(reason),
                                                };
                                            assert_eq!(
                                                key_down_capture_admission(&state),
                                                legacy_key_down,
                                                "key-down: {facts}"
                                            );
                                            // The start path's blocker (the D read of
                                            // `start_native_recording_inner`).
                                            assert_eq!(
                                                dictation_start_blocker(&state, status),
                                                legacy_dictation_start_blocker(&state, status),
                                                "start blocker: {facts}"
                                            );
                                            combinations += 1;
                                            comparisons += 4;

                                            drop(_audio);
                                            if let Some(id) = id {
                                                preview.clear_if(id);
                                            }
                                            if benchmark {
                                                state.benchmark.finish();
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            drop(committed);
        }
        state
            .app_state
            .meeting_active
            .store(false, Ordering::SeqCst);
        state
            .app_state
            .meeting_inference_active
            .store(false, Ordering::SeqCst);
        state
            .app_state
            .file_transcribing
            .store(false, Ordering::SeqCst);
        #[cfg(feature = "internal-benchmark")]
        state.corpus.set_active_for_test(false);
        set_status(&state, DictationStatus::Idle);
        (combinations, comparisons)
    }

    #[test]
    fn gates_match_the_old_predicate_over_the_whole_fact_space() {
        let (combinations, comparisons) = compare_gates_with_the_old_predicate();
        let corpus_states = if cfg!(feature = "internal-benchmark") {
            2
        } else {
            1
        };
        // relaunch, meeting, inference, file, corpus, benchmark, status,
        // preview, audio
        assert_eq!(combinations, 2 * 2 * 2 * 2 * corpus_states * 2 * 5 * 3 * 4);
        assert_eq!(comparisons, 4 * combinations);
    }

    // -- no silent fallbacks -------------------------------------------------

    /// Every combination of the boolean facts the shell's readers can set (plus
    /// every status and a handoff candidate), as occupancies.
    fn fact_space() -> Vec<Occupancy> {
        const BITS: u32 = 18;
        let statuses = [
            Status::Idle,
            Status::Starting,
            Status::Recording,
            Status::Recovering,
            Status::Processing,
        ];
        let mut out = Vec::new();
        // Every combination of up to four facts, plus every seventh of the
        // rest: each refusal and each pair of facts is reached.
        for bits in 0..(1u32 << BITS) {
            if bits.count_ones() > 4 && bits % 7 != 0 {
                continue;
            }
            let on = |i: u32| bits & (1 << i) != 0;
            for status in statuses {
                out.push(Occupancy {
                    status,
                    relaunch_committed: on(0),
                    meeting_active: on(1),
                    meeting_inference_active: on(2),
                    file_transcribing: on(3),
                    benchmark_running: on(4),
                    corpus_active: on(5),
                    preview_active: on(6),
                    handoff_candidate: on(7).then_some(1),
                    audio_active: on(8),
                    cleanup_pending: on(9),
                    app_disabled: on(10),
                    shared_backend_change: on(11),
                    standby: on(12),
                    unconfirmed_key_down: on(13),
                    mic_startup_benchmark_active: on(14),
                    keyboard_processing: on(15),
                    key_down: KeyDownShellFacts {
                        listener_matches: on(16),
                        gesture_active: on(17),
                        microphone_granted: on(0) ^ on(17),
                    },
                    ..Occupancy::default()
                });
            }
        }
        out
    }

    #[test]
    fn every_resume_point_is_a_wait_of_its_request() {
        for point in Resume::ALL {
            let (request, retry) = point.point();
            assert!(
                admit_after(request, retry, &Occupancy::default()).is_some(),
                "{point:?}: {request:?} has no {retry:?} wait"
            );
            // And the wait is the one `admit` returns for the fact the shell
            // retires: the pairing is not just accepted, it is reachable.
            let waiting = match retry {
                Retry::RetireStandby => Occupancy {
                    standby: true,
                    ..Occupancy::default()
                },
                Retry::RetireKeyDownCapture => Occupancy {
                    unconfirmed_key_down: true,
                    ..Occupancy::default()
                },
            };
            let first = if matches!(point, Resume::PreviewAfterKeyDownRetire) {
                // `admit` reaches the key-down wait after the standby wait.
                admit_after(request, Retry::RetireStandby, &waiting)
                    .expect("PreviewStart waits for standby")
                    .admission
            } else {
                admit(request, &waiting)
            };
            assert_eq!(first, Admission::WaitThen(retry), "{point:?}");
        }
    }

    /// The callers that turn a refusal into a `&'static str` (`tag_of`,
    /// `key_down_capture_admission`, `meeting_post_claim`) can receive a
    /// refusal carrying shell text (`SmartAutoFailed`, `Text`) from no fact
    /// combination: `static_text` has no fallback to hide one.
    #[test]
    fn static_callers_never_see_a_payload_refusal() {
        let requests = [
            Request::DictationPrepare,
            Request::KeyDownCapture,
            Request::RelaunchGate,
            Request::MeetingPreClaim,
            Request::MeetingPostClaim,
        ];
        let mut refusals = std::collections::BTreeSet::new();
        for occupancy in fact_space() {
            for request in requests {
                match admit(request, &occupancy) {
                    Admission::RefuseWith(code) => {
                        assert!(
                            code.static_text().is_ok(),
                            "{request:?} refused with {code:?}, which has no static literal"
                        );
                        assert_eq!(static_text(&code), code.text());
                        refusals.insert(format!("{request:?}/{code:?}"));
                    }
                    // `tag_of` callers: gates never wait or observe; key-down
                    // takes over a preview.
                    Admission::WaitThen(_) | Admission::NeedObservation(_) => {
                        panic!(
                            "{request:?} returned {:?} for {occupancy:?}",
                            admit(request, &occupancy)
                        )
                    }
                    Admission::Takeover(_) => assert_eq!(request, Request::KeyDownCapture),
                    Admission::Admit(_) => {}
                }
            }
        }
        for relaunch in [false, true] {
            if let Admission::RefuseWith(code) = admit(
                Request::MeetingPostClaim,
                &Occupancy {
                    relaunch_recheck: Some(relaunch),
                    ..Occupancy::default()
                },
            ) {
                assert!(code.static_text().is_ok(), "{code:?}");
            }
        }
        assert!(
            refusals.len() > 10,
            "the fact space exercised too few refusals: {refusals:?}"
        );
    }

    /// The callers that return the refusal's text (`text_if_refused`,
    /// `refusal_text`) get only `Admit` or a refusal from the request and the
    /// resume point they use, so their unreachable arms stay unreachable.
    #[test]
    fn text_callers_only_see_admit_or_a_refusal() {
        let evaluate = |request: Request, resume: Option<Retry>, occupancy: &Occupancy| match resume
        {
            None => admit(request, occupancy),
            Some(retry) => {
                admit_after(request, retry, occupancy)
                    .expect("resume point")
                    .admission
            }
        };
        let mut calls: Vec<(Request, Option<Retry>)> = vec![
            (Request::FileTranscriptionPostClaim, None),
            (Request::Benchmark, None),
            (Request::ModelRemoval, None),
            (Request::Corpus, None),
        ];
        calls.extend(
            Resume::ALL
                .iter()
                .map(|point| (point.point().0, Some(point.point().1))),
        );
        for occupancy in fact_space() {
            for (request, resume) in &calls {
                // `unconfirmed_key_down` is read only by `preview_audio_admission`,
                // which handles its `WaitThen` itself; no `text_if_refused`
                // caller of the preview start sets it.
                let occupancy = Occupancy {
                    unconfirmed_key_down: false,
                    ..occupancy.clone()
                };
                let admission = evaluate(*request, *resume, &occupancy);
                assert!(
                    matches!(admission, Admission::Admit(_) | Admission::RefuseWith(_)),
                    "{request:?} after {resume:?} returned {admission:?} for {occupancy:?}"
                );
            }
        }
        // The failed-claim mappings (`refusal_text` callers) always refuse.
        for request in [
            Request::MicStartupBenchmarkPreClaim,
            Request::Benchmark,
            Request::SharedBackendChange,
            Request::ModelRemoval,
        ] {
            for benchmark_running in [false, true] {
                let admission = admit(
                    request,
                    &Occupancy {
                        coordinator_idle: Some(false),
                        benchmark_running,
                        ..Occupancy::default()
                    },
                );
                assert!(
                    matches!(admission, Admission::RefuseWith(_)),
                    "{request:?} benchmark_running={benchmark_running}: {admission:?}"
                );
            }
        }
    }

    #[test]
    fn gate_tags_keep_their_literals() {
        let state = State::for_test();
        assert_eq!(dictation_prepare_refusal(&state), None);
        set_status(&state, DictationStatus::Starting);
        assert_eq!(dictation_prepare_refusal(&state), Some("dictation_busy"));
        assert_eq!(dictation_relaunch_busy(&state), Some("dictation_busy"));
        assert_eq!(key_down_capture_admission(&state), Err("dictation_busy"));
        set_status(&state, DictationStatus::Idle);
        let id = state.app_state.microphone_preview.claim(50).unwrap();
        assert_eq!(dictation_prepare_refusal(&state), Some("preview_active"));
        assert_eq!(dictation_relaunch_busy(&state), Some("preview_active"));
        assert_eq!(key_down_capture_admission(&state), Err("preview_active"));
        state.app_state.microphone_preview.clear_if(id);
    }

    #[test]
    fn fn_key_down_shell_pre_checks_keep_their_tags_and_order() {
        assert_eq!(
            key_down_shell_refusal(true, false, false, true, "granted"),
            None
        );
        assert_eq!(
            key_down_shell_refusal(false, true, true, false, "denied"),
            Some("listener_changed")
        );
        assert_eq!(
            key_down_shell_refusal(true, true, true, false, "denied"),
            Some("processing_skipped")
        );
        assert_eq!(
            key_down_shell_refusal(true, false, true, false, "denied"),
            Some("disabled")
        );
        assert_eq!(
            key_down_shell_refusal(true, false, false, false, "denied"),
            Some("gesture_ended")
        );
        for permission in ["not_determined", "denied", "restricted", ""] {
            assert_eq!(
                key_down_shell_refusal(true, false, false, true, permission),
                Some("permission_not_granted"),
                "{permission:?}"
            );
        }
    }

    // -- file transcription ------------------------------------------------------

    #[test]
    fn file_claim_refuses_with_the_old_messages_and_swaps_only_when_admitted() {
        let state = State::for_test();
        let app_state = &state.app_state;

        // The preview stop's error comes back verbatim and nothing is claimed.
        assert_eq!(
            file_transcription_pre_claim::<()>(app_state, Err("Could not stop the test".into())),
            Err("Could not stop the test".to_string())
        );
        assert!(!app_state.file_transcribing.load(Ordering::SeqCst));

        // A meeting refuses before the swap: the flag stays false.
        app_state.meeting_active.store(true, Ordering::SeqCst);
        assert_eq!(
            file_transcription_pre_claim(app_state, Ok(())),
            Err(
                "Wait for the meeting transcript to finish before transcribing a file.".to_string()
            )
        );
        assert!(!app_state.file_transcribing.load(Ordering::SeqCst));
        app_state.meeting_active.store(false, Ordering::SeqCst);
        app_state
            .meeting_inference_active
            .store(true, Ordering::SeqCst);
        assert_eq!(
            file_transcription_pre_claim(app_state, Ok(())),
            Err(
                "Wait for the meeting transcript to finish before transcribing a file.".to_string()
            )
        );
        app_state
            .meeting_inference_active
            .store(false, Ordering::SeqCst);

        // Admitted: the swap happened, the transition value comes back.
        assert_eq!(file_transcription_pre_claim(app_state, Ok(7)), Ok(7));
        assert!(app_state.file_transcribing.load(Ordering::SeqCst));
        // The swap's prior value refuses a second claim; the flag stays true
        // (it belongs to the first claimant).
        assert_eq!(
            file_transcription_pre_claim(app_state, Ok(())),
            Err("Already transcribing a file.".to_string())
        );
        assert!(app_state.file_transcribing.load(Ordering::SeqCst));
        app_state.file_transcribing.store(false, Ordering::SeqCst);
    }

    #[test]
    fn file_post_claim_refuses_with_the_old_messages_in_order() {
        let state = State::for_test();
        assert_eq!(
            file_transcription_post_claim(&state, DictationStatus::Idle),
            Ok(())
        );
        let live = "Can't transcribe a file while recording or processing live audio. \
                    Stop the current recording first.";
        for status in [
            DictationStatus::Starting,
            DictationStatus::Recording,
            DictationStatus::Recovering,
            DictationStatus::Processing,
        ] {
            assert_eq!(
                file_transcription_post_claim(&state, status),
                Err(live.to_string()),
                "{status:?}"
            );
        }
        assert!(state.benchmark.try_start());
        assert_eq!(
            file_transcription_post_claim(&state, DictationStatus::Recording),
            Err("Wait for the benchmark to finish before transcribing a file.".to_string()),
            "benchmark outranks status"
        );
        state.benchmark.finish();
    }

    // -- meeting -------------------------------------------------------------------

    #[test]
    fn meeting_pre_claim_keeps_its_messages_and_order() {
        let state = State::for_test();
        let app_state = &state.app_state;
        assert_eq!(meeting_pre_claim(&state), None);

        set_status(&state, DictationStatus::Recording);
        assert_eq!(
            meeting_pre_claim(&state),
            Some("Wait for dictation to finish before starting a meeting.")
        );
        assert!(state.benchmark.try_start());
        assert_eq!(
            meeting_pre_claim(&state),
            Some("Wait for the benchmark to finish before starting a meeting.")
        );
        app_state.file_transcribing.store(true, Ordering::SeqCst);
        assert_eq!(
            meeting_pre_claim(&state),
            Some("Wait for file transcription to finish before starting a meeting.")
        );
        app_state
            .meeting_inference_active
            .store(true, Ordering::SeqCst);
        assert_eq!(
            meeting_pre_claim(&state),
            Some("Murmur is recovering an interrupted meeting transcript. Try again shortly.")
        );
        app_state.meeting_active.store(true, Ordering::SeqCst);
        assert_eq!(
            meeting_pre_claim(&state),
            Some("A meeting is already active.")
        );
        {
            let _committed = CommittedRelaunchForTest::raise();
            assert_eq!(
                meeting_pre_claim(&state),
                Some("Murmur is restarting to finish a permission reset. Try again in a moment.")
            );
        }
        app_state.meeting_active.store(false, Ordering::SeqCst);
        app_state
            .meeting_inference_active
            .store(false, Ordering::SeqCst);
        app_state.file_transcribing.store(false, Ordering::SeqCst);
        state.benchmark.finish();
        set_status(&state, DictationStatus::Idle);
    }

    #[test]
    fn meeting_post_claim_refuses_only_a_committed_relaunch() {
        assert_eq!(meeting_post_claim(false), Ok(()));
        assert_eq!(
            meeting_post_claim(true),
            Err("Murmur is restarting to finish a permission reset. Try again in a moment.")
        );
    }

    // -- microphone preview ---------------------------------------------------------

    #[test]
    fn preview_start_keeps_its_messages_and_order() {
        let state = State::for_test();
        let preview = &state.app_state.microphone_preview;

        // The frontend treats exactly these texts specially; `admit` carries them.
        assert_eq!(
            crate::commands::microphone_preview::PREVIEW_DICTATION_BUSY,
            RefusalCode::PreviewDictationBusy.text()
        );
        assert_eq!(
            crate::commands::microphone_preview::PREVIEW_AUDIO_BUSY,
            RefusalCode::MicrophoneStillInUse.text()
        );

        let generation = preview.start_generation();
        assert_eq!(preview_start_revoked_refusal(preview, generation), Ok(()));
        preview.revoke_pending_starts();
        assert_eq!(
            preview_start_revoked_refusal(preview, generation),
            Err("Finish the current dictation before testing the microphone".to_string())
        );

        assert_eq!(
            preview_start_admission(&state, DictationStatus::Idle),
            Ok(())
        );
        assert_eq!(
            preview_start_admission(&state, DictationStatus::Starting),
            Err("Finish the current dictation before testing the microphone".to_string())
        );
        state.app_state.meeting_active.store(true, Ordering::SeqCst);
        assert_eq!(
            preview_start_admission(&state, DictationStatus::Starting),
            Err("Finish the current dictation before testing the microphone".to_string()),
            "status outranks the meeting"
        );
        assert_eq!(
            preview_start_admission(&state, DictationStatus::Idle),
            Err("Finish the current meeting transcript before testing the microphone".to_string())
        );
        state
            .app_state
            .meeting_active
            .store(false, Ordering::SeqCst);
        assert!(state.benchmark.try_start());
        assert_eq!(
            preview_start_admission(&state, DictationStatus::Idle),
            Err("Finish the current benchmark before testing the microphone".to_string())
        );
        state.benchmark.finish();
        let id = preview.claim(50).unwrap();
        assert_eq!(
            preview_start_admission(&state, DictationStatus::Idle),
            Err("A microphone test is already active".to_string())
        );

        // The recheck after the claim: refuses once the preview is no longer connecting.
        assert!(preview_still_connecting(preview, id));
        assert!(!preview_still_connecting(preview, id + 1));
        preview.set_phase_if(id, PreviewPhase::Active);
        assert!(!preview_still_connecting(preview, id));
        preview.clear_if(id);
    }

    #[test]
    fn preview_audio_check_retires_a_key_down_capture_once_then_reads_audio() {
        use std::cell::Cell;
        let busy = "The microphone is still in use or recovering. Wait for Murmur to become ready.";

        // Nothing unconfirmed: no retire; audio decides.
        let retired = Cell::new(0);
        assert_eq!(
            preview_audio_admission(|| false, || retired.set(retired.get() + 1), || true),
            Some(busy)
        );
        assert_eq!(retired.get(), 0);
        assert_eq!(
            preview_audio_admission(|| false, || retired.set(retired.get() + 1), || false),
            None
        );

        // Unconfirmed: retire once, then `audio_active` is read once, fresh, and
        // decides whether or not the capture is gone.
        let audio_reads = Cell::new(0);
        let retire_then_clear = Cell::new(false);
        assert_eq!(
            preview_audio_admission(
                || true,
                || retire_then_clear.set(true),
                || {
                    audio_reads.set(audio_reads.get() + 1);
                    !retire_then_clear.get()
                }
            ),
            None
        );
        assert_eq!(audio_reads.get(), 1);
        // A retire that timed out falls through to the refusal, never a second wait.
        assert_eq!(preview_audio_admission(|| true, || {}, || true), Some(busy));
    }

    // -- microphone startup benchmark ------------------------------------------------

    #[test]
    fn microphone_benchmark_keeps_its_messages_and_order() {
        let state = State::for_test();
        assert_eq!(
            microphone_benchmark_pre_claim(&state, DictationStatus::Idle),
            Ok(())
        );
        state.app_state.meeting_active.store(true, Ordering::SeqCst);
        assert_eq!(
            microphone_benchmark_pre_claim(&state, DictationStatus::Recording),
            Err(
                "Wait for the meeting transcript to finish before testing the microphone"
                    .to_string()
            ),
            "meeting outranks status"
        );
        state
            .app_state
            .meeting_active
            .store(false, Ordering::SeqCst);
        assert_eq!(
            microphone_benchmark_pre_claim(&state, DictationStatus::Recording),
            Err("Stop recording before testing the microphone".to_string())
        );
        state
            .app_state
            .file_transcribing
            .store(true, Ordering::SeqCst);
        assert_eq!(
            microphone_benchmark_pre_claim(&state, DictationStatus::Idle),
            Err("Wait for the file transcription to finish".to_string())
        );
        state
            .app_state
            .file_transcribing
            .store(false, Ordering::SeqCst);

        // The coordinator's own answer to `try_start()`.
        assert!(state.benchmark.try_start());
        assert_eq!(
            microphone_benchmark_claim_failed(&state.benchmark),
            "A microphone startup benchmark is already running"
        );
        state.benchmark.finish();
        assert!(state.benchmark.try_start_shared_backend_change());
        assert_eq!(
            microphone_benchmark_claim_failed(&state.benchmark),
            "Wait for model preparation to finish before testing the microphone"
        );
        state.benchmark.finish_shared_backend_change();

        // The second claim: the claim's own error text, unchanged.
        assert_eq!(microphone_benchmark_second_claim(Ok(4)), Ok(4));
        assert_eq!(
            microphone_benchmark_second_claim(Err(
                "A microphone startup benchmark is already running".into()
            )),
            Err("A microphone startup benchmark is already running".to_string())
        );
    }

    // -- benchmark ---------------------------------------------------------------------

    #[test]
    fn benchmark_keeps_its_messages_and_order() {
        let state = State::for_test();
        assert_eq!(benchmark_meeting_check(&state), Ok(()));
        assert_eq!(benchmark_pre_claim(&state, DictationStatus::Idle), Ok(()));

        state
            .app_state
            .meeting_inference_active
            .store(true, Ordering::SeqCst);
        assert_eq!(
            benchmark_meeting_check(&state),
            Err("Wait for the meeting transcript to finish before benchmarking".to_string())
        );
        state
            .app_state
            .meeting_inference_active
            .store(false, Ordering::SeqCst);

        assert_eq!(
            benchmark_pre_claim(&state, DictationStatus::Processing),
            Err("Stop recording before running a benchmark".to_string())
        );
        state
            .app_state
            .file_transcribing
            .store(true, Ordering::SeqCst);
        assert_eq!(
            benchmark_pre_claim(&state, DictationStatus::Recording),
            Err("Stop recording before running a benchmark".to_string()),
            "status outranks the file"
        );
        assert_eq!(
            benchmark_pre_claim(&state, DictationStatus::Idle),
            Err("Wait for the file transcription to finish".to_string())
        );
        state
            .app_state
            .file_transcribing
            .store(false, Ordering::SeqCst);

        assert!(state.benchmark.try_start());
        assert_eq!(
            benchmark_claim_failed(&state.benchmark),
            "A benchmark is already running"
        );
        state.benchmark.finish();
        assert!(state.benchmark.try_start_shared_backend_change());
        assert_eq!(
            benchmark_claim_failed(&state.benchmark),
            "Wait for model preparation to finish before benchmarking"
        );
        state.benchmark.finish_shared_backend_change();
    }

    #[cfg(feature = "internal-benchmark")]
    #[test]
    fn corpus_keeps_its_messages_and_order() {
        let state = State::for_test();
        assert_eq!(corpus_pre_claim(&state, DictationStatus::Idle), Ok(()));
        state.app_state.meeting_active.store(true, Ordering::SeqCst);
        assert_eq!(
            corpus_pre_claim(&state, DictationStatus::Recording),
            Err("Finish the current meeting transcript first".to_string())
        );
        state
            .app_state
            .meeting_active
            .store(false, Ordering::SeqCst);
        assert_eq!(
            corpus_pre_claim(&state, DictationStatus::Recording),
            Err("Finish the current dictation before recording corpus audio".to_string())
        );
        state
            .app_state
            .file_transcribing
            .store(true, Ordering::SeqCst);
        assert_eq!(
            corpus_pre_claim(&state, DictationStatus::Idle),
            Err("Finish the current file transcription first".to_string())
        );
        state
            .app_state
            .file_transcribing
            .store(false, Ordering::SeqCst);
        assert!(state.benchmark.try_start());
        assert_eq!(
            corpus_pre_claim(&state, DictationStatus::Idle),
            Err("Finish the current benchmark first".to_string())
        );
        state.benchmark.finish();
        assert_eq!(
            corpus_claim_failed("A corpus recording is already active".into()),
            "A corpus recording is already active"
        );
    }

    // -- shared backend change, model removal, model preparation ---------------------------

    #[test]
    fn shared_backend_change_refusal_keeps_its_message() {
        assert_eq!(
            shared_backend_change_refusal(),
            "Wait for the benchmark or current model preparation to finish before changing models."
        );
    }

    #[test]
    fn model_removal_keeps_its_messages_and_order() {
        let state = State::for_test();
        let app_state = &state.app_state;

        // Both `try_lock`s: the attempt's outcome is the fact, the guard comes back.
        assert_eq!(model_removal_transition_lock::<u8, ()>(Ok(1)), Ok(1));
        assert_eq!(
            model_removal_transition_lock::<u8, ()>(Err(())),
            Err("Wait for the current recording transition to finish".to_string())
        );
        assert_eq!(model_removal_dictation_lock(Some(2)), Ok(2));
        assert_eq!(
            model_removal_dictation_lock::<u8>(None),
            Err("Wait for the current dictation operation to finish".to_string())
        );

        let occupied =
            "Stop recording and wait for transcription to finish before removing a model";
        assert_eq!(
            model_removal_occupancy_check(app_state, DictationStatus::Idle),
            Ok(())
        );
        for status in [
            DictationStatus::Starting,
            DictationStatus::Recording,
            DictationStatus::Recovering,
            DictationStatus::Processing,
        ] {
            assert_eq!(
                model_removal_occupancy_check(app_state, status),
                Err(occupied.to_string()),
                "{status:?}"
            );
        }
        app_state.file_transcribing.store(true, Ordering::SeqCst);
        assert_eq!(
            model_removal_occupancy_check(app_state, DictationStatus::Idle),
            Err(occupied.to_string())
        );
        app_state.file_transcribing.store(false, Ordering::SeqCst);
        app_state
            .meeting_inference_active
            .store(true, Ordering::SeqCst);
        assert_eq!(
            model_removal_occupancy_check(app_state, DictationStatus::Idle),
            Err(occupied.to_string())
        );
        app_state
            .meeting_inference_active
            .store(false, Ordering::SeqCst);

        assert_eq!(
            model_removal_claim_failed(),
            "Wait for the current model operation to finish before removing a model"
        );
    }

    #[test]
    fn model_preparation_matches_the_old_predicate_and_reads_the_model_last() {
        use std::cell::Cell;
        let statuses = [
            DictationStatus::Idle,
            DictationStatus::Starting,
            DictationStatus::Recording,
            DictationStatus::Recovering,
            DictationStatus::Processing,
        ];
        for status in statuses {
            for current in [4_u64, 5] {
                for model_matches in [false, true] {
                    let reads = Cell::new(0);
                    // Recording start: `recording_id == spawned id && status in
                    // {Starting, Recording} && model matches`, model read last.
                    let eligible = model_preparation_eligible(
                        ModelPrepVariant::RecordingStart,
                        status,
                        current,
                        Some(5),
                        || {
                            reads.set(reads.get() + 1);
                            model_matches
                        },
                    );
                    let legacy = current == 5
                        && matches!(
                            status,
                            DictationStatus::Starting | DictationStatus::Recording
                        )
                        && model_matches;
                    assert_eq!(
                        eligible, legacy,
                        "{status:?} current={current} model={model_matches}"
                    );
                    let model_read_expected = current == 5
                        && matches!(
                            status,
                            DictationStatus::Starting | DictationStatus::Recording
                        );
                    assert_eq!(
                        reads.get(),
                        usize::from(model_read_expected),
                        "{status:?} current={current}"
                    );

                    // Idle warm-up: no id guard; `status == Idle && model matches`.
                    let eligible = model_preparation_eligible(
                        ModelPrepVariant::IdleWarm,
                        status,
                        current,
                        None,
                        || model_matches,
                    );
                    assert_eq!(eligible, status == DictationStatus::Idle && model_matches);
                }
            }
        }
    }

    #[test]
    fn staged_reads_stop_at_the_first_refusal() {
        use std::cell::Cell;
        let later = Cell::new(false);
        let outcome = staged(Staged::new(Request::Benchmark), |s| {
            s.read(|o| o.meeting_active = true)?;
            s.read(|_| later.set(true))
        });
        assert_eq!(outcome, Admission::RefuseWith(RefusalCode::BenchMeeting));
        assert!(!later.get(), "no fact is read after a refusal");
    }
}
