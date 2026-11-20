//! Admission: `admit(request, occupancy)` (appendix C6).
//!
//! `admit` is a **pure predicate** over an [`Occupancy`] the shell gathered at
//! the point today's code reads those facts. It evaluates one ordered check list
//! per request (C6.2) and **returns at the first check that fires; later checks
//! are not evaluated** ([`admit_traced`] exposes exactly which were).
//!
//! # Per-resource claim protocols (C6.4)
//!
//! `admit` centralizes the *predicate*, never the *atomicity*. Each resource
//! keeps the claim, the lock it is taken under, the recheck points and the
//! release it has today. Nothing here claims, locks or releases anything; the
//! shell calls the per-phase variants (C6.1a) in the order the code does.
//! Admission is **not** atomic across resources and this crate does not claim it
//! is: for example `Meeting` does not re-read dictation status after its claim
//! (T keeps a dictation start out), and `FileTranscription` claims under T and
//! rechecks under D only after T is dropped.
//!
//! | Resource | Claim | Under which lock | Recheck | Release |
//! |---|---|---|---|---|
//! | Dictation | status `Idle→Starting` + id bump (R101.15) | D inside T | after audio start (R102 id guard), after smart-destination capture (`q_capture_current`), at context install (R103), at ready publish (R104) | R203/R204a/R204b/R205/R211/R216/R210 (status back to Idle); supervisor releases microphone ownership in `finish_attempt` after notifying Idle |
//! | File transcription | `file_transcribing.swap(true)` after the pre-admission preview stop | T, then T released, then D recheck | D: benchmark, corpus, status | `FileTranscribeGuard::drop` |
//! | Meeting | store `meeting_active`/`meeting_inference_active`, **then** re-read `relaunch_committed` (Dekker) | T for the whole `start_meeting`; D not held across the claim | relaunch only; dictation status is read before the claim and **not** re-read | on start failure; thread exit clears `meeting_active` only if `meeting_generation` still matches |
//! | Preview | `start_generation` before any wait, then `preview.claim(vad)`, then `capture_claimed(Preview)` | T for the claim; D only for the preceding reads | `is_connecting(id)` after re-taking T in the spawned task | `Idle`/`Recovering`/`InitializationFailed` bridge, `stop_exact_preview`, `clear_if`, `fail_and_clear` |
//! | Mic startup benchmark | `benchmark.try_start()` under D, then `microphone_startup_benchmark.claim(run_id)` under T only | D then T | the second claim can fail ⇒ `coordinator.finish()` | `coordinator.finish()` at end of run |
//! | Benchmark | `coordinator.try_start()` | D inside T | none | `BenchmarkRunGuard` drop |
//! | Corpus | `corpus.claim(request)` | D+T | none | `corpus.clear_if(id)` on failure; bridge `Idle` |
//! | Shared backend change (model preparation and model removal) | `try_start_shared_backend_change()` | the coordinator's own mutex only; model removal calls it while holding `try_lock`ed T and D | none | `SharedBackendChangeGuard::drop` / `ChangeGuard::drop` |
//! | Relaunch | `set_committed(true)` → `gate().open()` re-read (D via `dictation_refusal(false)`) → relaunch, else `set_committed(false)` | D inside the gate | the re-read **is** the recheck | `ReleaseUnlessRelaunched::drop` |
//! | Speculative capture | `speculative::claim(token)` before T, single-use, 1.5 s TTL | `BINDING` mutex | adopted at the supervisor | `SpeculativeClaim::drop` discards |
//!
//! Any change to a claim protocol (for example making `Meeting` re-read
//! dictation status after its claim, or giving `Meeting` a preview check) is a
//! separate, separately-approved fix.
//!
//! # Supervisor-enforced microphone exclusion
//!
//! Microphone exclusion is **not** an `admit` decision. The supervisor's
//! `handle_start` refuses any owner while an attempt exists
//! (`audio_lifecycle.rs:1425-1433`) and adopts a flowing speculative capture, a
//! matching Standby capture or a kept preview. `DictationStart` therefore never
//! reads `audio_active`, standby or `unconfirmed_key_down` (matrix cells `S`).

use crate::decision::ObsKind;
use crate::ids::{PreviewId, RecordingId};
use crate::replies::ReplyType;
use crate::status::Status;

/// What is asking to start (C6.1). Requests that read facts before and after a
/// claim are split into per-phase variants (C6.1a), each with its own
/// [`Occupancy`] gathered at the point today's code reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Request {
    /// Confirmed dictation start (`recording.rs` start under D+T).
    DictationStart,
    /// Silent first-tap helper preparation.
    DictationPrepare,
    /// Fn key-down speculative capture.
    KeyDownCapture,
    /// Permission relaunch asking "is anything busy?".
    RelaunchGate,
    /// File transcription before the claim: preview stop, meeting check, claim swap.
    FileTranscriptionPreClaim,
    /// File transcription after the claim, under D with T dropped.
    FileTranscriptionPostClaim,
    /// Meeting start: reads before the claim.
    MeetingPreClaim,
    /// Meeting start: the `relaunch_committed` re-read after the claim stores.
    MeetingPostClaim,
    /// Microphone preview start, **before** `preview.claim()` (C6.2 #7).
    PreviewStart,
    /// The `is_connecting(id)` recheck after the claim, in the spawned task
    /// (`microphone_preview.rs:270`). A separate phase: it repeats none of the
    /// `PreviewStart` checks (the claim made `preview_active` true) and claims
    /// nothing.
    PreviewStartRecheck,
    /// Microphone startup benchmark up to `benchmark.try_start()`.
    MicStartupBenchmarkPreClaim,
    /// Microphone startup benchmark: the second claim under T only.
    MicStartupBenchmarkPostClaim,
    /// Benchmark.
    Benchmark,
    /// Corpus recording (feature `internal-benchmark`).
    Corpus,
    /// Shared backend change (model preparation / selection).
    SharedBackendChange,
    /// Model preparation eligibility (`q_model_prep_eligible`).
    ModelPreparation,
    /// Model removal (`try_lock` T then D, occupancy checks, then the claim).
    ModelRemoval,
}

impl Request {
    /// Every request, in C6.1 order.
    pub const ALL: [Request; 17] = [
        Self::DictationStart,
        Self::DictationPrepare,
        Self::KeyDownCapture,
        Self::RelaunchGate,
        Self::FileTranscriptionPreClaim,
        Self::FileTranscriptionPostClaim,
        Self::MeetingPreClaim,
        Self::MeetingPostClaim,
        Self::PreviewStart,
        Self::PreviewStartRecheck,
        Self::MicStartupBenchmarkPreClaim,
        Self::MicStartupBenchmarkPostClaim,
        Self::Benchmark,
        Self::Corpus,
        Self::SharedBackendChange,
        Self::ModelPreparation,
        Self::ModelRemoval,
    ];
}

/// Smart Auto resolution state of a dictation start (C2.2 `SmartAutoSelection`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SmartAutoObservation {
    /// The start did not ask for Smart Auto.
    #[default]
    NotRequested,
    /// Smart Auto was requested; `None` until the shell resolves it, then
    /// `Some(Ok(()))` or `Some(Err(text))` (the resolution payload is not needed here).
    Requested(Option<Result<(), String>>),
}

/// Shell-owned facts the Fn key-down gate reads before the dictation guards
/// (`fn_capture.rs` `admission_refusal`). `Default` is "nothing refuses".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyDownShellFacts {
    /// The listener context still matches (`listener_changed` otherwise).
    pub listener_matches: bool,
    /// The gesture is still active (`gesture_ended` otherwise).
    pub gesture_active: bool,
    /// Microphone permission is already `granted` (`permission_not_granted`).
    pub microphone_granted: bool,
}

impl Default for KeyDownShellFacts {
    fn default() -> Self {
        Self {
            listener_matches: true,
            gesture_active: true,
            microphone_granted: true,
        }
    }
}

/// Which `ModelPreparation` eligibility is asked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ModelPrepVariant {
    /// Recording-start preparation: status `Starting` or `Recording` (1136-1158).
    #[default]
    RecordingStart,
    /// Idle warm-up: status `Idle` (1240, 1248).
    IdleWarm,
}

/// A plain value of facts the shell gathers where today's code reads them
/// (C6.1). `Default` is a quiet app: status Idle, nothing occupied.
///
/// Fields marked *(added)* are not in the C6.1 sketch; they are facts a C6.2
/// check reads that the sketch left out (see the README "Deviations").
///
/// Invariant at the call sites (not enforced here): a running microphone
/// startup check holds the coordinator's `Benchmark` activity, so
/// `mic_startup_benchmark_active ⇒ benchmark_running`; a retiring capture or a
/// standby / unconfirmed key-down capture has `audio_active` (`phase != Idle`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Occupancy {
    // dictation, core-derived
    /// Dictation status.
    pub status: Status,
    /// A dictation pipeline is running.
    pub dictation_pipeline_running: bool,
    /// A dictation pipeline is draining (allowed by every request).
    pub dictation_pipeline_draining: bool,
    /// Retiring capture of the CURRENT id (`audio_lifecycle.rs:3485-3492`).
    pub cleanup_pending: bool,
    // ASR users
    /// A meeting owns ASR.
    pub meeting_active: bool,
    /// Meeting transcript recovery owns ASR (`meeting_blocks_asr` = either).
    pub meeting_inference_active: bool,
    /// A file transcription holds the backend.
    pub file_transcribing: bool,
    /// `CoordinatorActivity::Benchmark` only.
    pub benchmark_running: bool,
    /// `is_busy() && !benchmark_running`.
    pub shared_backend_change: bool,
    /// Corpus recording (feature `internal-benchmark`, else `false`).
    pub corpus_active: bool,
    // microphone
    /// A microphone preview is active.
    pub preview_active: bool,
    /// The preview a start or key-down capture may take over.
    pub handoff_candidate: Option<u64>,
    /// `PublicState::is_active` = `phase != Idle`: any owner, incl. Standby.
    pub audio_active: bool,
    /// A Standby capture exists *(added: the C6.3 matrix has a standby column)*.
    pub standby: bool,
    /// Only an unconfirmed Fn key-down capture holds the microphone.
    pub unconfirmed_key_down: bool,
    /// A microphone startup check is running.
    pub mic_startup_benchmark_active: bool,
    // per-phase observations (C6.1a)
    /// `FileTranscriptionPreClaim`: outcome of the pre-admission preview stop.
    pub preview_stop: Option<Result<(), String>>,
    /// `FileTranscriptionPreClaim`: the value `file_transcribing.swap(true)` returned.
    /// Once the shell has swapped, `file_transcribing` is the shell's own claim:
    /// it must pass the prior value here (or not call this phase again). The
    /// `file_transcribing` fallback used while this is `None` is acceptable only
    /// as a **pre-claim view**: production wiring must preserve the atomic swap
    /// and its returned prior value (`recording.rs:6599-6605`); it cannot
    /// substitute a flag load for that result.
    pub file_claim_prior: Option<bool>,
    /// `MeetingPostClaim`: `relaunch_committed` re-read after the claim stores.
    pub relaunch_recheck: Option<bool>,
    /// `MicStartupBenchmarkPostClaim`: `Some(false)` if the second claim failed.
    pub second_claim: Option<bool>,
    /// `PreviewStartRecheck` (`microphone_preview.rs:270`): `is_connecting(id)`.
    /// `Some(false)` refuses (the task clears the preview and returns silently).
    pub preview_still_connecting: Option<bool>,
    /// The coordinator mutex's own answer for a claim (`Some(false)` = not Idle).
    pub coordinator_idle: Option<bool>,
    // process
    /// A permission relaunch has committed to exiting.
    pub relaunch_committed: bool,
    /// `keyboard::is_app_disabled()`.
    pub app_disabled: bool,
    /// `keyboard::IS_PROCESSING`.
    pub keyboard_processing: bool,
    // added facts
    /// `smart_auto && device_name` (5491) *(added)*.
    pub fixed_and_smart_auto: bool,
    /// Smart Auto resolution state *(added; C2.2)*.
    pub smart_auto: SmartAutoObservation,
    /// Fn key-down shell facts *(added; C6.2 #3 "shell pre-checks")*.
    pub key_down: KeyDownShellFacts,
    /// `refuse_revoked_start` would refuse the preview start *(added)*.
    pub preview_start_revoked: bool,
    /// `recording_transition.try_lock()` failed *(added; ModelRemoval)*.
    pub transition_lock_busy: bool,
    /// `dictation.try_lock()` failed *(added; ModelRemoval)*.
    pub dictation_lock_busy: bool,
    /// The frozen context's model equals the one being prepared *(added)*.
    pub model_matches: bool,
    /// The current recording id (`AppState.recording_id`) *(added)*.
    pub recording_id: RecordingId,
    /// The recording id the preparation worker was spawned for *(added)*.
    /// `RecordingStart` preparation is only eligible while it equals
    /// `recording_id` (`recording.rs:1153`, `:1168`); `None` never matches.
    pub expected_recording_id: Option<RecordingId>,
    /// Which `ModelPreparation` eligibility is asked *(added)*.
    pub model_prep_variant: ModelPrepVariant,
}

impl Occupancy {
    /// `meeting_blocks_asr` (`state.rs:431-434`): a meeting or its inference recovery.
    pub fn meeting_blocks_asr(&self) -> bool {
        self.meeting_active || self.meeting_inference_active
    }
}

/// What the shell should do when `admit` neither admits nor refuses outright.
///
/// **A retry resumes; it never restarts.** After the shell has done the retire
/// it calls [`admit_after`] with this value and a fresh `Occupancy`. Evaluation
/// continues at the check **after** the wait; the checks before it are not run
/// again (production already passed them, and their facts may have changed
/// since: Settings closing during a retire revokes the start, but the already
/// passed revocation check is not re-read, `microphone_preview.rs:137-146`).
/// Because a resume only moves forward, a wait is returned at most once per
/// request, even when the retire timed out and the thing is still there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Retry {
    /// `retire_standby()`, then [`admit_after`] resumes at the check after
    /// `standby`.
    RetireStandby,
    /// Bounded wait / retire of an unconfirmed key-down capture
    /// (`preview_audio_blocker`), then [`admit_after`] resumes at the check
    /// after `unconfirmed_key_down` (`audio_active`), whether or not the
    /// capture is gone, including after a timeout.
    RetireKeyDownCapture,
}

impl Retry {
    /// The name of the check that returned this wait.
    const fn check(self) -> &'static str {
        match self {
            Self::RetireStandby => "standby",
            Self::RetireKeyDownCapture => "unconfirmed_key_down",
        }
    }
}

/// The resource claim the shell performs next, under today's lock (C6.4).
/// Informational: `admit` never claims.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClaimKind {
    /// No claim at this phase.
    None,
    /// Dictation: `Idle→Starting` + id bump.
    DictationTransition,
    /// `file_transcribing.swap(true)`.
    FileTranscription,
    /// `claim_meeting_ownership`.
    MeetingOwnership,
    /// `preview.claim(vad)`.
    PreviewGeneration,
    /// `coordinator.try_start()`.
    BenchmarkCoordinator,
    /// `corpus.claim(request)`.
    Corpus,
    /// `try_start_shared_backend_change()`.
    SharedBackendChange,
}

/// What happens to a visible preview when a request is admitted (the `K` cells).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewDisposition {
    /// No preview involved.
    NotPresent,
    /// The preview is kept and handed to the dictation (`hand_preview_to`).
    KeptForHandoff(PreviewId),
    /// A visible preview that is not the handoff candidate is stopped and the
    /// start retries (`transition_for_dictation_start`).
    StopThenRetry,
    /// The shell's pre-admission step stopped the preview (file transcription).
    StoppedFirst,
}

/// What an admitted request carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Claims {
    /// The claim to perform next.
    pub claim: ClaimKind,
    /// Preview disposition.
    pub preview: PreviewDisposition,
}

impl Claims {
    const fn of(claim: ClaimKind) -> Self {
        Self {
            claim,
            preview: PreviewDisposition::NotPresent,
        }
    }
}

/// The result of [`admit`] (C6.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Admission {
    /// Admitted.
    Admit(Claims),
    /// The request takes over a preview in place (Fn key-down with a handoff candidate).
    Takeover(PreviewId),
    /// Refused with today's literal.
    RefuseWith(RefusalCode),
    /// Retire something and ask again.
    WaitThen(Retry),
    /// The start needs `SmartAutoSelection` first (C2.2). *(added variant: C6.2
    /// #1 routes the smart-auto resolve "through `NeedObservation`")*
    NeedObservation(ObsKind),
}

/// Today's literal for each refusal, so no caller-visible message changes.
///
/// [`RefusalCode::text`] is the exact string; [`RefusalCode::reply`] maps the
/// dictation-start refusals to their `{"type","state"}` reply.
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum RefusalCode {
    // dictation start: JSON `{"type":..,"state":..}` (5483-5538)
    AppDisabled,
    BusyRestarting,
    BusyMeeting,
    BusyTranscribingFile,
    BusyRecordingCorpus,
    BusyBenchmarking,
    AlreadyStarting,
    AlreadyRecording,
    AudioRecovering,
    AlreadyProcessing,
    /// R101.12: `audio_recovering`; a pure refusal (the status stays `Idle`).
    AudioRecoveringCleanupPending,
    /// `Err("Smart Auto cannot be combined with a fixed microphone.")` (5491).
    SmartAutoWithFixedDevice,
    /// `Err(e)` from `resolve_smart_auto`, verbatim.
    SmartAutoFailed(String),
    // prepare / key-down / relaunch tags (`dictation_refusal`)
    DictationBusy,
    PreviewActive,
    MicrophoneBusy,
    CleanupPending,
    // Fn key-down shell tags (`fn_capture.rs` `admission_refusal`)
    KeyDownListenerChanged,
    KeyDownProcessingSkipped,
    KeyDownDisabled,
    KeyDownGestureEnded,
    KeyDownPermissionNotGranted,
    // shell-supplied text (the pre-admission preview stop error)
    Text(String),
    // shared messages
    MicrophoneStillInUse,
    WaitForFileTranscription,
    EnableMurmurBeforeMicTest,
    // file transcription (`recording.rs` transcribe_file)
    FileWaitForMeeting,
    FileAlreadyTranscribing,
    FileWaitForBenchmark,
    FileFinishCorpus,
    FileLiveAudioActive,
    // meeting (`meeting.rs`)
    MeetingRestarting,
    MeetingMicStartupCheck,
    MeetingFinishCorpus,
    MeetingAlreadyActive,
    MeetingRecovering,
    MeetingStopMicrophone,
    MeetingWaitForFile,
    MeetingWaitForBenchmark,
    MeetingWaitForDictation,
    // preview (`microphone_preview.rs`)
    PreviewDictationBusy,
    PreviewMeeting,
    PreviewBenchmark,
    PreviewCorpus,
    PreviewAlreadyActive,
    PreviewNoLongerConnecting,
    // mic startup benchmark (`microphone_startup_benchmark.rs`)
    MicBenchMeeting,
    MicBenchStopRecording,
    MicBenchCorpus,
    MicBenchAlreadyRunning,
    MicBenchWaitForModelPrep,
    // benchmark (`benchmark.rs`)
    BenchMeeting,
    BenchStopRecording,
    BenchCorpus,
    BenchAlreadyRunning,
    BenchWaitForModelPrep,
    // corpus (`corpus.rs`)
    CorpusMeeting,
    CorpusDictation,
    CorpusFile,
    CorpusBenchmark,
    CorpusAlreadyActive,
    // shared backend change / model removal
    SharedBackendBusy,
    ModelRemovalTransitionBusy,
    ModelRemovalDictationBusy,
    ModelRemovalOccupied,
    ModelRemovalOperationBusy,
    /// `q_model_prep_eligible` is false: the preparation task returns silently.
    ModelPreparationNotEligible,
}

impl RefusalCode {
    /// The literal string/tag/`type` of today's refusal as a `&'static str`,
    /// or the shell-supplied payload for the two variants that carry text
    /// ([`RefusalCode::SmartAutoFailed`], [`RefusalCode::Text`]). The match is
    /// exhaustive: a new variant does not compile until it has its text. For
    /// [`RefusalCode::ModelPreparationNotEligible`] (a silent return today) the
    /// literal is a stable diagnostic tag.
    pub fn static_text(&self) -> Result<&'static str, &str> {
        Ok(match self {
            Self::AppDisabled => "app_disabled",
            Self::BusyRestarting => "busy_restarting",
            Self::BusyMeeting => "busy_meeting",
            Self::BusyTranscribingFile => "busy_transcribing_file",
            Self::BusyRecordingCorpus => "busy_recording_corpus",
            Self::BusyBenchmarking => "busy_benchmarking",
            Self::AlreadyStarting => "already_starting",
            Self::AlreadyRecording => "already_recording",
            Self::AudioRecovering | Self::AudioRecoveringCleanupPending => "audio_recovering",
            Self::AlreadyProcessing => "already_processing",
            Self::SmartAutoWithFixedDevice => {
                "Smart Auto cannot be combined with a fixed microphone."
            }
            Self::SmartAutoFailed(text) | Self::Text(text) => return Err(text),
            Self::DictationBusy => "dictation_busy",
            Self::PreviewActive => "preview_active",
            Self::MicrophoneBusy => "microphone_busy",
            Self::CleanupPending => "cleanup_pending",
            Self::KeyDownListenerChanged => "listener_changed",
            Self::KeyDownProcessingSkipped => "processing_skipped",
            Self::KeyDownDisabled => "disabled",
            Self::KeyDownGestureEnded => "gesture_ended",
            Self::KeyDownPermissionNotGranted => "permission_not_granted",
            Self::MicrophoneStillInUse => {
                "The microphone is still in use or recovering. Wait for Murmur to become ready."
            }
            Self::WaitForFileTranscription => "Wait for the file transcription to finish",
            Self::EnableMurmurBeforeMicTest => "Enable Murmur before testing the microphone",
            Self::FileWaitForMeeting => {
                "Wait for the meeting transcript to finish before transcribing a file."
            }
            Self::FileAlreadyTranscribing => "Already transcribing a file.",
            Self::FileWaitForBenchmark => {
                "Wait for the benchmark to finish before transcribing a file."
            }
            Self::FileFinishCorpus => "Finish the corpus recording before transcribing a file.",
            Self::FileLiveAudioActive => {
                "Can't transcribe a file while recording or processing live audio. \
                 Stop the current recording first."
            }
            Self::MeetingRestarting => {
                "Murmur is restarting to finish a permission reset. Try again in a moment."
            }
            Self::MeetingMicStartupCheck => {
                "Wait for the microphone startup check to finish before starting a meeting."
            }
            Self::MeetingFinishCorpus => "Finish the corpus recording before starting a meeting.",
            Self::MeetingAlreadyActive => "A meeting is already active.",
            Self::MeetingRecovering => {
                "Murmur is recovering an interrupted meeting transcript. Try again shortly."
            }
            Self::MeetingStopMicrophone => {
                "Stop the active microphone recording before starting a meeting."
            }
            Self::MeetingWaitForFile => {
                "Wait for file transcription to finish before starting a meeting."
            }
            Self::MeetingWaitForBenchmark => {
                "Wait for the benchmark to finish before starting a meeting."
            }
            Self::MeetingWaitForDictation => "Wait for dictation to finish before starting a meeting.",
            Self::PreviewDictationBusy => "Finish the current dictation before testing the microphone",
            Self::PreviewMeeting => {
                "Finish the current meeting transcript before testing the microphone"
            }
            Self::PreviewBenchmark => "Finish the current benchmark before testing the microphone",
            Self::PreviewCorpus => {
                "Finish the personal corpus recording before testing the microphone"
            }
            Self::PreviewAlreadyActive => "A microphone test is already active",
            Self::PreviewNoLongerConnecting => "preview_no_longer_connecting",
            Self::MicBenchMeeting => {
                "Wait for the meeting transcript to finish before testing the microphone"
            }
            Self::MicBenchStopRecording => "Stop recording before testing the microphone",
            Self::MicBenchCorpus => "Finish the corpus recording before testing the microphone",
            Self::MicBenchAlreadyRunning => "A microphone startup benchmark is already running",
            Self::MicBenchWaitForModelPrep => {
                "Wait for model preparation to finish before testing the microphone"
            }
            Self::BenchMeeting => "Wait for the meeting transcript to finish before benchmarking",
            Self::BenchStopRecording => "Stop recording before running a benchmark",
            Self::BenchCorpus => "Finish the corpus recording before benchmarking",
            Self::BenchAlreadyRunning => "A benchmark is already running",
            Self::BenchWaitForModelPrep => {
                "Wait for model preparation to finish before benchmarking"
            }
            Self::CorpusMeeting => "Finish the current meeting transcript first",
            Self::CorpusDictation => "Finish the current dictation before recording corpus audio",
            Self::CorpusFile => "Finish the current file transcription first",
            Self::CorpusBenchmark => "Finish the current benchmark first",
            Self::CorpusAlreadyActive => "A corpus recording is already active",
            Self::SharedBackendBusy => {
                "Wait for the benchmark or current model preparation to finish before changing models."
            }
            Self::ModelRemovalTransitionBusy => "Wait for the current recording transition to finish",
            Self::ModelRemovalDictationBusy => "Wait for the current dictation operation to finish",
            Self::ModelRemovalOccupied => {
                "Stop recording and wait for transcription to finish before removing a model"
            }
            Self::ModelRemovalOperationBusy => {
                "Wait for the current model operation to finish before removing a model"
            }
            Self::ModelPreparationNotEligible => "model_preparation_not_eligible",
        })
    }

    /// The literal text of the refusal, whichever variant it is.
    pub fn text(&self) -> &str {
        match self.static_text() {
            Ok(literal) => literal,
            Err(payload) => payload,
        }
    }

    /// The `{"type","state"}` reply of a dictation-start refusal (`None` for
    /// refusals that are `Err(String)` or tags).
    pub fn reply(&self) -> Option<(ReplyType, Status)> {
        Some(match self {
            Self::AppDisabled => (ReplyType::AppDisabled, Status::Idle),
            Self::BusyRestarting => (ReplyType::BusyRestarting, Status::Idle),
            Self::BusyMeeting => (ReplyType::BusyMeeting, Status::Idle),
            Self::BusyTranscribingFile => (ReplyType::BusyTranscribingFile, Status::Idle),
            Self::BusyRecordingCorpus => (ReplyType::BusyRecordingCorpus, Status::Idle),
            Self::BusyBenchmarking => (ReplyType::BusyBenchmarking, Status::Idle),
            Self::AlreadyStarting => (ReplyType::AlreadyStarting, Status::Starting),
            Self::AlreadyRecording => (ReplyType::AlreadyRecording, Status::Recording),
            Self::AudioRecovering | Self::AudioRecoveringCleanupPending => {
                (ReplyType::AudioRecovering, Status::Recovering)
            }
            Self::AlreadyProcessing => (ReplyType::AlreadyProcessing, Status::Processing),
            _ => return None,
        })
    }
}

// ---------------------------------------------------------------------------
// Ordered checks
// ---------------------------------------------------------------------------

type Eval = fn(&Occupancy) -> Option<Admission>;

/// One ordered check: `Some` ends evaluation with that outcome.
struct Step {
    name: &'static str,
    eval: Eval,
}

fn refuse(code: RefusalCode) -> Option<Admission> {
    Some(Admission::RefuseWith(code))
}

fn refuse_if(condition: bool, code: RefusalCode) -> Option<Admission> {
    if condition {
        refuse(code)
    } else {
        None
    }
}

/// `dictation_activity_blocker` without the status arm: relaunch excluded.
fn activity_blocker(o: &Occupancy) -> bool {
    o.meeting_blocks_asr()
        || o.file_transcribing
        || o.corpus_active
        || o.benchmark_running
        || o.status != Status::Idle
}

/// `dictation_start_blocker`: relaunch, then the activity blockers.
fn start_blocker(o: &Occupancy) -> bool {
    o.relaunch_committed || activity_blocker(o)
}

/// Which coordinator refusal `try_start` reports for a Benchmark-style claim.
fn coordinator_busy(o: &Occupancy) -> Option<bool> {
    if o.benchmark_running {
        Some(true)
    } else if o.shared_backend_change || o.coordinator_idle == Some(false) {
        Some(false)
    } else {
        None
    }
}

static DICTATION_START: &[Step] = &[
    Step {
        name: "app_disabled",
        eval: |o| refuse_if(o.app_disabled, RefusalCode::AppDisabled),
    },
    Step {
        name: "smart_auto_with_fixed_device",
        eval: |o| {
            refuse_if(
                o.fixed_and_smart_auto,
                RefusalCode::SmartAutoWithFixedDevice,
            )
        },
    },
    Step {
        name: "relaunch_committed",
        eval: |o| refuse_if(o.relaunch_committed, RefusalCode::BusyRestarting),
    },
    Step {
        name: "meeting_blocks_asr",
        eval: |o| refuse_if(o.meeting_blocks_asr(), RefusalCode::BusyMeeting),
    },
    Step {
        name: "file_transcribing",
        eval: |o| refuse_if(o.file_transcribing, RefusalCode::BusyTranscribingFile),
    },
    Step {
        name: "corpus_active",
        eval: |o| refuse_if(o.corpus_active, RefusalCode::BusyRecordingCorpus),
    },
    Step {
        name: "benchmark_running",
        eval: |o| refuse_if(o.benchmark_running, RefusalCode::BusyBenchmarking),
    },
    Step {
        name: "status",
        eval: |o| match o.status {
            Status::Idle => None,
            Status::Starting => refuse(RefusalCode::AlreadyStarting),
            Status::Recording => refuse(RefusalCode::AlreadyRecording),
            Status::Recovering => refuse(RefusalCode::AudioRecovering),
            Status::Processing => refuse(RefusalCode::AlreadyProcessing),
        },
    },
    Step {
        name: "cleanup_pending",
        eval: |o| {
            refuse_if(
                o.cleanup_pending,
                RefusalCode::AudioRecoveringCleanupPending,
            )
        },
    },
    Step {
        name: "smart_auto_selection",
        eval: |o| match &o.smart_auto {
            SmartAutoObservation::NotRequested | SmartAutoObservation::Requested(Some(Ok(()))) => {
                None
            }
            SmartAutoObservation::Requested(None) => {
                Some(Admission::NeedObservation(ObsKind::SmartAutoSelection))
            }
            SmartAutoObservation::Requested(Some(Err(text))) => {
                refuse(RefusalCode::SmartAutoFailed(text.clone()))
            }
        },
    },
];

static DICTATION_PREPARE: &[Step] = &[
    Step {
        name: "start_blocker",
        eval: |o| refuse_if(start_blocker(o), RefusalCode::DictationBusy),
    },
    Step {
        name: "preview_active",
        eval: |o| refuse_if(o.preview_active, RefusalCode::PreviewActive),
    },
    Step {
        name: "audio_active",
        eval: |o| refuse_if(o.audio_active, RefusalCode::MicrophoneBusy),
    },
    Step {
        name: "cleanup_pending",
        eval: |o| refuse_if(o.cleanup_pending, RefusalCode::CleanupPending),
    },
];

static KEY_DOWN_CAPTURE: &[Step] = &[
    Step {
        name: "listener_matches",
        eval: |o| {
            refuse_if(
                !o.key_down.listener_matches,
                RefusalCode::KeyDownListenerChanged,
            )
        },
    },
    Step {
        name: "keyboard_processing",
        eval: |o| refuse_if(o.keyboard_processing, RefusalCode::KeyDownProcessingSkipped),
    },
    Step {
        name: "app_disabled",
        eval: |o| refuse_if(o.app_disabled, RefusalCode::KeyDownDisabled),
    },
    Step {
        name: "gesture_active",
        eval: |o| refuse_if(!o.key_down.gesture_active, RefusalCode::KeyDownGestureEnded),
    },
    Step {
        name: "microphone_permission",
        eval: |o| {
            refuse_if(
                !o.key_down.microphone_granted,
                RefusalCode::KeyDownPermissionNotGranted,
            )
        },
    },
    Step {
        name: "start_blocker",
        eval: |o| refuse_if(start_blocker(o), RefusalCode::DictationBusy),
    },
    Step {
        // The one `K` cell: a visible preview is taken over in place when it is
        // the handoff candidate; any other preview refuses (`key_down_admission_from`).
        name: "preview_active",
        eval: |o| {
            if !o.preview_active {
                None
            } else {
                match o.handoff_candidate {
                    Some(preview) => Some(Admission::Takeover(PreviewId(preview))),
                    None => refuse(RefusalCode::PreviewActive),
                }
            }
        },
    },
    Step {
        name: "audio_active",
        eval: |o| refuse_if(o.audio_active, RefusalCode::MicrophoneBusy),
    },
    Step {
        name: "cleanup_pending",
        eval: |o| refuse_if(o.cleanup_pending, RefusalCode::CleanupPending),
    },
];

static RELAUNCH_GATE: &[Step] = &[
    Step {
        // `dictation_activity_blocker`: the relaunch's own commitment is excluded.
        name: "activity_blocker",
        eval: |o| refuse_if(activity_blocker(o), RefusalCode::DictationBusy),
    },
    Step {
        name: "preview_active",
        eval: |o| refuse_if(o.preview_active, RefusalCode::PreviewActive),
    },
    Step {
        name: "audio_active",
        eval: |o| refuse_if(o.audio_active, RefusalCode::MicrophoneBusy),
    },
    Step {
        name: "cleanup_pending",
        eval: |o| refuse_if(o.cleanup_pending, RefusalCode::CleanupPending),
    },
];

static FILE_PRE_CLAIM: &[Step] = &[
    Step {
        // Shell pre-admission `transition_after_stopping_preview`: an `Err` from
        // the stop refuses before anything else.
        name: "preview_stop",
        eval: |o| match &o.preview_stop {
            Some(Err(text)) => refuse(RefusalCode::Text(text.clone())),
            _ => None,
        },
    },
    Step {
        name: "meeting_blocks_asr",
        eval: |o| refuse_if(o.meeting_blocks_asr(), RefusalCode::FileWaitForMeeting),
    },
    Step {
        // `file_transcribing.swap(true)`: refuse when the prior value was true.
        // Before the shell has swapped, the best view is the flag itself.
        name: "file_claim",
        eval: |o| {
            refuse_if(
                o.file_claim_prior.unwrap_or(o.file_transcribing),
                RefusalCode::FileAlreadyTranscribing,
            )
        },
    },
];

static FILE_POST_CLAIM: &[Step] = &[
    Step {
        name: "benchmark_running",
        eval: |o| refuse_if(o.benchmark_running, RefusalCode::FileWaitForBenchmark),
    },
    Step {
        name: "corpus_active",
        eval: |o| refuse_if(o.corpus_active, RefusalCode::FileFinishCorpus),
    },
    Step {
        name: "status",
        eval: |o| refuse_if(o.status != Status::Idle, RefusalCode::FileLiveAudioActive),
    },
];

static MEETING_PRE_CLAIM: &[Step] = &[
    Step {
        name: "relaunch_committed",
        eval: |o| refuse_if(o.relaunch_committed, RefusalCode::MeetingRestarting),
    },
    Step {
        name: "mic_startup_benchmark_active",
        eval: |o| {
            refuse_if(
                o.mic_startup_benchmark_active,
                RefusalCode::MeetingMicStartupCheck,
            )
        },
    },
    Step {
        name: "corpus_active",
        eval: |o| refuse_if(o.corpus_active, RefusalCode::MeetingFinishCorpus),
    },
    Step {
        name: "meeting_active",
        eval: |o| refuse_if(o.meeting_active, RefusalCode::MeetingAlreadyActive),
    },
    Step {
        name: "meeting_inference_active",
        eval: |o| refuse_if(o.meeting_inference_active, RefusalCode::MeetingRecovering),
    },
    Step {
        name: "audio_active",
        eval: |o| refuse_if(o.audio_active, RefusalCode::MeetingStopMicrophone),
    },
    Step {
        name: "file_transcribing",
        eval: |o| refuse_if(o.file_transcribing, RefusalCode::MeetingWaitForFile),
    },
    Step {
        name: "benchmark_running",
        eval: |o| refuse_if(o.benchmark_running, RefusalCode::MeetingWaitForBenchmark),
    },
    Step {
        name: "status",
        eval: |o| {
            refuse_if(
                o.status != Status::Idle,
                RefusalCode::MeetingWaitForDictation,
            )
        },
    },
];

static MEETING_POST_CLAIM: &[Step] = &[Step {
    // Dekker: the claim stored, then `relaunch_committed` is re-read; a refusal
    // undoes the claim.
    name: "relaunch_recheck",
    eval: |o| {
        refuse_if(
            o.relaunch_recheck == Some(true),
            RefusalCode::MeetingRestarting,
        )
    },
}];

static PREVIEW_START: &[Step] = &[
    Step {
        // The code retires Standby before every read (`retire_standby`, ahead
        // of `refuse_revoked_start`), so the wait comes first.
        name: "standby",
        eval: |o| {
            if o.standby {
                Some(Admission::WaitThen(Retry::RetireStandby))
            } else {
                None
            }
        },
    },
    Step {
        name: "start_revoked",
        eval: |o| refuse_if(o.preview_start_revoked, RefusalCode::PreviewDictationBusy),
    },
    Step {
        name: "status",
        eval: |o| refuse_if(o.status != Status::Idle, RefusalCode::PreviewDictationBusy),
    },
    Step {
        name: "meeting_blocks_asr",
        eval: |o| refuse_if(o.meeting_blocks_asr(), RefusalCode::PreviewMeeting),
    },
    Step {
        name: "benchmark_running",
        eval: |o| refuse_if(o.benchmark_running, RefusalCode::PreviewBenchmark),
    },
    Step {
        name: "corpus_active",
        eval: |o| refuse_if(o.corpus_active, RefusalCode::PreviewCorpus),
    },
    Step {
        name: "app_disabled",
        eval: |o| refuse_if(o.app_disabled, RefusalCode::EnableMurmurBeforeMicTest),
    },
    Step {
        name: "preview_active",
        eval: |o| refuse_if(o.preview_active, RefusalCode::PreviewAlreadyActive),
    },
    Step {
        // `preview_audio_blocker`: an unconfirmed key-down capture is retired
        // once (bounded wait); then `audio_active` decides, even if the capture
        // is still tearing down.
        name: "unconfirmed_key_down",
        eval: |o| {
            if o.unconfirmed_key_down {
                Some(Admission::WaitThen(Retry::RetireKeyDownCapture))
            } else {
                None
            }
        },
    },
    Step {
        name: "audio_active",
        eval: |o| refuse_if(o.audio_active, RefusalCode::MicrophoneStillInUse),
    },
];

/// The post-claim recheck: only `is_connecting(id)` (`microphone_preview.rs:270`).
static PREVIEW_START_RECHECK: &[Step] = &[Step {
    name: "preview_still_connecting",
    eval: |o| {
        refuse_if(
            o.preview_still_connecting == Some(false),
            RefusalCode::PreviewNoLongerConnecting,
        )
    },
}];

static MIC_BENCH_PRE_CLAIM: &[Step] = &[
    Step {
        name: "standby",
        eval: |o| {
            if o.standby {
                Some(Admission::WaitThen(Retry::RetireStandby))
            } else {
                None
            }
        },
    },
    Step {
        name: "meeting_blocks_asr",
        eval: |o| refuse_if(o.meeting_blocks_asr(), RefusalCode::MicBenchMeeting),
    },
    Step {
        name: "status",
        eval: |o| refuse_if(o.status != Status::Idle, RefusalCode::MicBenchStopRecording),
    },
    Step {
        name: "file_transcribing",
        eval: |o| refuse_if(o.file_transcribing, RefusalCode::WaitForFileTranscription),
    },
    Step {
        name: "corpus_active",
        eval: |o| refuse_if(o.corpus_active, RefusalCode::MicBenchCorpus),
    },
    Step {
        name: "audio_active",
        eval: |o| refuse_if(o.audio_active, RefusalCode::MicrophoneStillInUse),
    },
    Step {
        name: "app_disabled",
        eval: |o| refuse_if(o.app_disabled, RefusalCode::EnableMurmurBeforeMicTest),
    },
    Step {
        // `benchmark.try_start()`: "already running" if the coordinator runs a
        // Benchmark, else "Wait for model preparation".
        name: "coordinator",
        eval: |o| match coordinator_busy(o) {
            Some(true) => refuse(RefusalCode::MicBenchAlreadyRunning),
            Some(false) => refuse(RefusalCode::MicBenchWaitForModelPrep),
            None => None,
        },
    },
];

static MIC_BENCH_POST_CLAIM: &[Step] = &[Step {
    name: "second_claim",
    eval: |o| {
        refuse_if(
            o.second_claim == Some(false),
            RefusalCode::MicBenchAlreadyRunning,
        )
    },
}];

static BENCHMARK: &[Step] = &[
    Step {
        name: "meeting_blocks_asr",
        eval: |o| refuse_if(o.meeting_blocks_asr(), RefusalCode::BenchMeeting),
    },
    Step {
        name: "status",
        eval: |o| refuse_if(o.status != Status::Idle, RefusalCode::BenchStopRecording),
    },
    Step {
        name: "file_transcribing",
        eval: |o| refuse_if(o.file_transcribing, RefusalCode::WaitForFileTranscription),
    },
    Step {
        name: "corpus_active",
        eval: |o| refuse_if(o.corpus_active, RefusalCode::BenchCorpus),
    },
    Step {
        name: "coordinator",
        eval: |o| match coordinator_busy(o) {
            Some(true) => refuse(RefusalCode::BenchAlreadyRunning),
            Some(false) => refuse(RefusalCode::BenchWaitForModelPrep),
            None => None,
        },
    },
];

static CORPUS: &[Step] = &[
    Step {
        name: "meeting_blocks_asr",
        eval: |o| refuse_if(o.meeting_blocks_asr(), RefusalCode::CorpusMeeting),
    },
    Step {
        name: "status",
        eval: |o| refuse_if(o.status != Status::Idle, RefusalCode::CorpusDictation),
    },
    Step {
        name: "file_transcribing",
        eval: |o| refuse_if(o.file_transcribing, RefusalCode::CorpusFile),
    },
    Step {
        name: "benchmark_running",
        eval: |o| refuse_if(o.benchmark_running, RefusalCode::CorpusBenchmark),
    },
    Step {
        name: "corpus_claim",
        eval: |o| refuse_if(o.corpus_active, RefusalCode::CorpusAlreadyActive),
    },
];

static SHARED_BACKEND_CHANGE: &[Step] = &[Step {
    // Admit iff the coordinator is Idle; reads nothing about dictation, so it
    // may be admitted while dictation records.
    name: "coordinator",
    eval: |o| {
        refuse_if(
            coordinator_busy(o).is_some(),
            RefusalCode::SharedBackendBusy,
        )
    },
}];

static MODEL_PREPARATION: &[Step] = &[
    Step {
        // `recording_id.load() == recording_id` comes first in both reads
        // (`recording.rs:1153`, `:1168`): a stale worker is never eligible.
        // The idle warm-up has no generation (`recording.rs:1240`, `:1248`).
        name: "generation",
        eval: |o| match o.model_prep_variant {
            ModelPrepVariant::RecordingStart => refuse_if(
                o.expected_recording_id != Some(o.recording_id),
                RefusalCode::ModelPreparationNotEligible,
            ),
            ModelPrepVariant::IdleWarm => None,
        },
    },
    Step {
        name: "eligible",
        eval: |o| {
            let status_ok = match o.model_prep_variant {
                ModelPrepVariant::RecordingStart => {
                    matches!(o.status, Status::Starting | Status::Recording)
                }
                ModelPrepVariant::IdleWarm => o.status == Status::Idle,
            };
            refuse_if(
                !(status_ok && o.model_matches),
                RefusalCode::ModelPreparationNotEligible,
            )
        },
    },
];

static MODEL_REMOVAL: &[Step] = &[
    Step {
        name: "transition_lock",
        eval: |o| {
            refuse_if(
                o.transition_lock_busy,
                RefusalCode::ModelRemovalTransitionBusy,
            )
        },
    },
    Step {
        name: "dictation_lock",
        eval: |o| {
            refuse_if(
                o.dictation_lock_busy,
                RefusalCode::ModelRemovalDictationBusy,
            )
        },
    },
    Step {
        name: "occupied",
        eval: |o| {
            refuse_if(
                o.status != Status::Idle
                    || o.file_transcribing
                    || o.meeting_blocks_asr()
                    || o.audio_active,
                RefusalCode::ModelRemovalOccupied,
            )
        },
    },
    Step {
        name: "coordinator",
        eval: |o| {
            refuse_if(
                coordinator_busy(o).is_some(),
                RefusalCode::ModelRemovalOperationBusy,
            )
        },
    },
];

fn steps(request: Request) -> &'static [Step] {
    match request {
        Request::DictationStart => DICTATION_START,
        Request::DictationPrepare => DICTATION_PREPARE,
        Request::KeyDownCapture => KEY_DOWN_CAPTURE,
        Request::RelaunchGate => RELAUNCH_GATE,
        Request::FileTranscriptionPreClaim => FILE_PRE_CLAIM,
        Request::FileTranscriptionPostClaim => FILE_POST_CLAIM,
        Request::MeetingPreClaim => MEETING_PRE_CLAIM,
        Request::MeetingPostClaim => MEETING_POST_CLAIM,
        Request::PreviewStart => PREVIEW_START,
        Request::PreviewStartRecheck => PREVIEW_START_RECHECK,
        Request::MicStartupBenchmarkPreClaim => MIC_BENCH_PRE_CLAIM,
        Request::MicStartupBenchmarkPostClaim => MIC_BENCH_POST_CLAIM,
        Request::Benchmark => BENCHMARK,
        Request::Corpus => CORPUS,
        Request::SharedBackendChange => SHARED_BACKEND_CHANGE,
        Request::ModelPreparation => MODEL_PREPARATION,
        Request::ModelRemoval => MODEL_REMOVAL,
    }
}

/// What a request carries once every check has passed.
fn admitted(request: Request, o: &Occupancy) -> Admission {
    let claims = match request {
        Request::DictationStart => Claims {
            claim: ClaimKind::DictationTransition,
            preview: match (o.preview_active, o.handoff_candidate) {
                (false, _) => PreviewDisposition::NotPresent,
                (true, Some(preview)) => PreviewDisposition::KeptForHandoff(PreviewId(preview)),
                (true, None) => PreviewDisposition::StopThenRetry,
            },
        },
        Request::FileTranscriptionPreClaim => Claims {
            claim: ClaimKind::FileTranscription,
            preview: if o.preview_active {
                PreviewDisposition::StoppedFirst
            } else {
                PreviewDisposition::NotPresent
            },
        },
        Request::MeetingPreClaim => Claims::of(ClaimKind::MeetingOwnership),
        Request::PreviewStart => Claims::of(ClaimKind::PreviewGeneration),
        Request::MicStartupBenchmarkPreClaim | Request::Benchmark => {
            Claims::of(ClaimKind::BenchmarkCoordinator)
        }
        Request::Corpus => Claims::of(ClaimKind::Corpus),
        Request::SharedBackendChange | Request::ModelRemoval => {
            Claims::of(ClaimKind::SharedBackendChange)
        }
        Request::DictationPrepare
        | Request::KeyDownCapture
        | Request::RelaunchGate
        | Request::FileTranscriptionPostClaim
        | Request::MeetingPostClaim
        | Request::PreviewStartRecheck
        | Request::MicStartupBenchmarkPostClaim
        | Request::ModelPreparation => Claims::of(ClaimKind::None),
    };
    Admission::Admit(claims)
}

/// The result of [`admit_traced`]: the admission and the checks that ran.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Evaluation {
    /// The admission.
    pub admission: Admission,
    /// Names of the checks evaluated, in order. Ends at the first check that
    /// fired; on `Admit` it is the full list.
    pub evaluated: Vec<&'static str>,
}

/// The check names of a request, in C6.2 order.
pub fn check_order(request: Request) -> Vec<&'static str> {
    steps(request).iter().map(|step| step.name).collect()
}

/// [`admit`], also reporting which checks were evaluated (the proof that a
/// refusal short-circuits: later checks are not evaluated, C6.1a).
pub fn admit_traced(request: Request, occupancy: &Occupancy) -> Evaluation {
    run(request, steps(request), occupancy)
}

/// Resume `request` after a [`Retry`] the shell has performed: evaluate
/// **only the checks after the wait**, against a fresh `occupancy`. Earlier
/// checks are neither re-run nor re-read. `None` if `request` has no such wait.
pub fn admit_after(request: Request, retry: Retry, occupancy: &Occupancy) -> Option<Evaluation> {
    let all = steps(request);
    let at = all.iter().position(|step| step.name == retry.check())?;
    Some(run(request, &all[at + 1..], occupancy))
}

fn run(request: Request, steps: &[Step], occupancy: &Occupancy) -> Evaluation {
    let mut evaluated = Vec::new();
    for step in steps {
        evaluated.push(step.name);
        if let Some(admission) = (step.eval)(occupancy) {
            return Evaluation {
                admission,
                evaluated,
            };
        }
    }
    Evaluation {
        admission: admitted(request, occupancy),
        evaluated,
    }
}

/// Decide whether `request` may proceed against `occupancy` (C6.2). Pure: no
/// claim is made, no lock is taken, nothing is read beyond `occupancy`.
pub fn admit(request: Request, occupancy: &Occupancy) -> Admission {
    admit_traced(request, occupancy).admission
}

#[cfg(test)]
mod tests;
