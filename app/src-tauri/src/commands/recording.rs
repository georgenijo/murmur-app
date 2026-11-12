use crate::audio_lifecycle::{self, AudioCancelReason, AudioLifecycleEvent, AudioStartError};
use crate::dictation_context::{self, DictationContextSnapshot, ResolverInputs};
use crate::dictation_diagnostics::DictationCaptureCompletion;
use crate::dictation_telemetry::{DictationErrorCode, DictationTerminalOutcome};
use crate::microphone_auto::SmartAutoRequest;
use crate::model_runtime::{self, PreparationReason};
use crate::performance_metrics::{delivery as dt, MeasurementV1 as DeliveryMeasurement};
use crate::performance_metrics::{
    AcceleratorV1, ContentFreeInputSummaryV1, ModelWarmStateV1, PerformanceRunGuard,
    PerformanceStageV1, PerformanceStoreError, RunCorrelationV1, RunOutcomeV1, RuntimeBackendV1,
    RuntimeIdentityV1, RuntimeRoleV1, StableRunErrorV1, StageOutcomeV1, StageTimingV1,
};
use crate::state::{AppState, DictationStatus};
use crate::transcriber;
use crate::{audio, audio_decode, injector, keyboard, vad};
use crate::{MutexExt, State};
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, OnceLock};
use tauri::{Emitter, Manager};

/// Max number of code identifiers fed to Whisper as an initial prompt. Whisper
/// truncates the prompt to its ~224-token context window anyway, so this keeps
/// the bias list focused on the most frequent (most useful) terms. This budget is
/// intentionally *smaller* than [`CORRECTION_TERMS`]: the Whisper prompt is
/// token-bound, while Smart Correction is not (see [`CORRECTION_TERMS`]).
const WHISPER_PROMPT_TERMS: usize = 96;

/// Max number of code identifiers fed to the Smart Correction matcher
/// (`correction.rs` Tier 1/2). Unlike the Whisper initial prompt this path has no
/// token budget — it's a couple of linear passes over the transcript — so a far
/// larger term list is a pure recall win. The folder scan ranks ALL identifiers
/// once; the top [`CORRECTION_TERMS`] feed correction and the top
/// [`WHISPER_PROMPT_TERMS`] (a rank-prefix of them) feed Whisper.
const CORRECTION_TERMS: usize = 500;

/// Version of the stable fleet reliability contract attached to every eligible
/// live-dictation request. Historical attempts without this marker remain
/// visible to the legacy funnel but cannot be treated as SLO evidence.
const DICTATION_SLO_CONTRACT: u64 = 1;
const PARTIAL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(700);
const PARTIAL_MIN_SAMPLES: usize = 16_000 * 800 / 1_000;
const PARTIAL_WINDOW_SAMPLES: usize = 16_000 * 20;
const WHISPER_PARTIAL_WINDOW_SAMPLES: usize = 16_000 * 6;
const WHISPER_PARTIAL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(1500);
const WHISPER_PARTIAL_DEADLINE: std::time::Duration = std::time::Duration::from_secs(3);
type WhisperPreview = Arc<Mutex<Option<transcriber::whisper::WhisperPreviewBackend>>>;

struct SuccessfulPrivateCapture<'a> {
    raw_text: &'a str,
    final_text: &'a str,
    model_id: &'a str,
    total_ms: u64,
}

fn emit_dictation_terminal(
    state: &State,
    recording_id: u64,
    outcome: DictationTerminalOutcome,
    error_code: DictationErrorCode,
) -> bool {
    emit_dictation_terminal_with_capture(state, recording_id, outcome, error_code, 0, None)
}

fn emit_dictation_terminal_with_capture(
    state: &State,
    recording_id: u64,
    outcome: DictationTerminalOutcome,
    error_code: DictationErrorCode,
    char_count: u64,
    success: Option<SuccessfulPrivateCapture<'_>>,
) -> bool {
    let telemetry_emitted = state
        .app_state
        .dictation_telemetry
        .emit_terminal_with_output(recording_id, outcome, error_code, char_count);
    let completion = match (outcome, success) {
        (DictationTerminalOutcome::Success, Some(success)) => DictationCaptureCompletion::Success {
            raw_text: success.raw_text,
            final_text: success.final_text,
            model_id: success.model_id,
            total_ms: success.total_ms,
        },
        _ => DictationCaptureCompletion::Terminal {
            outcome,
            error_code,
        },
    };
    if let Some(capture) = state
        .dictation_diagnostics
        .complete(recording_id, completion)
    {
        let diagnostics = state.dictation_diagnostics.clone();
        tauri::async_runtime::spawn_blocking(move || {
            if diagnostics.persist_capture(capture).is_err() {
                tracing::warn!(
                    target: "system",
                    "dictation diagnostic capture could not be saved"
                );
            }
        });
    }
    telemetry_emitted
}

fn dictation_partial_is_current(app_state: &AppState, recording_id: u64) -> bool {
    recording_id > 0
        && app_state.recording_id.load(Ordering::SeqCst) == recording_id
        && app_state.dictation.lock_or_recover().status == DictationStatus::Recording
        && !app_state.is_cancelled(recording_id)
}

/// Called with the backend held: never wait on the status lock, since idle
/// unload holds status while acquiring that backend in the opposite order.
pub(crate) fn dictation_partial_is_current_if_uncontended(
    app_state: &AppState,
    recording_id: u64,
) -> bool {
    let Some(dictation) = app_state.dictation.try_lock_or_recover() else {
        return false;
    };
    recording_id > 0
        && app_state.recording_id.load(Ordering::SeqCst) == recording_id
        && dictation.status == DictationStatus::Recording
        && !app_state.is_cancelled(recording_id)
}

/// Whether a new preview decode may start. Generation ownership as above, and
/// nothing starts once the stop gesture has begun: a running Core ML call
/// cannot be cancelled, so the final decode must never queue behind a decode
/// that started after the user let go (workstream E3's cap). A decode that
/// already runs may still finish and publish; only starting is refused.
fn dictation_preview_may_start(app_state: &AppState, recording_id: u64) -> bool {
    dictation_partial_is_current(app_state, recording_id)
        && !crate::hotkey_release::stop_gesture_pending(recording_id)
}

/// [`dictation_preview_may_start`] for the moment the backend is acquired.
fn dictation_preview_may_start_if_uncontended(app_state: &AppState, recording_id: u64) -> bool {
    dictation_partial_is_current_if_uncontended(app_state, recording_id)
        && !crate::hotkey_release::stop_gesture_pending(recording_id)
}

fn note_nonempty_dictation_preview(app_state: &AppState, recording_id: u64) {
    if dictation_partial_is_current(app_state, recording_id) {
        // A late old worker cannot overwrite evidence already owned by a newer ID.
        app_state
            .dictation_nonempty_preview_id
            .fetch_max(recording_id, Ordering::SeqCst);
    }
}

fn emit_preview_vad_disagreement(app_state: &AppState, recording_id: u64, sample_count: usize) {
    if recording_id > 0
        && app_state
            .dictation_nonempty_preview_id
            .load(Ordering::SeqCst)
            == recording_id
    {
        tracing::info!(
            target: "pipeline",
            event_code = "pipeline.preview_vad_disagreement",
            recording_id,
            sample_count,
            "nonempty preview followed by final VAD no speech"
        );
    }
}

#[cfg(test)]
#[path = "recording_preview_tests.rs"]
mod preview_tests;

#[cfg(test)]
#[path = "recording_stream_reuse_tests.rs"]
mod stream_reuse_app_tests;

fn try_begin_dictation_partial(app_state: &AppState, recording_id: u64) -> bool {
    dictation_partial_is_current(app_state, recording_id)
        && app_state
            .dictation_partial_in_flight_id
            .compare_exchange(0, recording_id, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
}

fn finish_dictation_partial(app_state: &AppState, recording_id: u64) -> bool {
    app_state
        .dictation_partial_in_flight_id
        .compare_exchange(recording_id, 0, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
}

struct DictationPartialLease<R: tauri::Runtime> {
    app: tauri::AppHandle<R>,
    recording_id: u64,
}

impl<R: tauri::Runtime> Drop for DictationPartialLease<R> {
    fn drop(&mut self) {
        let state = self.app.state::<State>();
        if std::thread::panicking() {
            state
                .app_state
                .stream_reuse
                .lock_or_recover()
                .fail(self.recording_id, crate::stream_reuse::Reason::Superseded);
        }
        if finish_dictation_partial(&state.app_state, self.recording_id) {
            state.meeting_suggestions.wake(false);
        }
    }
}

fn dictation_partial_window(samples: &[f32]) -> &[f32] {
    if samples.len() > PARTIAL_WINDOW_SAMPLES {
        &samples[samples.len() - PARTIAL_WINDOW_SAMPLES..]
    } else {
        samples
    }
}

fn emit_dictation_partial_tick(recording_id: u64, outcome: &'static str, sample_count: usize) {
    tracing::info!(
        target: "pipeline",
        event_code = "pipeline.dictation_partial_tick",
        recording_id,
        outcome,
        sample_count = sample_count.min(PARTIAL_WINDOW_SAMPLES) as u64,
        "dictation live partial tick"
    );
}

/// Register `recording_id`'s partial ticker as the preview hide's ticker side.
/// The caller holds the dictation lock: the stop transition claims the hide
/// under the same lock, so registration either precedes that claim or sees
/// the recording already stopped and refuses. Older generations never
/// replace a newer registration.
fn register_partial_ticker_locked(
    app_state: &AppState,
    dictation: &crate::state::DictationState,
    recording_id: u64,
) -> bool {
    recording_id > 0
        && app_state.recording_id.load(Ordering::SeqCst) == recording_id
        && dictation.status == DictationStatus::Recording
        && !app_state.is_cancelled(recording_id)
        && crate::commands::dictation_preview::note_ticker_started(recording_id)
}

/// Spawn the live-preview ticker. `register` runs under the dictation lock
/// (see [`register_partial_ticker_locked`]); no ticker runs if it refuses.
fn spawn_dictation_partial_ticker(
    app: tauri::AppHandle,
    recording_id: u64,
    register: impl FnOnce() -> bool,
) {
    let Some(context) = app.state::<State>().app_state.active_context(recording_id) else {
        return;
    };
    let model = &context.transcription.model_name;
    let whisper = transcriber::whisper::supports_live_preview(model);
    if !whisper && !transcriber::is_coreml_model(model) {
        emit_dictation_partial_tick(recording_id, "unsupported_model", 0);
        return;
    }
    if !register() {
        return;
    }
    // Decode while speaking: only for a recording that asked for it at start
    // (frozen snapshot), runs Core ML, and whose incremental VAD is alive; the
    // pieces are cut from that VAD's segments. Anything else decodes the whole
    // clip at Stop, exactly as before.
    {
        let state = app.state::<State>();
        if !whisper
            && context.transcription.streaming_pipeline_enabled
            && vad::is_enabled(context.transcription.vad_sensitivity)
            && state.app_state.vad_stream.consumed(recording_id).is_some()
        {
            state
                .app_state
                .stream_reuse
                .lock_or_recover()
                .begin(recording_id);
        }
    }
    let interval = if whisper {
        WHISPER_PARTIAL_INTERVAL
    } else {
        PARTIAL_INTERVAL
    };
    let preview: WhisperPreview = Arc::new(Mutex::new(None));
    drop(tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(interval).await;
            if !dictation_partial_is_current(&app.state::<State>().app_state, recording_id) {
                break;
            }
            // Do not await a decode: ticks skip the global single-flight lease,
            // and stop/hide must not wait for inference or model initialization.
            let tick_app = app.clone();
            let tick_preview = preview.clone();
            drop(tauri::async_runtime::spawn(async move {
                decode_one_dictation_partial(&tick_app, recording_id, tick_preview).await;
            }));
        }
        // An accepted stop hands this hide to delivery, which queues it
        // behind the paste; only cancels and interruptions hide here.
        crate::commands::dictation_preview::hide_on_ticker_exit(&app, recording_id);
    }));
}

/// Start the incremental VAD (streaming setting, workstream E1) for a recording
/// that has just become `Recording`. Reads only the context frozen at recording
/// start; the batch VAD at Stop stays the fallback for every failure.
fn spawn_vad_stream(app: tauri::AppHandle, recording_id: u64) {
    let Some(context) = app.state::<State>().app_state.active_context(recording_id) else {
        return;
    };
    let transcription = &context.transcription;
    if !transcription.streaming_pipeline_enabled
        || !vad::is_enabled(transcription.vad_sensitivity)
        || !vad::vad_model_path().is_some_and(|path| path.exists())
    {
        return;
    }
    crate::vad_stream::start(app, recording_id, dictation_partial_is_current);
}

/// A piece of VAD-trimmed speech ready to decode, with its plan.
type StreamCommit = (crate::stream_reuse::CommitPlan, Vec<f32>);

/// Decide, from the incremental VAD's current segments, whether a piece can be
/// committed now, and gather exactly that piece's audio. Takes no backend lock
/// and copies only the piece. Any inconsistency ends reuse for this recording
/// (Stop then decodes the whole clip).
fn prepare_stream_commit(
    app_state: &AppState,
    recording_id: u64,
    vad_threshold: f32,
) -> Option<StreamCommit> {
    prepare_stream_commit_from(
        app_state,
        recording_id,
        |params| app_state.vad_stream.snapshot(recording_id, params),
        |raw_start| audio_lifecycle::peek_dictation_samples_since(recording_id, raw_start),
        vad_threshold,
    )
}

/// [`prepare_stream_commit`] with its two reads (the VAD stream and the capture
/// buffer) passed in. A peek that is merely not current, because the recording
/// stopped or the supervisor was slow to answer, only skips this tick: it says
/// nothing about the audio, so it must not end reuse. Only a buffer that is
/// shorter than the committed offset (`Reset`) proves the audio is gone.
fn prepare_stream_commit_from(
    app_state: &AppState,
    recording_id: u64,
    snapshot: impl FnOnce(crate::vad_stream::SegmentParams) -> Option<crate::vad_stream::StreamSnapshot>,
    peek: impl FnOnce(usize) -> audio_lifecycle::PeekSince,
    vad_threshold: f32,
) -> Option<StreamCommit> {
    use crate::stream_reuse::{Plan, Reason};
    if !app_state
        .stream_reuse
        .lock_or_recover()
        .is_active(recording_id)
    {
        return None;
    }
    let fail = |reason| {
        app_state
            .stream_reuse
            .lock_or_recover()
            .fail(recording_id, reason);
    };
    let Some(snapshot) = snapshot(crate::vad_stream::SegmentParams::murmur(vad_threshold)) else {
        fail(Reason::VadNotIncremental);
        return None;
    };
    let plan = app_state
        .stream_reuse
        .lock_or_recover()
        .plan(recording_id, &snapshot);
    let Plan::Commit(plan) = plan else {
        return None;
    };
    let raw = match peek(plan.raw_start) {
        audio_lifecycle::PeekSince::Samples(raw) => raw,
        audio_lifecycle::PeekSince::NotCurrent => return None,
        audio_lifecycle::PeekSince::Reset => {
            fail(Reason::BufferReset);
            return None;
        }
    };
    match plan.piece(&raw) {
        Some(piece) => Some((plan, piece)),
        None => {
            fail(Reason::BufferReset);
            None
        }
    }
}

/// Record a decoded piece. Runs with the backend held, so ownership is
/// rechecked without waiting on any status lock; a piece that finishes after
/// the recording stopped or was superseded is dropped without touching state.
fn commit_stream_piece(
    app_state: &AppState,
    recording_id: u64,
    plan: crate::stream_reuse::CommitPlan,
    piece: Vec<f32>,
    decoded: Result<String, String>,
) {
    if !dictation_partial_is_current_if_uncontended(app_state, recording_id) {
        return;
    }
    let mut reuse = app_state.stream_reuse.lock_or_recover();
    match decoded {
        Ok(text) => {
            let _ = reuse.commit(recording_id, plan, piece, &text);
        }
        Err(_) => reuse.fail(recording_id, crate::stream_reuse::Reason::DecodeError),
    }
}

async fn decode_one_dictation_partial(
    app: &tauri::AppHandle,
    recording_id: u64,
    preview: WhisperPreview,
) -> bool {
    let (transcription, lease, retained_preview) = {
        let state = app.state::<State>();
        if !dictation_partial_is_current(&state.app_state, recording_id) {
            return false;
        }
        // The user has let go: start nothing that the final decode could queue
        // behind (a running Core ML call cannot be cancelled).
        if crate::hotkey_release::stop_gesture_pending(recording_id) {
            emit_dictation_partial_tick(recording_id, "stop_pending", 0);
            return true;
        }
        if !try_begin_dictation_partial(&state.app_state, recording_id) {
            emit_dictation_partial_tick(recording_id, "in_flight", 0);
            return true;
        }
        let lease = DictationPartialLease {
            app: app.clone(),
            recording_id,
        };
        let retained_preview = preview;
        let Some(context) = state.app_state.active_context(recording_id) else {
            emit_dictation_partial_tick(recording_id, "no_context", 0);
            return false;
        };
        (context.transcription.clone(), lease, retained_preview)
    };
    let whisper = transcriber::whisper::supports_live_preview(&transcription.model_name);
    let window_limit = if whisper {
        WHISPER_PARTIAL_WINDOW_SAMPLES
    } else {
        PARTIAL_WINDOW_SAMPLES
    };
    // Only the trailing display window is copied each tick, never the whole
    // recording; decode reuse reads just the audio of the piece it commits.
    let samples =
        audio_lifecycle::peek_dictation_samples(recording_id, window_limit).unwrap_or_default();
    let sample_count = samples.len();
    if sample_count < PARTIAL_MIN_SAMPLES {
        emit_dictation_partial_tick(recording_id, "too_short", sample_count);
        return true;
    }
    let window = dictation_partial_window(&samples).to_vec();
    drop(samples);
    let worker_app = app.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<Option<String>, &'static str> {
        // The blocking worker outlives cancellation of its awaiting task. Its
        // model locks and last preview Arc must drop before completion.
        let _lease = lease;
        let worker_preview = retained_preview;
        let state = worker_app.state::<State>();
        if !dictation_preview_may_start(&state.app_state, recording_id) {
            return Err("stopped");
        }
        let raw = if whisper {
            let mut preview = worker_preview.lock_or_recover();
            if !dictation_partial_is_current(&state.app_state, recording_id) {
                return Err("stopped");
            }
            if preview.is_none() {
                *preview =
                    transcriber::whisper::WhisperPreviewBackend::load(&transcription.model_name)
                        .ok();
            }
            if !dictation_partial_is_current(&state.app_state, recording_id) {
                return Err("stopped");
            }
            let deadline = std::time::Instant::now() + WHISPER_PARTIAL_DEADLINE;
            preview
                .as_mut()
                .ok_or("model_unavailable")?
                .transcribe(
                    &window,
                    &transcription.language,
                    transcription.prompt.as_deref(),
                    transcription.smart_punctuation,
                    || {
                        !dictation_partial_is_current(&state.app_state, recording_id)
                            || std::time::Instant::now() >= deadline
                    },
                )
                .map_err(|_| "decode_error")?
        } else {
            // Decode while speaking: decide, before taking the backend, whether
            // a piece of VAD-trimmed speech is ready to commit. This reads the
            // VAD stream and, for a piece, only that piece's audio.
            let mut reuse_commit = prepare_stream_commit(
                &state.app_state,
                recording_id,
                vad::threshold_for_sensitivity(transcription.vad_sensitivity),
            );
            let decoded = state
                .app_state
                .model_runtime
                .try_with_ready_backend(
                    &transcription.model_name,
                    || dictation_preview_may_start_if_uncontended(&state.app_state, recording_id),
                    |backend| {
                        let _preview_backend = crate::shadow_measurement::PreviewBackendLease::new(
                            &state.app_state.preview_holds_backend,
                        );
                        if let Some((plan, piece)) = reuse_commit.take() {
                            // Never hold reuse state across inference. Stop/cancel
                            // may clear it while this non-cancellable decode runs.
                            let decoded = backend.transcribe_preview(
                                &piece,
                                &transcription.language,
                                transcription.prompt.as_deref(),
                                transcription.smart_punctuation,
                                transcriber::PreviewPass::Dictation(recording_id),
                            );
                            commit_stream_piece(
                                &state.app_state,
                                recording_id,
                                plan,
                                piece,
                                decoded,
                            );
                        }
                        // The stop gesture may have started during the piece's
                        // decode: do not start the display decode behind it.
                        if !dictation_preview_may_start_if_uncontended(
                            &state.app_state,
                            recording_id,
                        ) {
                            return Ok(None);
                        }
                        backend
                            .transcribe_preview(
                                &window,
                                &transcription.language,
                                transcription.prompt.as_deref(),
                                transcription.smart_punctuation,
                                transcriber::PreviewPass::Dictation(recording_id),
                            )
                            .map(Some)
                    },
                )
                // A failed display decode leaves the committed pieces valid; a
                // failed piece decode already ended reuse in `commit_stream_piece`.
                .map_err(|_| "decode_error")?;
            match decoded {
                None => {
                    return Err(
                        if crate::hotkey_release::stop_gesture_pending(recording_id) {
                            "stopped"
                        } else {
                            "backend_unavailable"
                        },
                    )
                }
                // The stop gesture skipped the display decode: nothing to show.
                Some(None) => return Err("stopped"),
                Some(Some(raw)) => raw,
            }
        };
        let cleaned = raw.trim().to_string();
        if !cleaned.is_empty() {
            note_nonempty_dictation_preview(&state.app_state, recording_id);
        }
        Ok((!cleaned.is_empty()).then_some(cleaned))
    })
    .await
    .unwrap_or(Err("worker_failed"));
    let (text, no_text_outcome) = match result {
        Ok(text) => (text, "empty"),
        Err(outcome) => (None, outcome),
    };
    let state = app.state::<State>();
    if no_text_outcome == "worker_failed" {
        state
            .app_state
            .stream_reuse
            .lock_or_recover()
            .fail(recording_id, crate::stream_reuse::Reason::Superseded);
    }
    if let Some(text) = text {
        if dictation_partial_is_current(&state.app_state, recording_id) {
            let outcome = match app.emit_to(
                "dictation-preview",
                "dictation-partial",
                serde_json::json!({ "recordingId": recording_id, "text": text }),
            ) {
                Ok(()) => "emitted",
                Err(_) => "emit_failed",
            };
            emit_dictation_partial_tick(recording_id, outcome, sample_count);
        } else {
            emit_dictation_partial_tick(recording_id, "stale", sample_count);
        }
    } else {
        emit_dictation_partial_tick(recording_id, no_text_outcome, sample_count);
    }
    dictation_partial_is_current(&state.app_state, recording_id)
}

fn dictation_status_code(status: DictationStatus) -> &'static str {
    match status {
        DictationStatus::Idle => "idle",
        DictationStatus::Starting => "starting",
        DictationStatus::Recording => "recording",
        DictationStatus::Recovering => "recovering",
        DictationStatus::Processing => "processing",
    }
}

fn emit_dictation_state_changed(recording_id: u64, from: DictationStatus, to: DictationStatus) {
    if recording_id == 0 || from == to {
        return;
    }
    tracing::info!(
        target: "pipeline",
        event_code = "pipeline.dictation_state_changed",
        recording_id,
        from = dictation_status_code(from),
        to = dictation_status_code(to),
        "dictation state changed"
    );
}

/// Mutate the live dictation state and publish its bounded, content-free
/// transition evidence as one operation. Keep production status writes on this
/// seam so early returns cannot silently create gaps in the SLO timeline.
fn transition_dictation_status(
    recording_id: u64,
    dictation: &mut crate::state::DictationState,
    to: DictationStatus,
) -> DictationStatus {
    let from = dictation.status;
    dictation.status = to;
    emit_dictation_state_changed(recording_id, from, to);
    from
}

/// Mark an eligible live request at the exact Idle -> Starting boundary.
/// Capture the performance origin and publish the denominator before the
/// state-transition trace so diagnostic I/O cannot disappear from measured
/// request-to-first-PCM latency.
fn accept_live_dictation_request(
    recording_id: u64,
    dictation: &mut crate::state::DictationState,
    emit_requested: impl FnOnce(DictationStatus),
) -> i64 {
    let performance_started_at_ms = chrono::Utc::now().timestamp_millis();
    emit_requested(dictation.status);
    transition_dictation_status(recording_id, dictation, DictationStatus::Starting);
    performance_started_at_ms
}

#[derive(Clone, Copy)]
enum DictationPresentation {
    CleanupInProgress,
    InitializationFailed(audio::AudioFailureKind),
    CleanupStalled,
    Interrupted { partial_transcription: bool },
}

impl DictationPresentation {
    fn codes(self) -> (&'static str, &'static str) {
        match self {
            Self::CleanupInProgress => ("microphone_cleanup_in_progress", "wait"),
            Self::InitializationFailed(audio::AudioFailureKind::PermissionDenied) => (
                "microphone_initialization_failed",
                "open_microphone_settings",
            ),
            Self::InitializationFailed(audio::AudioFailureKind::DeviceUnavailable) => {
                ("microphone_initialization_failed", "choose_microphone")
            }
            Self::InitializationFailed(_) => ("microphone_initialization_failed", "retry"),
            Self::CleanupStalled => ("microphone_cleanup_stalled", "restart_app"),
            Self::Interrupted {
                partial_transcription: true,
            } => ("microphone_interrupted", "wait_for_partial_transcription"),
            Self::Interrupted {
                partial_transcription: false,
            } => ("microphone_interrupted", "retry"),
        }
    }
}

fn emit_dictation_presentation(recording_id: u64, presentation: DictationPresentation) {
    if recording_id == 0 {
        return;
    }
    let (status_code, action_code) = presentation.codes();
    tracing::info!(
        target: "pipeline",
        event_code = "pipeline.dictation_presentation",
        recording_id,
        status_code,
        action_code,
        "dictation status presented"
    );
}

/// Scan `folder` for code identifiers and build the cached folder-scan prompt
/// string, ranked by descending frequency and capped at [`CORRECTION_TERMS`].
/// The cache holds the larger (top-500) list; the Whisper path takes its
/// [`WHISPER_PROMPT_TERMS`] rank-prefix at use time (see [`whisper_prefix`]).
/// Empty/blank folder returns an empty string. Delegates to the guarded,
/// pure-logic `vocab` module (file count / size caps live there).
fn build_code_vocab_prompt(folder: &str) -> String {
    let folder = folder.trim();
    if folder.is_empty() {
        return String::new();
    }
    let prompt =
        crate::vocab::build_vocab_prompt_from_dir(std::path::Path::new(folder), CORRECTION_TERMS);
    tracing::info!(
        target: "pipeline",
        prompt_chars = prompt.len(),
        "code vocabulary scan completed"
    );
    prompt
}

/// Take the top [`WHISPER_PROMPT_TERMS`] space-separated terms of a ranked
/// folder-scan prompt. The cached prompt holds the top-[`CORRECTION_TERMS`] list
/// ranked by descending frequency, so its first 96 words ARE the top-96 terms.
/// This is what keeps the Whisper budget decoupled from the (larger) correction
/// budget without a second walk or a second cache field.
fn whisper_prefix(prompt: &str) -> String {
    prompt
        .split_whitespace()
        .take(WHISPER_PROMPT_TERMS)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Return the code-aware vocabulary prompt for the current settings, or an empty
/// string when the feature is disabled.
///
/// When enabled, the built-in dev-term dictionary always contributes so the
/// feature works with no folder selected ("just works"). A configured project
/// folder is scanned (and cached) and its identifiers are placed *first* — they're
/// more specific than the generic built-ins, so they survive Whisper's prompt
/// truncation. Folder scanning still happens at most once per folder/enable change.
fn resolve_code_vocab_prompt(app_state: &AppState, bundle_id: Option<&str>) -> String {
    // Fast path under the lock: feature off => nothing to do.
    let (enabled, folder, cached) = {
        let dictation = app_state.dictation.lock_or_recover();
        (
            dictation.code_vocab_enabled && dictation.code_vocabulary_enabled_for(bundle_id),
            dictation.code_vocab_folder.clone(),
            dictation.code_vocab_prompt.clone(),
        )
    };
    if !enabled {
        return String::new();
    }

    let builtin = crate::vocab::builtin_terms_prompt();

    // Optional project scan layered on top of the built-ins. The cache holds the
    // top-CORRECTION_TERMS (500) ranked list; Whisper's token-bound prompt takes
    // only its top-WHISPER_PROMPT_TERMS (96) rank-prefix here.
    let folder_prompt = if folder.trim().is_empty() {
        String::new()
    } else if let Some(prompt) = cached {
        whisper_prefix(&prompt)
    } else {
        // Not yet scanned: build now and cache the full (top-500) list (only if
        // settings still match), then feed Whisper its top-96 prefix.
        let prompt = build_code_vocab_prompt(&folder);
        let mut dictation = app_state.dictation.lock_or_recover();
        if dictation.code_vocab_enabled && dictation.code_vocab_folder == folder {
            dictation.code_vocab_prompt = Some(prompt.clone());
            // Refresh the correction matcher so Smart Correction picks up this
            // folder's top-500 terms now, not just on the next settings change.
            // configure_dictation no longer prebuilds the prompt, so on the common
            // restart-with-persisted-folder path the matcher would otherwise stay
            // builtin-only for the whole session (it's rebuilt only here, in
            // configure_dictation, and in scan_code_vocab — and the transcription
            // path hits none of the other two). rebuild_correction_matcher writes a
            // separate leaf mutex, so there's no deadlock with the held guard.
            rebuild_correction_matcher(app_state, &dictation);
            app_state.bump_settings_revision();
        }
        whisper_prefix(&prompt)
    };

    // Project identifiers first (most specific), built-in dictionary after.
    if folder_prompt.trim().is_empty() {
        builtin
    } else {
        format!("{} {}", folder_prompt.trim(), builtin)
    }
}

/// Build the code-vocabulary prompt represented by an already-locked settings
/// generation. `None` means the folder cache is not ready for this generation.
fn cached_code_vocab_prompt(
    dictation: &crate::state::DictationState,
    bundle_id: Option<&str>,
) -> Option<String> {
    let enabled_for_context =
        dictation.code_vocab_enabled && dictation.code_vocabulary_enabled_for(bundle_id);
    if !enabled_for_context {
        return Some(String::new());
    }
    let builtin = crate::vocab::builtin_terms_prompt();
    if dictation.code_vocab_folder.trim().is_empty() {
        return Some(builtin);
    }
    dictation.code_vocab_prompt.as_ref().map(|prompt| {
        let folder_prompt = whisper_prefix(prompt);
        if folder_prompt.trim().is_empty() {
            builtin
        } else {
            format!("{} {}", folder_prompt.trim(), builtin)
        }
    })
}

/// Resolve one immutable context from a consistent settings/vocabulary
/// generation. A concurrent settings change causes a retry rather than a mixed
/// snapshot.
pub(crate) fn resolve_live_context(
    app_state: &AppState,
    app_identity: &crate::frontmost::FrontmostAppIdentity,
    delivery_target: &crate::frontmost::DeliveryTargetSnapshot,
) -> Arc<DictationContextSnapshot> {
    let bundle_id = app_identity.bundle_id.as_deref();
    loop {
        let code_vocab = resolve_code_vocab_prompt(app_state, bundle_id);
        let dictation = app_state.dictation.lock_or_recover();
        let Some(current_code_vocab) = cached_code_vocab_prompt(&dictation, bundle_id) else {
            continue;
        };
        if current_code_vocab != code_vocab {
            continue;
        }
        let sanitized = crate::vocabulary_alias::prompt_terms(
            &dictation.vocabulary_entries,
            bundle_id,
            &dictation.app_profiles,
        )
        .replace('\0', "");
        let prompt = combine_prompts(&sanitized, &code_vocab);
        let correction_matcher = app_state
            .correction_matcher
            .lock_or_recover()
            .as_ref()
            .map(|matchers| matchers.select(bundle_id));
        let ide_context_index = bundle_id.and_then(|bundle_id| {
            dictation
                .app_profiles
                .iter()
                .find(|profile| profile.bundle_id == bundle_id)
                .filter(|profile| profile.ide_context_enabled)
                .and_then(|profile| {
                    app_state
                        .ide_context
                        .lock_or_recover()
                        .snapshot(bundle_id, &profile.ide_project_roots)
                })
        });
        let vocabulary_version = app_state.settings_revision.load(Ordering::SeqCst);
        let mut context = dictation_context::resolve(ResolverInputs {
            bundle_id,
            global: &dictation,
            prompt,
            correction_matcher,
            ide_context_index,
            vocabulary_version,
        });
        context.app.process_id = app_identity.process_id;
        context.app.delivery_target = delivery_target.clone();
        return Arc::new(context);
    }
}

/// Routing state selected once for an accepted recording generation.
///
/// A smart destination may point at a different application than the one that
/// is temporarily frontmost (for example, an editor retained across a media
/// interaction). The profile is resolved for the intended destination.
struct RecordingDestination {
    target: crate::frontmost::DeliveryTargetSnapshot,
    smart_destination: crate::smart_destination::Snapshot,
}

fn select_recording_destination(
    accepted_target: &crate::frontmost::DeliveryTargetSnapshot,
    smart_destination: crate::smart_destination::Snapshot,
) -> RecordingDestination {
    let target = smart_destination
        .identity()
        .cloned()
        .unwrap_or_else(|| accepted_target.clone());
    RecordingDestination {
        target,
        smart_destination,
    }
}

/// Resolve the profile from the intended delivery target and bind the opaque
/// smart capability into that same immutable snapshot. Callers cannot splice a
/// profile resolved for one app together with routing state for another.
fn resolve_recording_context(
    app_state: &AppState,
    destination: RecordingDestination,
) -> Arc<DictationContextSnapshot> {
    let app_identity = crate::frontmost::profile_identity_from_delivery_target(&destination.target);
    let mut context = resolve_live_context(app_state, &app_identity, &destination.target);
    Arc::make_mut(&mut context).app.smart_destination = destination.smart_destination;
    context
}

/// Merge the manual custom-vocabulary string and the code-aware vocabulary into a
/// single Whisper initial prompt. Returns `None` when both are blank so the
/// backend skips setting a prompt entirely.
///
/// `code` carries the folder-scan terms ahead of the built-in dictionary (see
/// [`resolve_code_vocab_prompt`]). The hand-typed `custom` list is concatenated
/// FIRST so user-entered terms keep precedence: Whisper truncates the initial
/// prompt to ~224 tokens keeping the START, so a large code-scan prompt must not
/// crowd out terms the user explicitly typed. Order is therefore custom, then
/// folder, then builtin — and the whole thing is deduped case-insensitively so the
/// same term never burns two slots of the budget (the Smart Correction matcher
/// dedupes separately).
fn combine_prompts(custom: &str, code: &str) -> Option<String> {
    let custom = custom.trim();
    let code = code.trim();
    match (custom.is_empty(), code.is_empty()) {
        (true, true) => None,
        (false, true) => Some(dedupe_prompt_terms(custom)),
        (true, false) => Some(dedupe_prompt_terms(code)),
        (false, false) => Some(dedupe_prompt_terms(&format!("{} {}", custom, code))),
    }
}

/// Collapse a space-joined prompt to its first occurrence of each term,
/// case-insensitively, preserving order and the first surface form. Used at the
/// combine step so cross-source repeats (folder scan vs. built-in dictionary vs.
/// custom vocabulary) don't waste Whisper's limited prompt budget.
fn dedupe_prompt_terms(prompt: &str) -> String {
    let mut seen = std::collections::HashSet::new();
    prompt
        .split_whitespace()
        .filter(|t| seen.insert(t.to_ascii_lowercase()))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Split the free-text custom-vocabulary field into individual written terms.
/// Entries are separated by commas or newlines (not spaces) so multi-word terms
/// like "API Gateway" survive as one entry. Blank entries are dropped.
fn parse_vocab_terms(s: &str) -> Vec<String> {
    s.split([',', '\n', '\r'])
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .map(String::from)
        .collect()
}

/// Rebuild the post-model correction matcher from the current dictation settings
/// and store it in `app_state.correction_matcher`. Called on settings-change (in
/// `configure_dictation`) — never per-utterance. `dictation` is the already-held
/// lock guard; the matcher is stored under a separate leaf mutex.
pub(crate) fn rebuild_correction_matcher(
    app_state: &AppState,
    dictation: &crate::state::DictationState,
) {
    let code_enabled = dictation.code_vocab_enabled;
    let mut terms = Vec::new();
    if code_enabled {
        // Built-in dev dictionary + any cached project scan become correction
        // terms (their spoken forms are auto-derived: "useEffect" -> "use effect").
        for t in crate::vocab::builtin_terms_prompt().split_whitespace() {
            terms.push(t.to_string());
        }
        // The cached folder prompt holds the top-CORRECTION_TERMS (500) ranked
        // identifiers. Unlike the Whisper prompt path (which takes only the top-96
        // rank-prefix because it's token-bound), Smart Correction has no token
        // budget, so it consumes the full cached list — a big recall win.
        if let Some(folder_prompt) = &dictation.code_vocab_prompt {
            for t in folder_prompt.split_whitespace() {
                terms.push(t.to_string());
            }
        }
    }
    let matcher = crate::vocabulary_alias::CorrectionMatcherSet::build_with_contextual_code_terms(
        &terms,
        &dictation.vocabulary_entries,
        &dictation.app_profiles,
        dictation.correction_fuzzy,
        // Gate the std* abbrev builtins on the dev-context (code-vocab) signal.
        code_enabled,
    );
    *app_state.correction_matcher.lock_or_recover() = Some(std::sync::Arc::new(matcher));
}

/// RAII guard that resets dictation status to Idle on drop,
/// ensuring status is restored on any early return or error path.
///
/// Tracks `recording_id` so it only resets state if this recording is still
/// the active one — prevents a stale pipeline from clobbering a new recording
/// that started after an Escape cancel.
struct IdleGuard<'a> {
    app_state: &'a AppState,
    recording_id: u64,
    disarmed: bool,
}

impl<'a> IdleGuard<'a> {
    fn new(app_state: &'a AppState, recording_id: u64) -> Self {
        Self {
            app_state,
            recording_id,
            disarmed: false,
        }
    }

    fn disarm(&mut self) {
        self.disarmed = true;
    }
}

impl IdleGuard<'_> {
    fn finish(&mut self, on_idle: impl FnOnce(&crate::state::DictationState)) {
        if !self.disarmed {
            self.disarmed = true;
            self.app_state.clear_active_context(self.recording_id);
            // Lock first, then check recording_id — prevents TOCTOU where
            // a concurrent start advances the ID between our check and the
            // status write.
            let mut dictation = self.app_state.dictation.lock_or_recover();
            let current_rid = self.app_state.recording_id.load(Ordering::SeqCst);
            if current_rid != self.recording_id {
                return;
            }
            transition_dictation_status(self.recording_id, &mut dictation, DictationStatus::Idle);
            keyboard::set_processing(false);
            on_idle(&dictation);
        }
    }
}

impl Drop for IdleGuard<'_> {
    fn drop(&mut self) {
        self.finish(|_| {});
    }
}

/// RAII guard that clears `file_transcribing` on drop, so a file transcription
/// releases its mutual-exclusion claim on every return path (early errors, `?`,
/// or success).
struct FileTranscribeGuard<'a> {
    app_state: &'a AppState,
    app_handle: Option<tauri::AppHandle>,
}

impl Drop for FileTranscribeGuard<'_> {
    fn drop(&mut self) {
        self.app_state
            .file_transcribing
            .store(false, Ordering::SeqCst);
        if let Some(app_handle) = &self.app_handle {
            let _ = app_handle.emit("file-transcription-status-changed", false);
        }
    }
}

struct SharedBackendChangeGuard(
    Arc<crate::benchmark::BenchmarkCoordinator>,
    Option<tauri::AppHandle>,
);

impl Drop for SharedBackendChangeGuard {
    fn drop(&mut self) {
        self.0.finish_shared_backend_change();
        if let Some(app) = &self.1 {
            crate::meeting_suggestions::busy_changed(app);
        }
    }
}

fn status_allows_model_preparation(status: DictationStatus) -> bool {
    matches!(
        status,
        DictationStatus::Starting | DictationStatus::Recording
    )
}

/// Start loading the selected model as soon as microphone initialization is
/// accepted. Preparation is mic-independent, so it overlaps both Core Audio
/// startup and the user's speech. The normal pipeline still calls `load_model`,
/// so it either observes a hit or waits on this same backend lock when a very
/// short recording ends before preparation completes.
fn spawn_model_preparation(app_handle: tauri::AppHandle, model_name: String, recording_id: u64) {
    let queued_at = std::time::Instant::now();
    drop(tauri::async_runtime::spawn_blocking(move || {
        let queue_ms = queued_at.elapsed().as_millis() as u64;
        let state = app_handle.state::<State>();
        let is_active = {
            let dictation = state.app_state.dictation.lock_or_recover();
            state.app_state.recording_id.load(Ordering::SeqCst) == recording_id
                && status_allows_model_preparation(dictation.status)
                && state
                    .app_state
                    .active_context(recording_id)
                    .is_some_and(|context| context.transcription.model_name == model_name)
        };
        if !is_active {
            return;
        }

        // The recording may have been cancelled while this worker waited for a
        // previous inference or model switch to release the backend.
        let is_still_active = {
            let dictation = state.app_state.dictation.lock_or_recover();
            state.app_state.recording_id.load(Ordering::SeqCst) == recording_id
                && status_allows_model_preparation(dictation.status)
                && state
                    .app_state
                    .active_context(recording_id)
                    .is_some_and(|context| context.transcription.model_name == model_name)
        };
        if !is_still_active {
            return;
        }

        let rss_before_mb = crate::resource_monitor::get_process_rss_mb();
        let result = state.app_state.model_runtime.prepare(
            Some(&app_handle),
            &model_name,
            PreparationReason::Recording,
        );
        let rss_after_mb = crate::resource_monitor::get_process_rss_mb();
        let total_ms = queued_at.elapsed().as_millis() as u64;
        match result {
            Ok(report) => {
                let correlation = RunCorrelationV1::Dictation { recording_id };
                let _ = state.performance.update_active(&correlation, |active| {
                    active.runtimes = runtime_identity(
                        &model_name,
                        if report.cache_hit {
                            ModelWarmStateV1::Warm
                        } else {
                            ModelWarmStateV1::ColdLoaded
                        },
                    );
                    active.stages.retain(|stage| {
                        !matches!(
                            stage.stage,
                            PerformanceStageV1::ModelQueue | PerformanceStageV1::ModelLoad
                        )
                    });
                    active.stages.push(StageTimingV1::measured(
                        PerformanceStageV1::ModelQueue,
                        queue_ms.saturating_add(report.lock_wait_ms),
                    ));
                    active.stages.push(StageTimingV1::measured(
                        PerformanceStageV1::ModelLoad,
                        report.load_ms,
                    ));
                });
                tracing::info!(
                    target: "pipeline",
                    recording_id,
                    model = model_name.as_str(),
                    backend = model_runtime::model_definition(&model_name).map(|model| model.backend.as_str()).unwrap_or("unknown"),
                    cache_hit = report.cache_hit,
                    queue_ms,
                    lock_wait_ms = report.lock_wait_ms,
                    load_ms = report.load_ms,
                    total_ms,
                    rss_before_mb,
                    rss_after_mb,
                    "model_prepare_complete"
                )
            }
            Err(_error) => tracing::warn!(
                target: "pipeline",
                recording_id,
                model = model_name.as_str(),
                total_ms,
                failed = true,
                "model_prepare_failed"
            ),
        }
    }));
}

/// Warm an installed Core ML model after startup configuration. A newly linked
/// application binary can trigger one-time ANE specialization even when the
/// compiled model cache already exists. Starting it while the app is idle keeps
/// that cost away from the user's first short dictation; recording-start
/// preparation remains the fallback if recording begins first.
fn spawn_idle_model_preparation(
    app_handle: tauri::AppHandle,
    model_name: String,
    change_guard: SharedBackendChangeGuard,
) {
    let queued_at = std::time::Instant::now();
    drop(tauri::async_runtime::spawn_blocking(move || {
        let _change_guard = change_guard;
        let state = app_handle.state::<State>();
        let is_current = {
            let dictation = state.app_state.dictation.lock_or_recover();
            dictation.status == DictationStatus::Idle && dictation.model_name == model_name
        };
        if !is_current {
            return;
        }

        let is_still_current = {
            let dictation = state.app_state.dictation.lock_or_recover();
            dictation.status == DictationStatus::Idle && dictation.model_name == model_name
        };
        if !is_still_current {
            return;
        }

        let result = state.app_state.model_runtime.prepare(
            Some(&app_handle),
            &model_name,
            PreparationReason::StartupWarm,
        );
        let total_ms = queued_at.elapsed().as_millis() as u64;
        match result {
            Ok(report) => tracing::info!(
                target: "pipeline",
                model = model_name.as_str(),
                backend = model_runtime::model_definition(&model_name).map(|model| model.backend.as_str()).unwrap_or("unknown"),
                cache_hit = report.cache_hit,
                lock_wait_ms = report.lock_wait_ms,
                load_ms = report.load_ms,
                total_ms,
                reason = "startup_configuration",
                "model_prepare_complete"
            ),
            Err(_error) => tracing::info!(
                target: "pipeline",
                model = model_name.as_str(),
                total_ms,
                reason = "startup_configuration",
                failed = true,
                "model_prepare_skipped"
            ),
        }
    }));
}

#[derive(Default)]
pub(crate) struct PipelineTimings {
    pub vad_ms: u64,
    pub model_queue_ms: u64,
    pub model_load_ms: u64,
    pub decode_ms: u64,
    pub inference_ms: u64,
    pub transform_ms: u64,
    pub transform_stages: Vec<StageTimingV1>,
    pub correction_ms: u64,
    pub file_output_ms: u64,
    pub paste_ms: u64,
    pub rss_before_mb: u64,
    pub rss_after_mb: u64,
    pub warm_state: Option<ModelWarmStateV1>,
    /// Clipboard write (and paste, when enabled) confirmed; the
    /// `releaseToPaste` endpoint exists only then (#826).
    pub delivery_confirmed: bool,
    /// Sub-millisecond marks between the named stages (A4).
    pub gaps: StageGaps,
}

/// Content-free microsecond marks along the stop path, so the time between
/// the named stages and `totalProcessing` is accounted for. The segments are
/// disjoint and ordered; `pipeline.dictation_stage_gaps` reports each and the
/// remainder (`unattributed_us`) against the exact total.
#[derive(Default, Clone, Copy)]
pub(crate) struct StageGaps {
    /// Set when `run_transcription_pipeline` starts and when it returns.
    pub entered_at: Option<std::time::Instant>,
    pub ended_at: Option<std::time::Instant>,
    /// Entry to the VAD stage: signal level logging and the cancel check.
    pub prologue_us: u64,
    /// The whole VAD stage, including the `spawn_blocking` hop.
    pub vad_us: u64,
    /// Inside `vad_us`: time the blocking pool took to start the VAD closure.
    pub vad_blocking_queue_us: u64,
    /// VAD end to the backend request: memory read and stage bookkeeping.
    pub pre_backend_us: u64,
    /// The backend request to the start of decode: diarization reservation,
    /// lock wait, and any model load.
    pub backend_acquire_us: u64,
    /// Inside `backend_acquire_us`, at millisecond resolution.
    pub backend_model_queue_us: u64,
    pub backend_model_load_us: u64,
    pub backend_decode_us: u64,
    /// End of decode to the backend request returning: lease release.
    pub backend_release_us: u64,
    /// Backend return to the start of the transform stage.
    pub transform_setup_us: u64,
    pub transform_us: u64,
    /// Transform end to the start of delivery: history, file output.
    pub post_transform_us: u64,
    /// Delivery: clipboard write, paste, and their bookkeeping.
    pub delivery_us: u64,
}

fn micros(started: std::time::Instant) -> u64 {
    u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX)
}

fn micros_between(earlier: std::time::Instant, later: std::time::Instant) -> u64 {
    u64::try_from(later.saturating_duration_since(earlier).as_micros()).unwrap_or(u64::MAX)
}

/// What the stop command measured itself, around the pipeline.
struct StopPathMarks {
    t_total: std::time::Instant,
    capture_finalization_us: u64,
    handoff_started_at: std::time::Instant,
    pipeline_returned_at: std::time::Instant,
    idle_status_emit_us: u64,
    idle_block_ended_at: std::time::Instant,
    delivered_at: std::time::Instant,
}

fn emit_dictation_stage_gaps(recording_id: u64, gaps: &StageGaps, marks: &StopPathMarks) {
    let (Some(entered_at), Some(ended_at)) = (gaps.entered_at, gaps.ended_at) else {
        return;
    };
    let total_us = micros_between(marks.t_total, marks.delivered_at);
    let handoff_us = micros_between(marks.handoff_started_at, entered_at);
    let pipeline_return_us = micros_between(ended_at, marks.pipeline_returned_at);
    let tail_us = micros_between(marks.idle_block_ended_at, marks.delivered_at);
    let accounted_us = [
        marks.capture_finalization_us,
        handoff_us,
        gaps.prologue_us,
        gaps.vad_us,
        gaps.pre_backend_us,
        gaps.backend_acquire_us,
        gaps.backend_decode_us,
        gaps.backend_release_us,
        gaps.transform_setup_us,
        gaps.transform_us,
        gaps.post_transform_us,
        gaps.delivery_us,
        pipeline_return_us,
        marks.idle_status_emit_us,
        tail_us,
    ]
    .iter()
    .fold(0u64, |sum, part| sum.saturating_add(*part));
    tracing::info!(
        target: "pipeline",
        event_code = "pipeline.dictation_stage_gaps",
        recording_id,
        total_us,
        unattributed_us = total_us.saturating_sub(accounted_us),
        capture_finalization_us = marks.capture_finalization_us,
        handoff_us,
        pipeline_prologue_us = gaps.prologue_us,
        vad_us = gaps.vad_us,
        vad_blocking_queue_us = gaps.vad_blocking_queue_us,
        pre_backend_us = gaps.pre_backend_us,
        backend_acquire_us = gaps.backend_acquire_us,
        backend_model_queue_us = gaps.backend_model_queue_us,
        backend_model_load_us = gaps.backend_model_load_us,
        backend_decode_us = gaps.backend_decode_us,
        backend_release_us = gaps.backend_release_us,
        transform_setup_us = gaps.transform_setup_us,
        transform_us = gaps.transform_us,
        post_transform_us = gaps.post_transform_us,
        delivery_us = gaps.delivery_us,
        pipeline_return_us,
        idle_status_emit_us = marks.idle_status_emit_us,
        tail_us,
        "dictation stop-path stage gaps"
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PipelineTerminal {
    Success,
    NoSpeech(DictationErrorCode),
    Cancelled(PerformanceStageV1),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DictationDeliveryOutcome {
    ClipboardOnly,
    AutoPastePosted,
    ClipboardWriteFailed,
    Unconfirmed,
}

impl DictationDeliveryOutcome {
    fn as_event_value(self) -> &'static str {
        match self {
            Self::ClipboardOnly => "clipboardOnly",
            Self::AutoPastePosted => "autoPastePosted",
            Self::ClipboardWriteFailed => "clipboardWriteFailed",
            Self::Unconfirmed => "unconfirmed",
        }
    }
}

fn confirmed_delivery_outcome(outcome: injector::InjectionOutcome) -> DictationDeliveryOutcome {
    match outcome {
        injector::InjectionOutcome::NoText | injector::InjectionOutcome::Stopped(_) => {
            DictationDeliveryOutcome::Unconfirmed
        }
        injector::InjectionOutcome::ClipboardOnly(
            injector::ClipboardOnlyReason::ClipboardChanged,
        ) => DictationDeliveryOutcome::Unconfirmed,
        injector::InjectionOutcome::ClipboardOnly(_) => DictationDeliveryOutcome::ClipboardOnly,
        injector::InjectionOutcome::AutoPasted => DictationDeliveryOutcome::AutoPastePosted,
    }
}

/// Clipboard-only and posted auto-paste are confirmed deliveries; a failed
/// clipboard write, timeout, or stopped delivery is not.
fn delivery_confirms_paste(outcome: DictationDeliveryOutcome) -> bool {
    matches!(
        outcome,
        DictationDeliveryOutcome::ClipboardOnly | DictationDeliveryOutcome::AutoPastePosted
    )
}

/// `releaseToPaste` exists only for a hotkey-released, successful run whose
/// delivery was confirmed (#826).
fn release_to_paste_stage(
    outcome: &RunOutcomeV1,
    delivery_confirmed: bool,
    release: Option<crate::hotkey_release::HotkeyRelease>,
    delivered_at: std::time::Instant,
) -> Option<StageTimingV1> {
    let release = release.filter(|_| delivery_confirmed)?;
    matches!(outcome, RunOutcomeV1::Success).then(|| release.release_to_paste(delivered_at))
}

fn delivery_event_recording_id(
    recording_id: u64,
    current_recording_id: u64,
    cancelled: bool,
    status: DictationStatus,
) -> Option<u64> {
    if recording_id == current_recording_id && !cancelled && status == DictationStatus::Processing {
        Some(recording_id)
    } else {
        None
    }
}

fn recording_capture_is_current(
    recording_id: u64,
    current_recording_id: u64,
    cancelled: bool,
    status: DictationStatus,
) -> bool {
    recording_id == current_recording_id
        && !cancelled
        && matches!(
            status,
            DictationStatus::Starting
                | DictationStatus::Recording
                | DictationStatus::Recovering
                | DictationStatus::Processing
        )
}

/// A stored preference may be paused by global delivery settings. Use the
/// same gate for tracking and accepted recordings so per-app overrides retain
/// the ordinary paste path while Smart destination is unavailable.
fn smart_destination_eligible(dictation: &crate::state::DictationState) -> bool {
    dictation.smart_destination_enabled
        && dictation.auto_paste
        && !dictation.save_audio
        && !dictation.save_transcript
}

fn smart_destination_status(
    smart_destination: &crate::smart_destination::Snapshot,
    accepted_target: &crate::frontmost::DeliveryTargetSnapshot,
) -> &'static str {
    match smart_destination.identity() {
        Some(target)
            if crate::frontmost::profile_identity_from_delivery_target(target)
                == crate::frontmost::profile_identity_from_delivery_target(accepted_target) =>
        {
            "current"
        }
        Some(_) => "recent",
        // Normal auto-paste goes to the verified app at stop, which may not be
        // the accepted app, so this status is presented without an app name.
        None if matches!(
            smart_destination,
            crate::smart_destination::Snapshot::Unverified
        ) =>
        {
            "current"
        }
        None => "unavailable",
    }
}

/// Presentation uses only the accepted delivery policy and an application name,
/// never a document title, input value, or selected text.
fn emit_smart_destination(
    app: &tauri::AppHandle,
    app_state: &AppState,
    rid: u64,
    context: &DictationContextSnapshot,
    accepted_target: &crate::frontmost::DeliveryTargetSnapshot,
) {
    let dictation = app_state.dictation.lock_or_recover();
    if !recording_capture_is_current(
        rid,
        app_state.recording_id.load(Ordering::SeqCst),
        app_state.is_cancelled(rid),
        dictation.status,
    ) {
        return;
    }
    let eligible = context.delivery.auto_paste
        && !context.delivery.save_audio
        && !context.delivery.save_transcript;
    let status = if eligible {
        smart_destination_status(&context.app.smart_destination, accepted_target)
    } else {
        "unavailable"
    };
    let app_name = if status == "unavailable"
        || matches!(
            context.app.smart_destination,
            crate::smart_destination::Snapshot::Unverified
        ) {
        None
    } else {
        destination_application_name(&context.app.delivery_target)
    };
    let _ = app.emit(
        "smart-destination-selected",
        serde_json::json!({
            "recordingId": rid, "status": status, "appName": app_name,
        }),
    );
}

#[cfg(target_os = "macos")]
fn destination_application_name(
    target: &crate::frontmost::DeliveryTargetSnapshot,
) -> Option<String> {
    let crate::frontmost::DeliveryTargetSnapshot::Complete(identity) = target else {
        return None;
    };
    objc2::rc::autoreleasepool(|_| {
        let app = objc2_app_kit::NSRunningApplication::runningApplicationWithProcessIdentifier(
            identity.process_id,
        )?;
        if app.bundleIdentifier()?.to_string() != identity.bundle_id {
            return None;
        }
        let name = app.localizedName()?.to_string();
        let name = name.trim();
        (!name.is_empty() && name.chars().count() <= 80 && !name.chars().any(char::is_control))
            .then(|| name.to_string())
    })
}

#[cfg(not(target_os = "macos"))]
fn destination_application_name(
    _target: &crate::frontmost::DeliveryTargetSnapshot,
) -> Option<String> {
    None
}

/// Run a delivery side effect only while this recording still owns the
/// Processing state. The dictation lock stays held through `operation`, making
/// cancellation/supersession atomic with the clipboard write and optional
/// paste that the main-thread callback performs.
fn with_current_delivery_owner<T>(
    app_state: &AppState,
    recording_id: u64,
    operation: impl FnOnce() -> T,
) -> Option<T> {
    let dictation = app_state.dictation.lock_or_recover();
    delivery_event_recording_id(
        recording_id,
        app_state.recording_id.load(Ordering::SeqCst),
        app_state.is_cancelled(recording_id),
        dictation.status,
    )?;
    Some(operation())
}

fn with_current_delivery_owner_timed<T>(
    app_state: &AppState,
    recording_id: u64,
    preparing: bool,
    operation: impl FnOnce() -> T,
) -> Option<T> {
    let wait = dt::Timer::new(if preparing {
        |r| &mut r.prepare_owner_wait_us
    } else {
        |r| &mut r.inject_owner_wait_us
    });
    let dictation = app_state.dictation.lock_or_recover();
    drop(wait);
    delivery_event_recording_id(
        recording_id,
        app_state.recording_id.load(Ordering::SeqCst),
        app_state.is_cancelled(recording_id),
        dictation.status,
    )?;
    Some(operation())
}

/// Run a generation-owned best-effort side effect even after Processing has
/// completed. Holding the dictation lock through the operation prevents a new
/// recording from advancing the generation between the ownership check and a
/// local caption publication. Blocking preparation happens before this fence;
/// the guarded operation is only an atomic rename or one published-file delete.
fn with_current_recording_generation<T>(
    app_state: &AppState,
    recording_id: u64,
    operation: impl FnOnce() -> T,
) -> Option<T> {
    let _dictation = app_state.dictation.lock_or_recover();
    if app_state.recording_id.load(Ordering::SeqCst) != recording_id
        || app_state.is_cancelled(recording_id)
    {
        return None;
    }
    Some(operation())
}

fn emit_auto_paste_failure_if_current(
    app_handle: &tauri::AppHandle,
    app_state: &AppState,
    recording_id: u64,
    message: &'static str,
) {
    let _ = with_current_delivery_owner(app_state, recording_id, || {
        app_handle.emit("auto-paste-failed", message)
    });
}

/// Route after a current-app or media smart preparation could not proceed.
///
/// A current-app capture falls back to the normal verified paste. That paste
/// keeps the anchored delivery target (the complete stop sample when one
/// exists), exactly as with Smart destination off: a user who moved to another
/// app while speaking and stopped there gets the paste in that app instead of
/// a refusal against the start app. A media-handoff capture, a refused handoff
/// to an app that holds no text input or a secure field, or a preparation
/// whose reply never arrived stays clipboard-only because normal paste could
/// target the wrong app or field.
fn smart_prepare_fallback(
    smart: &crate::smart_destination::Snapshot,
    refused_without_activation: bool,
    anchored_target: &crate::frontmost::DeliveryTargetSnapshot,
) -> (
    crate::smart_destination::Snapshot,
    crate::frontmost::DeliveryTargetSnapshot,
) {
    let route = if smart.allows_same_app_fallback() && refused_without_activation {
        crate::smart_destination::Snapshot::Unverified
    } else {
        crate::smart_destination::Snapshot::Unavailable
    };
    (route, anchored_target.clone())
}

fn emit_delivery_target_verification(
    recording_id: u64,
    anchor: crate::frontmost::DeliveryAnchor,
    stop_sample: crate::frontmost::StopSampleStatus,
    evidence: crate::frontmost::VerificationEvidence,
) {
    tracing::info!(
        target: "pipeline",
        event_code = "pipeline.delivery_target_verified",
        recording_id,
        anchor_code = anchor.as_str(),
        stop_sample_code = stop_sample.as_str(),
        outcome_code = evidence.outcome.as_str(),
        source_code = evidence.source.as_str(),
        retry_count = evidence.retry_count,
        elapsed_ms = evidence.elapsed_ms,
        same_application = evidence.same_application,
        same_process = evidence.same_process,
        same_process_instance = evidence.same_process_instance,
        window_relation_code = evidence.window_relation.as_str(),
        activation_changed = evidence.activation_changed,
        space_changed = evidence.space_changed,
        current_is_self = evidence.current_is_self,
        ownership_current = evidence.ownership_current,
        "delivery target verification"
    );
}

#[cfg(test)]
mod clipboard_delivery_tests {
    use super::*;
    use crate::injector::{ClipboardOnlyReason, InjectionOutcome};

    #[test]
    fn classifies_every_confirmed_clipboard_only_reason() {
        for reason in [
            ClipboardOnlyReason::AutoPasteDisabled,
            ClipboardOnlyReason::AccessibilityDenied,
            ClipboardOnlyReason::TargetChanged,
            ClipboardOnlyReason::FocusNotEditable,
            ClipboardOnlyReason::PasteFailed,
            ClipboardOnlyReason::SmartTargetUnavailable,
        ] {
            assert_eq!(
                confirmed_delivery_outcome(InjectionOutcome::ClipboardOnly(reason)),
                DictationDeliveryOutcome::ClipboardOnly
            );
        }
    }

    #[test]
    fn successful_auto_paste_does_not_signal_clipboard_only() {
        assert_eq!(
            confirmed_delivery_outcome(InjectionOutcome::AutoPasted),
            DictationDeliveryOutcome::AutoPastePosted
        );
    }

    #[test]
    fn empty_text_is_not_classified_as_clipboard_ready() {
        assert_eq!(
            confirmed_delivery_outcome(InjectionOutcome::NoText),
            DictationDeliveryOutcome::Unconfirmed
        );
        assert_eq!(
            confirmed_delivery_outcome(InjectionOutcome::ClipboardOnly(
                ClipboardOnlyReason::ClipboardChanged,
            )),
            DictationDeliveryOutcome::Unconfirmed
        );
    }

    #[test]
    fn stale_or_cancelled_delivery_does_not_signal() {
        assert_eq!(
            delivery_event_recording_id(11, 12, false, DictationStatus::Processing),
            None
        );
        assert_eq!(
            delivery_event_recording_id(12, 12, true, DictationStatus::Processing),
            None
        );
        assert_eq!(
            delivery_event_recording_id(12, 12, false, DictationStatus::Idle),
            None
        );
        assert_eq!(
            delivery_event_recording_id(12, 12, false, DictationStatus::Processing),
            Some(12)
        );
    }

    #[test]
    fn asynchronous_destination_capture_is_generation_and_lifecycle_owned() {
        for status in [
            DictationStatus::Starting,
            DictationStatus::Recording,
            DictationStatus::Recovering,
            DictationStatus::Processing,
        ] {
            assert!(recording_capture_is_current(12, 12, false, status));
        }
        assert!(!recording_capture_is_current(
            12,
            13,
            false,
            DictationStatus::Starting,
        ));
        assert!(!recording_capture_is_current(
            12,
            12,
            true,
            DictationStatus::Processing,
        ));
        assert!(!recording_capture_is_current(
            12,
            12,
            false,
            DictationStatus::Idle,
        ));
    }

    #[test]
    fn a_partial_ticker_registers_only_while_its_recording_is_live() {
        // Ids above any other test's, since the ticker registration only grows.
        let app_state = AppState::default();
        app_state.recording_id.store(9_000_001, Ordering::SeqCst);
        let mut dictation = app_state.dictation.lock_or_recover();
        dictation.status = DictationStatus::Recording;
        // A stale generation is refused even while the lock shows Recording.
        assert!(!register_partial_ticker_locked(
            &app_state, &dictation, 9_000_000
        ));
        assert!(register_partial_ticker_locked(
            &app_state, &dictation, 9_000_001
        ));
        // A ready callback that resumes after the stop transition is refused:
        // the stop already claimed (or declined) the hide under this lock.
        app_state.recording_id.store(9_000_002, Ordering::SeqCst);
        dictation.status = DictationStatus::Processing;
        assert!(!register_partial_ticker_locked(
            &app_state, &dictation, 9_000_002
        ));
        dictation.status = DictationStatus::Recording;
        assert!(register_partial_ticker_locked(
            &app_state, &dictation, 9_000_002
        ));
        // The predecessor resuming now cannot overwrite its successor.
        app_state.recording_id.store(9_000_001, Ordering::SeqCst);
        assert!(!register_partial_ticker_locked(
            &app_state, &dictation, 9_000_001
        ));
    }

    #[test]
    fn queued_delivery_rechecks_ownership_before_running_its_side_effect() {
        let app_state = AppState::default();
        app_state.recording_id.store(12, Ordering::SeqCst);
        app_state.dictation.lock_or_recover().status = DictationStatus::Processing;

        let mut calls = 0;
        assert_eq!(
            with_current_delivery_owner(&app_state, 12, || {
                assert!(matches!(
                    app_state.dictation.try_lock(),
                    Err(std::sync::TryLockError::WouldBlock)
                ));
                calls += 1;
                42
            }),
            Some(42)
        );

        app_state.dictation.lock_or_recover().status = DictationStatus::Idle;
        assert_eq!(
            with_current_delivery_owner(&app_state, 12, || {
                calls += 1;
                43
            }),
            None
        );

        app_state.dictation.lock_or_recover().status = DictationStatus::Processing;
        app_state.recording_id.store(13, Ordering::SeqCst);
        assert_eq!(
            with_current_delivery_owner(&app_state, 12, || {
                calls += 1;
                44
            }),
            None
        );
        assert_eq!(calls, 1);

        let cancelled_state = AppState::default();
        cancelled_state.recording_id.store(12, Ordering::SeqCst);
        cancelled_state.dictation.lock_or_recover().status = DictationStatus::Processing;
        cancelled_state.cancel_recording(12);
        assert_eq!(
            with_current_delivery_owner(&cancelled_state, 12, || 45),
            None
        );
    }

    #[test]
    fn delayed_generation_side_effect_cannot_cross_cancel_or_supersession() {
        let app_state = AppState::default();
        app_state.recording_id.store(12, Ordering::SeqCst);

        let mut calls = 0;
        assert_eq!(
            with_current_recording_generation(&app_state, 12, || {
                assert!(matches!(
                    app_state.dictation.try_lock(),
                    Err(std::sync::TryLockError::WouldBlock)
                ));
                calls += 1;
                42
            }),
            Some(42)
        );

        app_state.recording_id.store(13, Ordering::SeqCst);
        assert_eq!(
            with_current_recording_generation(&app_state, 12, || {
                calls += 1;
                43
            }),
            None
        );

        let cancelled_state = AppState::default();
        cancelled_state.recording_id.store(12, Ordering::SeqCst);
        cancelled_state.cancel_recording(12);
        assert_eq!(
            with_current_recording_generation(&cancelled_state, 12, || 44),
            None
        );
        assert_eq!(calls, 1);
    }

    #[test]
    fn delivery_event_values_are_stable_and_content_free() {
        assert_eq!(
            DictationDeliveryOutcome::ClipboardOnly.as_event_value(),
            "clipboardOnly"
        );
        assert_eq!(
            DictationDeliveryOutcome::AutoPastePosted.as_event_value(),
            "autoPastePosted"
        );
        assert_eq!(
            DictationDeliveryOutcome::ClipboardWriteFailed.as_event_value(),
            "clipboardWriteFailed"
        );
        assert_eq!(
            DictationDeliveryOutcome::Unconfirmed.as_event_value(),
            "unconfirmed"
        );
    }
}

struct PipelineResult {
    // Return ownership only. Shadow admission/setup happens after this future's Idle guard drops.
    final_input: Option<Vec<f32>>,
    text: String,
    timings: PipelineTimings,
    terminal: PipelineTerminal,
    history: Option<PipelineHistory>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PipelineHistory {
    raw_text: String,
    stages: Vec<HistoryStageResult>,
}

#[derive(serde::Serialize)]
pub struct HistoryStageResult {
    pub stage: &'static str,
    pub outcome: &'static str,
    pub changed: bool,
}

fn runtime_identity(model_name: &str, warm_state: ModelWarmStateV1) -> Vec<RuntimeIdentityV1> {
    runtime_identity_for_role(model_name, warm_state, RuntimeRoleV1::Transcription)
}

fn emit_performance_store_begin_failure(recording_id: u64, error: &PerformanceStoreError) {
    tracing::warn!(
        target: "system",
        event_code = "performance.store_operation_failed",
        operation = error.operation.as_str(),
        error_class = error.class.as_str(),
        attempts = u64::from(error.attempts),
        recording_id,
        "performance diagnostics store operation failed"
    );
}

#[derive(Clone)]
struct DictationPerformanceCompletion {
    delivery_timing: Option<Box<dt::DeliveryTimingV1>>,
    outcome: RunOutcomeV1,
    stages: Vec<StageTimingV1>,
    input: Option<ContentFreeInputSummaryV1>,
    runtimes: Option<Vec<RuntimeIdentityV1>>,
}

enum DictationPerformanceBeginState {
    Beginning {
        completion: Option<DictationPerformanceCompletion>,
        latest_stage: Option<PerformanceStageV1>,
    },
    Ready,
    Completing(DictationPerformanceCompletion),
}

struct DictationPerformanceBeginResolution {
    completion: Option<DictationPerformanceCompletion>,
    latest_stage: Option<PerformanceStageV1>,
}

#[derive(Default)]
struct DictationPerformanceBeginCoordinator {
    entries: Mutex<HashMap<u64, DictationPerformanceBeginState>>,
}

enum DictationPerformanceCompletionAction {
    Persist(DictationPerformanceCompletion),
    Deferred,
    AlreadyCompleting,
}

impl DictationPerformanceBeginCoordinator {
    fn register(&self, recording_id: u64) {
        self.entries.lock_or_recover().insert(
            recording_id,
            DictationPerformanceBeginState::Beginning {
                completion: None,
                latest_stage: None,
            },
        );
    }

    fn abort(
        &self,
        recording_id: u64,
        performance: &crate::performance_metrics::PerformanceMetrics,
    ) {
        self.entries.lock_or_recover().remove(&recording_id);
        // Retract the deferral slot first. A pipeline registering after this
        // removal must persist (and release its job), never defer into a dead run.
        performance.discard_shadow(recording_id);
    }

    fn request_completion(
        &self,
        recording_id: u64,
        completion: DictationPerformanceCompletion,
    ) -> DictationPerformanceCompletionAction {
        let mut entries = self.entries.lock_or_recover();
        match entries.get_mut(&recording_id) {
            Some(DictationPerformanceBeginState::Beginning {
                completion: pending,
                ..
            }) => {
                if pending.is_none() {
                    *pending = Some(completion);
                    DictationPerformanceCompletionAction::Deferred
                } else {
                    DictationPerformanceCompletionAction::AlreadyCompleting
                }
            }
            Some(DictationPerformanceBeginState::Ready) => {
                entries.insert(
                    recording_id,
                    DictationPerformanceBeginState::Completing(completion.clone()),
                );
                DictationPerformanceCompletionAction::Persist(completion)
            }
            Some(DictationPerformanceBeginState::Completing(_)) => {
                DictationPerformanceCompletionAction::AlreadyCompleting
            }
            None => {
                entries.insert(
                    recording_id,
                    DictationPerformanceBeginState::Completing(completion.clone()),
                );
                DictationPerformanceCompletionAction::Persist(completion)
            }
        }
    }

    fn resolve_begin(
        &self,
        recording_id: u64,
        succeeded: bool,
        performance: &crate::performance_metrics::PerformanceMetrics,
    ) -> Option<DictationPerformanceBeginResolution> {
        let mut entries = self.entries.lock_or_recover();
        let (completion, latest_stage) = match entries.get_mut(&recording_id) {
            Some(DictationPerformanceBeginState::Beginning {
                completion,
                latest_stage,
            }) => (completion.take(), latest_stage.take()),
            Some(
                DictationPerformanceBeginState::Ready
                | DictationPerformanceBeginState::Completing(_),
            )
            | None => return None,
        };
        if succeeded {
            if let Some(completion) = &completion {
                entries.insert(
                    recording_id,
                    DictationPerformanceBeginState::Completing(completion.clone()),
                );
            } else {
                entries.insert(recording_id, DictationPerformanceBeginState::Ready);
            }
            Some(DictationPerformanceBeginResolution {
                completion,
                latest_stage,
            })
        } else {
            entries.remove(&recording_id);
            drop(entries);
            // Cleanup follows failed resolution, closing the register/defer race.
            performance.discard_shadow(recording_id);
            None
        }
    }

    fn finish_completion(
        &self,
        recording_id: u64,
        performance: &crate::performance_metrics::PerformanceMetrics,
    ) {
        let mut entries = self.entries.lock_or_recover();
        let should_remove = match entries.get(&recording_id) {
            Some(DictationPerformanceBeginState::Completing(claimed)) => {
                // Keep the claimed payload alive until persistence succeeds;
                // observing it here also makes that ownership explicit.
                let _ = &claimed.outcome;
                true
            }
            _ => false,
        };
        if should_remove {
            entries.remove(&recording_id);
            drop(entries);
            // Completion already consumed its job; discard any late registration
            // arriving between that consumption and removal of the claim.
            performance.discard_shadow(recording_id);
        }
    }

    fn defer_stage(&self, recording_id: u64, stage: PerformanceStageV1) -> bool {
        let mut entries = self.entries.lock_or_recover();
        match entries.get_mut(&recording_id) {
            Some(DictationPerformanceBeginState::Beginning { latest_stage, .. }) => {
                *latest_stage = Some(stage);
                true
            }
            Some(DictationPerformanceBeginState::Completing(_)) => true,
            Some(DictationPerformanceBeginState::Ready) => false,
            None => false,
        }
    }

    fn completion_is_claimed(&self, recording_id: u64) -> bool {
        matches!(
            self.entries.lock_or_recover().get(&recording_id),
            Some(DictationPerformanceBeginState::Completing(_))
        )
    }

    #[cfg(test)]
    fn claimed_outcome(&self, recording_id: u64) -> Option<RunOutcomeV1> {
        match self.entries.lock_or_recover().get(&recording_id) {
            Some(DictationPerformanceBeginState::Completing(claimed)) => {
                Some(claimed.outcome.clone())
            }
            _ => None,
        }
    }

    #[cfg(test)]
    fn begin_is_pending(&self, recording_id: u64) -> bool {
        self.entries.lock_or_recover().contains_key(&recording_id)
    }
}

struct DictationPerformanceBeginRegistration<'a> {
    coordinator: &'a DictationPerformanceBeginCoordinator,
    performance: &'a crate::performance_metrics::PerformanceMetrics,
    recording_id: u64,
    armed: bool,
}

impl<'a> DictationPerformanceBeginRegistration<'a> {
    fn new(
        coordinator: &'a DictationPerformanceBeginCoordinator,
        recording_id: u64,
        performance: &'a crate::performance_metrics::PerformanceMetrics,
    ) -> Self {
        Self {
            coordinator,
            performance,
            recording_id,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for DictationPerformanceBeginRegistration<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.coordinator.abort(self.recording_id, self.performance);
        }
    }
}

fn dictation_performance_begins() -> &'static DictationPerformanceBeginCoordinator {
    static COORDINATOR: OnceLock<DictationPerformanceBeginCoordinator> = OnceLock::new();
    COORDINATOR.get_or_init(DictationPerformanceBeginCoordinator::default)
}

fn persist_dictation_performance_completion(
    performance: &crate::performance_metrics::PerformanceMetrics,
    recording_id: u64,
    completion: DictationPerformanceCompletion,
) -> Result<(), String> {
    performance
        .complete_with_delivery(
            &RunCorrelationV1::Dictation { recording_id },
            completion.outcome,
            completion.stages,
            completion.input,
            completion.runtimes,
            completion.delivery_timing.map(|value| *value),
        )
        .map(|_| ())
}

fn flush_dictation_performance_begin(
    coordinator: &DictationPerformanceBeginCoordinator,
    performance: &crate::performance_metrics::PerformanceMetrics,
    recording_id: u64,
    resolution: DictationPerformanceBeginResolution,
) {
    if let Some(stage) = resolution.latest_stage {
        let _ = performance.set_current_stage(&RunCorrelationV1::Dictation { recording_id }, stage);
    }
    if let Some(completion) = resolution.completion {
        if persist_dictation_performance_completion(performance, recording_id, completion).is_ok() {
            coordinator.finish_completion(recording_id, performance);
        }
    }
}

fn spawn_dictation_performance_completion(
    performance: crate::performance_metrics::PerformanceMetrics,
    recording_id: u64,
    completion: DictationPerformanceCompletion,
) {
    drop(tauri::async_runtime::spawn_blocking(move || {
        if persist_dictation_performance_completion(&performance, recording_id, completion).is_ok()
        {
            dictation_performance_begins().finish_completion(recording_id, &performance);
        }
    }));
}

fn complete_dictation_performance(
    performance: &crate::performance_metrics::PerformanceMetrics,
    recording_id: u64,
    outcome: RunOutcomeV1,
    stages: Vec<StageTimingV1>,
    input: Option<ContentFreeInputSummaryV1>,
    runtimes: Option<Vec<RuntimeIdentityV1>>,
) {
    complete_dictation_performance_with_delivery(
        performance,
        recording_id,
        outcome,
        stages,
        input,
        runtimes,
        None,
    );
}

#[allow(clippy::too_many_arguments)]
fn complete_dictation_performance_with_delivery(
    performance: &crate::performance_metrics::PerformanceMetrics,
    recording_id: u64,
    outcome: RunOutcomeV1,
    stages: Vec<StageTimingV1>,
    input: Option<ContentFreeInputSummaryV1>,
    runtimes: Option<Vec<RuntimeIdentityV1>>,
    delivery_timing: Option<dt::DeliveryTimingV1>,
) {
    let completion = DictationPerformanceCompletion {
        delivery_timing: delivery_timing.map(Box::new),
        outcome,
        stages,
        input,
        runtimes,
    };
    match dictation_performance_begins().request_completion(recording_id, completion) {
        DictationPerformanceCompletionAction::Persist(completion) => {
            spawn_dictation_performance_completion(performance.clone(), recording_id, completion);
        }
        DictationPerformanceCompletionAction::AlreadyCompleting
            if dictation_performance_begins().completion_is_claimed(recording_id) =>
        {
            // A terminal outcome already owns completion, possibly after a
            // failed persistence attempt. This ignored request must not retain
            // a job registered after that attempt consumed its earlier slot.
            performance.discard_shadow(recording_id);
        }
        DictationPerformanceCompletionAction::Deferred
        | DictationPerformanceCompletionAction::AlreadyCompleting => {}
    }
}

fn spawn_native_dictation_performance_begin(
    performance: crate::performance_metrics::PerformanceMetrics,
    recording_id: u64,
    runtimes: Vec<RuntimeIdentityV1>,
    started_at_ms: i64,
) {
    drop(tauri::async_runtime::spawn_blocking(move || {
        let coordinator = dictation_performance_begins();
        let mut registration =
            DictationPerformanceBeginRegistration::new(coordinator, recording_id, &performance);
        let result =
            performance.begin_dictation_diagnosed_at(recording_id, runtimes, started_at_ms);
        let succeeded = result.is_ok();
        if let Err(error) = &result {
            emit_performance_store_begin_failure(recording_id, error);
        }
        if let Some(resolution) = coordinator.resolve_begin(recording_id, succeeded, &performance) {
            flush_dictation_performance_begin(coordinator, &performance, recording_id, resolution);
        }
        registration.disarm();
    }));
}

struct NativeDictationPerformanceGuard {
    performance: crate::performance_metrics::PerformanceMetrics,
    recording_id: u64,
    current_stage: PerformanceStageV1,
    buffered_stages: Vec<StageTimingV1>,
    delivery: Option<dt::DeliveryTiming>,
    finished: bool,
}

impl NativeDictationPerformanceGuard {
    fn new(
        performance: crate::performance_metrics::PerformanceMetrics,
        recording_id: u64,
        stage: PerformanceStageV1,
    ) -> Self {
        let guard = Self {
            performance,
            recording_id,
            current_stage: stage,
            buffered_stages: Vec::new(),
            delivery: None,
            finished: false,
        };
        guard.track_current_stage();
        guard
    }

    fn track_current_stage(&self) {
        // While begin is pending, the coordinator flushes the latest stage as
        // soon as the row exists. Once begin is Ready, final/Drop completion
        // carries the authoritative stage data. Native recognition must never
        // wait on an interim diagnostics write.
        let _ = dictation_performance_begins().defer_stage(self.recording_id, self.current_stage);
    }

    fn enter(&mut self, stage: PerformanceStageV1) {
        self.current_stage = stage;
        self.track_current_stage();
    }

    fn record(&mut self, timing: StageTimingV1) {
        self.current_stage = timing.stage;
        if let Some(existing) = self
            .buffered_stages
            .iter_mut()
            .find(|stage| stage.stage == timing.stage)
        {
            *existing = timing.clone();
        } else {
            self.buffered_stages.push(timing.clone());
        }
        self.track_current_stage();
    }

    fn finish(
        mut self,
        outcome: RunOutcomeV1,
        mut stages: Vec<StageTimingV1>,
        input: Option<ContentFreeInputSummaryV1>,
        runtimes: Option<Vec<RuntimeIdentityV1>>,
    ) {
        for timing in std::mem::take(&mut self.buffered_stages) {
            if !stages.iter().any(|stage| stage.stage == timing.stage) {
                stages.push(timing);
            }
        }
        self.finished = true;
        complete_dictation_performance_with_delivery(
            &self.performance,
            self.recording_id,
            outcome,
            stages,
            input,
            runtimes,
            self.delivery.as_ref().map(dt::DeliveryTiming::close),
        );
    }
}

impl Drop for NativeDictationPerformanceGuard {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        complete_dictation_performance_with_delivery(
            &self.performance,
            self.recording_id,
            RunOutcomeV1::Failed {
                stage: self.current_stage,
                error_code: StableRunErrorV1::InternalEarlyExit,
            },
            std::mem::take(&mut self.buffered_stages),
            None,
            None,
            self.delivery.as_ref().map(dt::DeliveryTiming::close),
        );
    }
}

trait DictationPerformanceStageRecorder {
    fn set_delivery(&mut self, timing: dt::DeliveryTiming);
    fn enter(&mut self, stage: PerformanceStageV1);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NativeStartContextAction {
    Activate,
    FlushLifecycleTerminal { install_context: bool },
    Superseded,
}

fn native_start_context_action(
    is_current_generation: bool,
    status: DictationStatus,
) -> NativeStartContextAction {
    if !is_current_generation {
        return NativeStartContextAction::Superseded;
    }
    match status {
        DictationStatus::Starting => NativeStartContextAction::Activate,
        DictationStatus::Recovering => NativeStartContextAction::FlushLifecycleTerminal {
            install_context: true,
        },
        DictationStatus::Idle => NativeStartContextAction::FlushLifecycleTerminal {
            install_context: false,
        },
        DictationStatus::Recording | DictationStatus::Processing => {
            NativeStartContextAction::Superseded
        }
    }
}

impl DictationPerformanceStageRecorder for PerformanceRunGuard {
    fn set_delivery(&mut self, timing: dt::DeliveryTiming) {
        PerformanceRunGuard::set_delivery(self, timing);
    }
    fn enter(&mut self, stage: PerformanceStageV1) {
        PerformanceRunGuard::enter(self, stage);
    }
}

impl DictationPerformanceStageRecorder for NativeDictationPerformanceGuard {
    fn set_delivery(&mut self, timing: dt::DeliveryTiming) {
        self.delivery = Some(timing);
    }
    fn enter(&mut self, stage: PerformanceStageV1) {
        NativeDictationPerformanceGuard::enter(self, stage);
    }
}

pub(crate) fn runtime_identity_for_role(
    model_name: &str,
    warm_state: ModelWarmStateV1,
    role: RuntimeRoleV1,
) -> Vec<RuntimeIdentityV1> {
    let Ok(definition) = model_runtime::model_definition(model_name) else {
        return Vec::new();
    };
    let backend = match definition.backend {
        model_runtime::BackendKind::Whisper => RuntimeBackendV1::Whisper,
        model_runtime::BackendKind::Parakeet => RuntimeBackendV1::Parakeet,
        model_runtime::BackendKind::Coreml => RuntimeBackendV1::Coreml,
    };
    let accelerator = match model_runtime::model_accelerator(definition) {
        "CPU" => AcceleratorV1::Cpu,
        "Metal GPU" => AcceleratorV1::MetalGpu,
        "Apple Neural Engine" => AcceleratorV1::AppleNeuralEngine,
        _ => AcceleratorV1::PlatformFallback,
    };
    vec![RuntimeIdentityV1 {
        role,
        model_id: model_name.to_string(),
        backend,
        accelerator,
        warm_state,
    }]
}

fn transcript_stage_timing(
    report: &crate::transcript_transform::StageReport,
) -> Option<StageTimingV1> {
    use crate::transcript_transform::{
        StageOutcome, CLEANUP_STAGE, CLI_COMMAND_STAGE, HOMOPHONE_CONTEXT_STAGE, IDE_CONTEXT_STAGE,
        SMART_CORRECTION_STAGE, SMART_FORMATTING_STAGE, SPOKEN_NUMBERS_STAGE,
        SPOKEN_STRUCTURE_STAGE,
    };

    let stage = match report.stage {
        CLEANUP_STAGE => PerformanceStageV1::Cleanup,
        SMART_CORRECTION_STAGE => PerformanceStageV1::SmartCorrection,
        HOMOPHONE_CONTEXT_STAGE => PerformanceStageV1::HomophoneContext,
        SMART_FORMATTING_STAGE => PerformanceStageV1::SmartFormatting,
        SPOKEN_STRUCTURE_STAGE => PerformanceStageV1::SpokenStructure,
        SPOKEN_NUMBERS_STAGE => PerformanceStageV1::SpokenNumbers,
        IDE_CONTEXT_STAGE => PerformanceStageV1::IdeContext,
        CLI_COMMAND_STAGE => PerformanceStageV1::CliCommand,
        _ => return None,
    };
    let outcome = match report.outcome {
        StageOutcome::Applied => StageOutcomeV1::Completed,
        StageOutcome::Skipped => StageOutcomeV1::Skipped,
        StageOutcome::Fallback => StageOutcomeV1::Fallback,
        StageOutcome::Failed => StageOutcomeV1::Failed,
    };
    Some(StageTimingV1 {
        stage,
        duration_ms: if report.outcome == StageOutcome::Skipped {
            crate::performance_metrics::MeasurementV1::NotApplicable
        } else {
            crate::performance_metrics::MeasurementV1::measured(report.duration_us / 1_000)
        },
        outcome,
    })
}

fn pipeline_stages(timings: &PipelineTimings, total_ms: u64) -> Vec<StageTimingV1> {
    let mut stages = vec![
        StageTimingV1::measured(PerformanceStageV1::Vad, timings.vad_ms),
        // Preparation may overlap capture and has its own ModelQueue sample.
        // Preserve the final pass's wait separately; do not replace preparation
        // or silently discard contention behind a preview decode.
        if timings.warm_state.is_some() {
            StageTimingV1::measured(PerformanceStageV1::InferenceQueue, timings.model_queue_ms)
        } else {
            StageTimingV1::not_applicable(PerformanceStageV1::InferenceQueue)
        },
        StageTimingV1::measured(PerformanceStageV1::ModelLoad, timings.model_load_ms),
        StageTimingV1::measured(PerformanceStageV1::InferenceDecode, timings.decode_ms),
        StageTimingV1::measured(
            PerformanceStageV1::TranscriptTransform,
            timings.transform_ms,
        ),
        if timings.file_output_ms == 0 {
            StageTimingV1::not_applicable(PerformanceStageV1::FileOutput)
        } else {
            StageTimingV1::measured(PerformanceStageV1::FileOutput, timings.file_output_ms)
        },
        StageTimingV1::measured(PerformanceStageV1::ClipboardPaste, timings.paste_ms),
        StageTimingV1::measured(PerformanceStageV1::TotalProcessing, total_ms),
    ];
    stages.extend(timings.transform_stages.clone());
    stages
}

struct NativeDictationMetrics<'a> {
    recording_id: u64,
    outcome: DictationTerminalOutcome,
    timings: &'a PipelineTimings,
    total_ms: u64,
    audio_secs: f64,
    word_count: usize,
    char_count: usize,
    model_name: &'a str,
    backend_name: &'a str,
}

fn emit_native_dictation_metrics(metrics: NativeDictationMetrics<'_>) {
    let event_code = match metrics.outcome {
        DictationTerminalOutcome::Success => "pipeline.dictation_completed",
        _ => "pipeline.dictation_metrics",
    };
    tracing::info!(
        target: "pipeline",
        event_code = event_code,
        recording_id = metrics.recording_id,
        vad_ms = metrics.timings.vad_ms,
        model_load_ms = metrics.timings.model_load_ms,
        decode_ms = metrics.timings.decode_ms,
        inference_ms = metrics.timings.inference_ms,
        correction_ms = metrics.timings.correction_ms,
        paste_ms = metrics.timings.paste_ms,
        total_ms = metrics.total_ms,
        audio_secs = metrics.audio_secs,
        word_count = metrics.word_count,
        char_count = metrics.char_count,
        rss_before_mb = metrics.timings.rss_before_mb,
        rss_after_mb = metrics.timings.rss_after_mb,
        model = metrics.model_name,
        backend = metrics.backend_name,
        "dictation processing metrics"
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn transcribe_with_coreml_vad_retry(
    backend: &mut dyn transcriber::TranscriptionBackend,
    model_name: &str,
    samples_for_transcription: &[f32],
    original_samples: &[f32],
    vad_trimmed: bool,
    language: &str,
    prompt: Option<&str>,
    smart_punctuation: bool,
) -> Result<String, String> {
    transcribe_with_coreml_vad_retry_report(
        backend,
        model_name,
        samples_for_transcription,
        original_samples,
        vad_trimmed,
        language,
        prompt,
        smart_punctuation,
    )
    .map(|attempt| attempt.text)
}

struct BackendTranscriptionAttempt {
    text: String,
    empty_error: Option<DictationErrorCode>,
}

#[allow(clippy::too_many_arguments)]
fn transcribe_with_coreml_vad_retry_report(
    backend: &mut dyn transcriber::TranscriptionBackend,
    model_name: &str,
    samples_for_transcription: &[f32],
    original_samples: &[f32],
    vad_trimmed: bool,
    language: &str,
    prompt: Option<&str>,
    smart_punctuation: bool,
) -> Result<BackendTranscriptionAttempt, String> {
    let text = backend.transcribe(
        samples_for_transcription,
        language,
        prompt,
        smart_punctuation,
    )?;
    if model_runtime::model_definition(model_name)?.retry_unfiltered_on_empty
        && vad_trimmed
        && text.trim().is_empty()
    {
        tracing::warn!(
            target: "pipeline",
            filtered_sample_count = samples_for_transcription.len(),
            original_sample_count = original_samples.len(),
            "coreml_empty_after_vad_retry_original"
        );
        let retried = backend.transcribe(original_samples, language, prompt, smart_punctuation)?;
        let empty_error = retried
            .trim()
            .is_empty()
            .then_some(DictationErrorCode::CoremlVadRetryExhausted);
        return Ok(BackendTranscriptionAttempt {
            text: retried,
            empty_error,
        });
    }
    let empty_error = text
        .trim()
        .is_empty()
        .then_some(DictationErrorCode::EmptyOutput);
    Ok(BackendTranscriptionAttempt { text, empty_error })
}

/// What the final decode did with decode reuse.
struct FinalDecode {
    attempt: BackendTranscriptionAttempt,
    /// `None` when the recording asked for no reuse decision (setting off).
    reuse: Option<crate::stream_reuse::Outcome>,
}

/// The final decode. Without a reuse decision (`reuse` is `Ok(None)`: setting
/// off) this is the pre-existing whole-clip decoder with precisely the same
/// PCM, settings, retry policy and returned bytes.
///
/// With one, the stitched result is used only when every proof in
/// [`crate::stream_reuse::StreamReuse::take_final`] passed and the decoded tail
/// starts with the committed overlap text. Any doubt, a decode error, or a
/// failed stitch runs the unchanged whole-clip decode on `filtered`. The
/// stitched text is raw ASR text: the one ordered transform pipeline runs over
/// it once, afterwards.
fn decode_final_transcript(
    backend: &mut dyn transcriber::TranscriptionBackend,
    transcription: &dictation_context::TranscriptionSettings,
    filtered: &[f32],
    original: &[f32],
    vad_trimmed: bool,
    reuse: Result<Option<crate::stream_reuse::FinalPlan>, crate::stream_reuse::Reason>,
) -> Result<FinalDecode, String> {
    use crate::stream_reuse::{Outcome, Reason};
    let whole_clip = |backend: &mut dyn transcriber::TranscriptionBackend| {
        transcribe_with_coreml_vad_retry_report(
            backend,
            &transcription.model_name,
            filtered,
            original,
            vad_trimmed,
            &transcription.language,
            transcription.prompt.as_deref(),
            transcription.smart_punctuation,
        )
    };
    let fallback = |backend: &mut dyn transcriber::TranscriptionBackend,
                    reason: Reason,
                    pieces: u32,
                    overlap_words: u64,
                    tail_samples: usize| {
        whole_clip(backend).map(|attempt| FinalDecode {
            attempt,
            reuse: Some(Outcome {
                code: reason.as_str(),
                pieces,
                // Time spent at stop: a tail decode that was then discarded,
                // plus the whole-clip decode that replaced it.
                decoded_ms: samples_to_ms(tail_samples + filtered.len()),
                overlap_words,
            }),
        })
    };
    let plan = match reuse {
        Ok(None) => {
            return whole_clip(backend).map(|attempt| FinalDecode {
                attempt,
                reuse: None,
            })
        }
        Err(reason) => return fallback(backend, reason, 0, 0, 0),
        Ok(Some(plan)) => plan,
    };
    let reused = |text: String, decoded_samples: usize| FinalDecode {
        attempt: BackendTranscriptionAttempt {
            text,
            empty_error: None,
        },
        reuse: Some(Outcome {
            code: "reused",
            pieces: plan.pieces,
            decoded_ms: samples_to_ms(decoded_samples),
            overlap_words: plan.overlap_words as u64,
        }),
    };
    if let Some(text) = plan.complete_text() {
        // The committed pieces already cover every kept sample.
        return Ok(reused(text, 0));
    }
    let tail = &filtered[plan.start..];
    let stitched = match backend.transcribe(
        tail,
        &transcription.language,
        transcription.prompt.as_deref(),
        transcription.smart_punctuation,
    ) {
        Ok(decoded) => plan
            .stitch(&decoded)
            .ok_or(Reason::StitchMismatch)
            .map(|text| reused(text, tail.len())),
        Err(_) => Err(Reason::TailDecodeError),
    };
    match stitched {
        Ok(decode) => Ok(decode),
        Err(reason) => fallback(
            backend,
            reason,
            plan.pieces,
            plan.overlap_words as u64,
            tail.len(),
        ),
    }
}

fn samples_to_ms(samples: usize) -> u64 {
    (samples / 16) as u64
}

/// The Stop-time reuse decision for `recording_id`, taken after the backend is
/// held. `Ok(None)` means the recording never asked for reuse. The plan is
/// consumed at most once, and only for the recording that is still the live,
/// uncancelled one.
fn take_stream_reuse_plan(
    app_state: &AppState,
    transcription: &dictation_context::TranscriptionSettings,
    recording_id: u64,
    final_bounds: Option<&[crate::stream_reuse::Bounds]>,
    filtered: &[f32],
) -> Result<Option<crate::stream_reuse::FinalPlan>, crate::stream_reuse::Reason> {
    use crate::stream_reuse::Reason;
    if !transcription.streaming_pipeline_enabled {
        return Ok(None);
    }
    // Setting on, but this recording could not have a reuse session (no VAD to
    // cut pieces from, or a model other than Core ML): say so, decode whole.
    if !vad::is_enabled(transcription.vad_sensitivity)
        || !transcriber::is_coreml_model(&transcription.model_name)
    {
        return Err(Reason::Inactive);
    }
    // A preview that still holds its lease finishes in microseconds once it
    // has dropped the backend (we hold the backend, so it can no longer commit
    // anything); wait for that briefly rather than lose reuse to the race.
    // This is a spin on `yield_now`, bounded at 10 ms, only when the setting
    // is on and the lease is still held; it adds up to 10 ms to Stop in that
    // case and nothing otherwise (then the outcome is `busy`, whole-clip).
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(10);
    while app_state
        .dictation_partial_in_flight_id
        .load(Ordering::SeqCst)
        != 0
        && std::time::Instant::now() < deadline
    {
        std::thread::yield_now();
    }
    let session =
        app_state
            .stream_reuse
            .lock_or_recover()
            .take_final(recording_id, final_bounds, filtered);
    if app_state.recording_id.load(Ordering::SeqCst) != recording_id
        || app_state.is_cancelled(recording_id)
    {
        return Err(Reason::Superseded);
    }
    if app_state
        .dictation_partial_in_flight_id
        .load(Ordering::SeqCst)
        != 0
    {
        return Err(Reason::Busy);
    }
    session.map(Some)
}

/// Register only after Idle and captured delivery timing. Ownership moves into
/// a worker that starts after the exact completed UUID is durable.
fn register_shadow_measurement(
    app: &tauri::AppHandle,
    recording_id: u64,
    context: Arc<DictationContextSnapshot>,
    audio: crate::shadow_measurement::ShadowAudio,
) {
    use crate::shadow_measurement as shadow;
    let Some(permit) = shadow::ShadowPermit::acquire() else {
        return;
    };
    app.state::<State>().performance.register_shadow(
        recording_id,
        Box::new(move |metrics, run| {
            let _permit = permit;
            let supported = transcriber::is_coreml_model(&context.transcription.model_name);
            let threshold = vad::threshold_for_sensitivity(context.transcription.vad_sensitivity);
            let path = vad::vad_model_path().filter(|p| p.exists());
            let result = audio.measure(
                supported,
                |original, filtered| {
                    if !vad::is_enabled(context.transcription.vad_sensitivity) {
                        return None;
                    }
                    let (result, bounds) = vad::filter_speech_with_segments(
                        &path.as_ref()?.to_string_lossy(),
                        original,
                        threshold,
                    )
                    .ok()?;
                    match result {
                        vad::VadResult::Speech(repeated)
                            if shadow::pcm_equal(&repeated, filtered) =>
                        {
                            Some(bounds)
                        }
                        _ => None,
                    }
                },
                |original, at, filtered| {
                    shadow::prefix_matches(
                        &path.as_ref()?.to_string_lossy(),
                        original,
                        at,
                        threshold,
                        filtered,
                    )
                },
            );
            let _ = metrics.amend_shadow(&run.run_id, recording_id, &result);
        }),
    );
}

#[derive(Default)]
struct PipelineStopContext {
    delivery_target: Option<crate::frontmost::DeliveryTargetSnapshot>,
    /// The live preview's hide, owned by delivery from the accepted stop.
    preview_hide: Option<crate::commands::dictation_preview::PreviewHideOwner>,
}

/// Shared transcription pipeline: model init -> transcribe -> inject text -> set idle.
/// `recording_id` is checked against `app_state.cancelled_id` at checkpoints;
/// if cancelled, returns empty text without clipboard write or paste.
async fn run_transcription_pipeline(
    samples: &[f32],
    app_handle: &tauri::AppHandle,
    app_state: &AppState,
    recording_id: u64,
    performance_guard: &mut impl DictationPerformanceStageRecorder,
    context: Arc<DictationContextSnapshot>,
    stop: PipelineStopContext,
) -> Result<PipelineResult, String> {
    let PipelineStopContext {
        delivery_target: stop_delivery_target,
        preview_hide,
    } = stop;
    // Dropped on every return without a delivery job, which hides at once.
    let mut preview_hide = preview_hide;
    let t_pipeline = std::time::Instant::now();
    let mut gaps = StageGaps {
        entered_at: Some(t_pipeline),
        ..StageGaps::default()
    };
    // Guard resets status to Idle on any return path (error or success),
    // but only if this recording is still the active one
    let mut _guard = IdleGuard::new(app_state, recording_id);

    let transcription = &context.transcription;
    let transformations = &context.transformations;
    let delivery = &context.delivery;
    // When saving to a file, suppress auto-paste into the focused app. The
    // clipboard write inside `inject_text` is unconditional, so text remains
    // copyable regardless of these toggles.
    let effective_auto_paste =
        delivery.auto_paste && !(delivery.save_transcript || delivery.save_audio);

    // Pre-VAD signal level logging for mic diagnosis
    let rms = audio::compute_rms(samples);
    let peak = audio::compute_peak(samples);
    tracing::info!(
        target: "pipeline",
        rms,
        peak,
        "captured audio signal summary"
    );

    // Checkpoint 1: cancelled before VAD?
    if app_state.is_cancelled(recording_id) {
        tracing::info!(target: "pipeline", "cancelled before VAD (recording_id={})", recording_id);
        return Ok(PipelineResult {
            final_input: None,
            text: String::new(),
            timings: PipelineTimings::default(),
            terminal: PipelineTerminal::Cancelled(PerformanceStageV1::Vad),
            history: None,
        });
    }

    // Sensitivity zero is an explicit lowest-latency mode. Other values retain
    // the authoritative full-buffer VAD + inference pass after recording stops.
    performance_guard.enter(PerformanceStageV1::Vad);
    let t_vad = std::time::Instant::now();
    gaps.prologue_us = micros_between(t_pipeline, t_vad);
    // `final_bounds`: the incremental VAD's padded segment bounds, present only
    // when that result (not the batch VAD) produced the kept audio. Decode reuse
    // compares them with the segments it decoded during recording.
    let mut final_bounds: Option<Vec<(usize, usize)>> = None;
    let (samples_for_transcription, vad_trimmed) = if !vad::is_enabled(
        transcription.vad_sensitivity,
    ) {
        tracing::info!(target: "pipeline", "VAD disabled for lowest-latency transcription");
        (samples.to_vec(), false)
    } else {
        let vad_threshold = vad::threshold_for_sensitivity(transcription.vad_sensitivity);
        match vad::vad_model_path() {
            Some(vad_path) if vad_path.exists() => {
                let vad_path_str = vad_path.to_string_lossy().to_string();
                let samples_owned = samples.to_vec();
                // Streaming setting (frozen at recording start): take this
                // recording's incremental VAD; the batch VAD is the fallback.
                let streaming = transcription.streaming_pipeline_enabled;
                let stream_handoff = streaming
                    .then(|| app_state.vad_stream.take(recording_id))
                    .flatten();
                let vad_queued_at = std::time::Instant::now();
                let vad_result = match tokio::task::spawn_blocking(move || {
                    let queue_us = micros(vad_queued_at);
                    let (result, _outcome, bounds) = crate::vad_stream::stop_vad(
                        streaming,
                        stream_handoff,
                        recording_id,
                        &samples_owned,
                        crate::vad_stream::SegmentParams::murmur(vad_threshold),
                        || vad::filter_speech(&vad_path_str, &samples_owned, vad_threshold),
                    );
                    let result = result.map(|result| (result, bounds));
                    (result, queue_us)
                })
                .await
                {
                    Ok((result, queue_us)) => {
                        gaps.vad_blocking_queue_us = queue_us;
                        result.map(|(result, bounds)| {
                            final_bounds = bounds;
                            result
                        })
                    }
                    Err(e) => Err(format!("VAD task panicked: {}", e)),
                };

                match vad_result {
                    Ok(vad::VadResult::NoSpeech) => {
                        emit_preview_vad_disagreement(app_state, recording_id, samples.len());
                        tracing::info!(target: "pipeline", "VAD detected no speech ({} samples, {:?}), skipping transcription",
                            samples.len(), t_vad.elapsed());
                        return Ok(PipelineResult {
                            final_input: None,
                            text: String::new(),
                            timings: PipelineTimings {
                                vad_ms: t_vad.elapsed().as_millis() as u64,
                                ..PipelineTimings::default()
                            },
                            terminal: PipelineTerminal::NoSpeech(DictationErrorCode::VadNoSpeech),
                            history: None,
                        });
                    }
                    Ok(vad::VadResult::Speech(trimmed)) => {
                        tracing::info!(target: "pipeline", "VAD trimmed {} -> {} samples ({:.0}% speech, {:?})",
                            samples.len(), trimmed.len(),
                            trimmed.len() as f64 / samples.len() as f64 * 100.0,
                            t_vad.elapsed());
                        let vad_trimmed = trimmed.len() != samples.len();
                        (trimmed, vad_trimmed)
                    }
                    Err(e) => {
                        tracing::warn!(target: "pipeline", "VAD failed ({}), proceeding without filtering", e);
                        (samples.to_vec(), false)
                    }
                }
            }
            _ => {
                let handle = app_handle.clone();
                tokio::spawn(async move {
                    if let Err(e) = super::models::ensure_vad_model(&handle).await {
                        tracing::warn!(target: "pipeline", "VAD model download failed ({}), skipping VAD", e);
                    }
                });
                (samples.to_vec(), false)
            }
        }
    };
    let vad_ms = t_vad.elapsed().as_millis() as u64;
    gaps.vad_us = micros(t_vad);
    let t_after_vad = std::time::Instant::now();

    if app_state.is_cancelled(recording_id) {
        tracing::info!(target: "pipeline", "cancelled before transcription (recording_id={})", recording_id);
        return Ok(PipelineResult {
            final_input: None,
            text: String::new(),
            timings: PipelineTimings {
                vad_ms,
                ..PipelineTimings::default()
            },
            terminal: PipelineTerminal::Cancelled(PerformanceStageV1::InferenceDecode),
            history: None,
        });
    }

    let rss_before_mb = crate::resource_monitor::get_process_rss_mb();
    performance_guard.enter(PerformanceStageV1::InferenceDecode);
    let t_transcribe = std::time::Instant::now();
    let mut decode_ms = 0;
    let t_backend = std::time::Instant::now();
    gaps.pre_backend_us = micros_between(t_after_vad, t_backend);
    let mut decode_ended_at = None;
    let (transcription_attempt, load_report) = app_state.model_runtime.with_ready_backend(
        Some(app_handle),
        &transcription.model_name,
        PreparationReason::Pipeline,
        |backend| {
            let decode_started = std::time::Instant::now();
            gaps.backend_acquire_us = micros_between(t_backend, decode_started);
            // Take only after acquiring the backend: a preview decode that was
            // running holds it, so by now it has finished and committed or not.
            let reuse_plan = take_stream_reuse_plan(
                app_state,
                transcription,
                recording_id,
                final_bounds.as_deref(),
                &samples_for_transcription,
            );
            let result = decode_final_transcript(
                backend,
                transcription,
                &samples_for_transcription,
                samples,
                vad_trimmed,
                reuse_plan,
            )
            .map(|decoded| {
                if let Some(outcome) = &decoded.reuse {
                    crate::stream_reuse::emit_outcome(recording_id, outcome);
                }
                decoded.attempt
            });
            decode_ms = decode_started.elapsed().as_millis() as u64;
            let decode_ended = std::time::Instant::now();
            gaps.backend_decode_us = micros_between(decode_started, decode_ended);
            decode_ended_at = Some(decode_ended);
            result
        },
    )?;
    let t_after_backend = std::time::Instant::now();
    gaps.backend_release_us = decode_ended_at
        .map(|ended| micros_between(ended, t_after_backend))
        .unwrap_or(0);
    gaps.backend_model_queue_us = load_report.lock_wait_ms.saturating_mul(1_000);
    gaps.backend_model_load_us = load_report.load_ms.saturating_mul(1_000);
    let backend_empty_error = transcription_attempt.empty_error;
    let text = transcription_attempt.text;
    let model_load_ms = load_report.load_ms;
    let inference_ms = t_transcribe.elapsed().as_millis() as u64;
    let rss_after_mb = crate::resource_monitor::get_process_rss_mb();
    tracing::info!(target: "pipeline", "transcription ({} samples): {:?}", samples_for_transcription.len(), t_transcribe.elapsed());
    let mut timings = PipelineTimings {
        vad_ms,
        model_queue_ms: load_report.lock_wait_ms,
        model_load_ms,
        decode_ms,
        inference_ms,
        rss_before_mb,
        rss_after_mb,
        warm_state: Some(if load_report.cache_hit {
            ModelWarmStateV1::Warm
        } else {
            ModelWarmStateV1::ColdLoaded
        }),
        ..PipelineTimings::default()
    };

    // Post-recognition transformation is backend-neutral and ordered in one
    // authoritative entry point. Its stage config and resources come from the
    // immutable recording-start snapshot rather than mutable app settings.
    let transform_context = crate::transcript_transform::TranscriptContext {
        session_id: app_state.next_transcript_session_id(),
        source: crate::transcript_transform::TranscriptSource::Live,
        context_handle: Some(format!("recording:{recording_id}")),
        cli_formatting_mode: transformations.cli_formatting_mode,
        stages: crate::transcript_transform::TranscriptStageConfig {
            cleanup_enabled: transformations.cleanup_enabled,
            cleanup_remove_filler: transformations.cleanup_remove_filler,
            cleanup_capitalize: transformations.cleanup_capitalize,
            smart_correction_enabled: transformations.correction_enabled,
            homophone_context_enabled: transformations.homophone_context_enabled,
            smart_formatting_enabled: transformations.smart_formatting_enabled,
            spoken_structure_policy: transformations.spoken_structure_policy,
            spoken_numbers_enabled: transformations.spoken_numbers_enabled,
            ide_context_enabled: transformations.ide_context_enabled,
            cli_command_enabled: transformations.cli_formatting_enabled,
            text_register: transformations.text_register,
        },
    };
    let cli_lexicon =
        crate::cli_command::CliLexicon::from_context(transcription.prompt.as_deref(), &[]);
    let transform_resources = crate::transcript_transform::TranscriptTransformResources {
        correction_matcher: transformations.correction_matcher.clone(),
        cli_lexicon,
        ide_context_index: transformations.ide_context_index.clone(),
        homophone_engine: context.homophone_engine.clone(),
    };
    let transform_started = std::time::Instant::now();
    gaps.transform_setup_us = micros_between(t_after_backend, transform_started);
    performance_guard.enter(PerformanceStageV1::TranscriptTransform);
    let transformed = crate::transcript_transform::transform_transcript(
        text,
        &transform_context,
        transform_resources,
    )
    .map_err(|error| error.to_string())?;
    let transform_ms = transform_started.elapsed().as_millis() as u64;
    let transform_ended_at = std::time::Instant::now();
    gaps.transform_us = micros_between(transform_started, transform_ended_at);
    tracing::info!(
        target: "pipeline",
        changed = transformed.was_changed(),
        "transcript_transform_complete"
    );
    let correction_ms =
        transformed.stage_duration_ms(crate::transcript_transform::SMART_CORRECTION_STAGE);
    let transform_stages = transformed
        .stages
        .iter()
        .filter_map(transcript_stage_timing)
        .collect();
    let history = PipelineHistory {
        raw_text: transformed.original_text.clone(),
        stages: transformed
            .stages
            .iter()
            .map(|stage| HistoryStageResult {
                stage: stage.stage,
                outcome: stage.outcome.as_str(),
                changed: stage.changed,
            })
            .collect(),
    };
    let text = transformed.text;

    // Update last_transcription_at for idle timeout tracking
    *app_state.last_transcription_at.lock_or_recover() = Some(std::time::Instant::now());
    // Checkpoint 3: cancelled before text injection?
    if app_state.is_cancelled(recording_id) {
        tracing::info!(target: "pipeline", "cancelled before injection (recording_id={})", recording_id);
        timings.correction_ms = correction_ms;
        timings.transform_ms = transform_ms;
        timings.transform_stages = transform_stages;
        return Ok(PipelineResult {
            final_input: None,
            text: String::new(),
            timings,
            terminal: PipelineTerminal::Cancelled(PerformanceStageV1::ClipboardPaste),
            history: None,
        });
    }

    // Phase: File output (optional) -- persist audio/transcript before injection.
    // Non-fatal: a write failure is logged and surfaced to the UI, but the text
    // is already on its way to the clipboard. Uses the original (pre-VAD) samples.
    let file_output_started = std::time::Instant::now();
    performance_guard.enter(PerformanceStageV1::FileOutput);
    if delivery.save_audio || delivery.save_transcript {
        if let Err(e) = crate::file_output::write_dictation_outputs(
            samples,
            &text,
            delivery.save_audio,
            delivery.save_transcript,
            &delivery.output_dir,
        ) {
            tracing::warn!(target: "pipeline", "file output failed: {}", e);
            let _ = app_handle.emit(
                "file-output-failed",
                "Couldn't save dictation to file. Text is still in your clipboard.",
            );
        }
    }
    let file_output_ms = if delivery.save_audio || delivery.save_transcript {
        file_output_started.elapsed().as_millis() as u64
    } else {
        0
    };

    // Phase: Text injection (clipboard write + optional verified native paste)
    let t_inject = std::time::Instant::now();
    gaps.post_transform_us = micros_between(transform_ended_at, t_inject);
    performance_guard.enter(PerformanceStageV1::ClipboardPaste);
    let delivery_timing = dt::DeliveryTiming::new(t_inject);
    performance_guard.set_delivery(delivery_timing.clone());
    let _close_delivery = dt::CloseOnDrop(delivery_timing.clone());
    if !text.is_empty() {
        let text_to_inject = text.clone();
        let paste_delay_ms = delivery.paste_delay_ms;
        let (anchored_target, delivery_anchor) = crate::frontmost::select_delivery_anchor(
            &context.app.delivery_target,
            stop_delivery_target.as_ref(),
        );
        let stop_sample = crate::frontmost::StopSampleStatus::of(stop_delivery_target.as_ref());
        let mut delivery_target = anchored_target.clone();
        let mut smart_destination = context.app.smart_destination.clone();
        let _smart_cleanup = crate::smart_destination::AbortOnDrop::new(
            app_handle.clone(),
            smart_destination.clone(),
        );
        let accepted_route = smart_destination.route_code();
        delivery_timing.update(|r| {
            r.accepted_route =
                DeliveryMeasurement::measured(dt::accepted_route(&smart_destination));
            r.configured_delay_ms = paste_delay_ms;
        });
        let mut recovery_smart_destination = smart_destination.clone();
        let mut recovery_target = smart_destination
            .identity()
            .unwrap_or(&delivery_target)
            .clone();
        // Keep the configurable focus-settle wait off the macOS main thread so
        // activation/Space notification callbacks can advance their
        // content-free counters before target verification runs.
        if effective_auto_paste && paste_delay_ms > 0 && injector::is_accessibility_enabled() {
            let delay_timer = delivery_timing.timer(|r| &mut r.configured_delay_elapsed_us);
            tokio::time::sleep(std::time::Duration::from_millis(paste_delay_ms)).await;
            drop(delay_timer);
        }
        // Activation and insertion are separate main-thread turns: AppKit
        // refreshes its active-application state only after this turn returns.
        if effective_auto_paste
            && injector::is_accessibility_enabled()
            && matches!(
                smart_destination,
                crate::smart_destination::Snapshot::Target { .. }
            )
        {
            let (sender, receiver) = tokio::sync::oneshot::channel();
            let prepare_app = app_handle.clone();
            let preparing = smart_destination.clone();
            let preparation_authority = Arc::new(injector::authority::PasteAttempt::new(
                std::time::Duration::from_secs(1),
                recording_id,
            ));
            let _cancel_preparation = injector::authority::CancelOnDrop(&preparation_authority);
            let worker_authority = preparation_authority.clone();
            let prepare_timing = delivery_timing.clone();
            let prepare_queue = delivery_timing.timer(|r| &mut r.prepare_queue_us);
            delivery_timing.update(|r| {
                let unavailable = DeliveryMeasurement::Unavailable {
                    reason: crate::performance_metrics::UnavailableReasonV1::NoSamples,
                };
                r.prepare_queue_us = unavailable.clone();
                r.prepare_owner_wait_us = unavailable.clone();
                r.prepare_work_us = unavailable;
            });
            app_handle
                .run_on_main_thread(crate::main_thread_watch::job(
                    "smart_destination.prepare",
                    move || {
                        let _timing_scope = prepare_timing.bind();
                        drop(prepare_queue);
                        if sender.is_closed() || !worker_authority.is_current() {
                            return;
                        }
                        let state = prepare_app.state::<State>();
                        let outcome = with_current_delivery_owner_timed(
                            &state.app_state,
                            recording_id,
                            true,
                            || {
                                let _work = dt::Timer::new(|r| &mut r.prepare_work_us);
                                crate::smart_destination::prepare(
                                    &preparing,
                                    Some(&worker_authority),
                                )
                            },
                        );
                        let _ = sender.send(outcome);
                    },
                ))
                .map_err(|error| {
                    delivery_timing.update(|r| dt::set_outcome(r, dt::Outcome::DispatchFailed));
                    error.to_string()
                })?;
            match tokio::time::timeout(std::time::Duration::from_secs(1), receiver).await {
                Ok(Ok(Some(crate::smart_destination::PrepareOutcome::Ready))) => {}
                Ok(Ok(Some(crate::smart_destination::PrepareOutcome::WaitForActivation))) => {
                    let activation_timer = delivery_timing.timer(|r| &mut r.activation_wait_us);
                    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                    drop(activation_timer);
                }
                outcome => {
                    delivery_timing.update(|r| {
                        use crate::smart_destination::PrepareOutcome as P;
                        r.preparation = DeliveryMeasurement::measured(match outcome {
                            Err(_) => {
                                r.timed_out = true;
                                dt::SmartOutcome::Timeout
                            }
                            Ok(Err(_)) => dt::SmartOutcome::ReplyLost,
                            Ok(Ok(None)) => dt::SmartOutcome::StaleOwner,
                            Ok(Ok(Some(P::RefusedClipboardOnly))) => {
                                dt::SmartOutcome::RefusedClipboardOnly
                            }
                            _ => dt::SmartOutcome::Refused,
                        });
                    });
                    // Only a delivered `Refused` proves nothing was activated
                    // and no handoff refusal is pending. A timeout or lost
                    // reply may hide a late clipboard-only decision.
                    let refused_without_activation = matches!(
                        outcome,
                        Ok(Ok(Some(crate::smart_destination::PrepareOutcome::Refused)))
                    );
                    let abandoned = smart_destination.clone();
                    let abort_timing = delivery_timing.clone();
                    let _ = app_handle.run_on_main_thread(crate::main_thread_watch::job(
                        "smart_destination.abort_after_prepare",
                        move || {
                            let _scope = abort_timing.bind();
                            let _cleanup = dt::Timer::new(|r| &mut r.cleanup_us);
                            crate::smart_destination::abort(&abandoned, None);
                        },
                    ));
                    (smart_destination, delivery_target) = smart_prepare_fallback(
                        &smart_destination,
                        refused_without_activation,
                        anchored_target,
                    );
                    if smart_destination.uses_normal_paste() {
                        // Paste Last retries the same normal paste, bound to
                        // the same anchored target, not the abandoned field.
                        recovery_smart_destination = smart_destination.clone();
                        recovery_target = delivery_target.clone();
                    }
                }
            }
        }
        let delivery_route = smart_destination.route_code();
        delivery_timing.update(|r| {
            r.actual_route =
                dt::actual_route(&smart_destination, effective_auto_paste, &r.preparation)
        });
        let (tx, rx) = tokio::sync::oneshot::channel::<(
            Option<Result<injector::InjectionResult, String>>,
            std::time::Instant,
        )>();
        let injection_app_handle = app_handle.clone();
        let injection_authority = Arc::new(injector::authority::PasteAttempt::new(
            std::time::Duration::from_secs(2),
            recording_id,
        ));
        let _cancel_injection = injector::authority::CancelOnDrop(&injection_authority);
        let worker_authority = injection_authority.clone();
        let inject_timing = delivery_timing.clone();
        let inject_queue = delivery_timing.timer(|r| &mut r.inject_queue_us);
        // Joins this paste's main-thread wait to what occupied the thread;
        // submitted when this scope ends, after the reply or timeout.
        let paste_probe = crate::main_thread_watch::PasteProbe::new(recording_id);
        let worker_probe = paste_probe.worker();
        // The preview hide is queued only after the paste job, so it can never
        // sit ahead of the paste on the main thread.
        crate::commands::dictation_preview::dispatch_then_hide(
            || {
                app_handle
                    .run_on_main_thread(crate::main_thread_watch::job(
                        "injector.paste",
                        move || {
                            worker_probe.started();
                            let _timing_scope = inject_timing.bind();
                            drop(inject_queue);
                            let state = injection_app_handle.state::<State>();
                            let attempt = with_current_delivery_owner_timed(
                                &state.app_state,
                                recording_id,
                                false,
                                || {
                                    let _work = dt::Timer::new(|r| &mut r.inject_work_us);
                                    injector::inject_with_destination_authorized(
                                        &text_to_inject,
                                        effective_auto_paste,
                                        paste_delay_ms,
                                        &delivery_target,
                                        &smart_destination,
                                        Some(&worker_authority),
                                    )
                                },
                            );
                            {
                                let _cleanup = dt::Timer::new(|r| &mut r.cleanup_us);
                                crate::smart_destination::abort(&smart_destination, None);
                            }
                            match &attempt {
                                Some(Ok(result)) => {
                                    if let Some(evidence) = result.verification {
                                        emit_delivery_target_verification(
                                            recording_id,
                                            delivery_anchor,
                                            stop_sample,
                                            evidence,
                                        );
                                    }
                                }
                                None => emit_delivery_target_verification(
                                    recording_id,
                                    delivery_anchor,
                                    stop_sample,
                                    crate::frontmost::verify_delivery_target(
                                        &delivery_target,
                                        false,
                                    ),
                                ),
                                Some(Err(_)) => {}
                            }
                            worker_probe.finished();
                            let _ = tx.send((attempt, std::time::Instant::now()));
                        },
                    ))
                    .map_err(|e| {
                        delivery_timing.update(|r| dt::set_outcome(r, dt::Outcome::DispatchFailed));
                        format!("Failed to dispatch to main thread: {}", e)
                    })
            },
            preview_hide.take(),
        )?;
        let paste_hint = if cfg!(target_os = "macos") {
            "Text is in your clipboard -- press Cmd+V to paste manually."
        } else {
            "Text is in your clipboard -- press Ctrl+V to paste manually."
        };
        let mut injection_outcome_code = "unconfirmed";
        let received = tokio::time::timeout(std::time::Duration::from_secs(2), rx).await;
        let receiver_started = match &received {
            Ok(Ok((_, sent_at))) => *sent_at,
            _ => std::time::Instant::now(),
        };
        let delivery_outcome = match received {
            Ok(Ok((Some(Ok(result)), _))) => {
                delivery_timing
                    .update(|r| dt::set_outcome(r, dt::injection_outcome(result.outcome)));
                injection_outcome_code = result.outcome.code();
                if !matches!(
                    result.outcome,
                    injector::InjectionOutcome::NoText | injector::InjectionOutcome::Stopped(_)
                ) {
                    app_handle.state::<State>().delivery_recovery.remember(
                        recording_id,
                        text.clone(),
                        recovery_target,
                        recovery_smart_destination,
                        paste_delay_ms,
                    );
                }
                if let injector::InjectionOutcome::ClipboardOnly(reason) = result.outcome {
                    let failure_message = match reason {
                        injector::ClipboardOnlyReason::SmartTargetUnavailable => Some(
                            "The intended text field could not be verified. Click your destination and press Cmd+V.",
                        ),
                        injector::ClipboardOnlyReason::TargetChanged => Some(
                            "App focus changed. Text is in your clipboard; paste it when ready.",
                        ),
                        injector::ClipboardOnlyReason::ClipboardChanged => Some(
                            "The clipboard changed before Murmur could retry. Copy the transcription from History.",
                        ),
                        injector::ClipboardOnlyReason::FocusNotEditable
                        | injector::ClipboardOnlyReason::PasteFailed => Some(paste_hint),
                        injector::ClipboardOnlyReason::AutoPasteDisabled
                        | injector::ClipboardOnlyReason::AccessibilityDenied => None,
                    };
                    if let Some(message) = failure_message {
                        emit_auto_paste_failure_if_current(
                            app_handle,
                            app_state,
                            recording_id,
                            message,
                        );
                    }
                }
                if let injector::InjectionOutcome::Stopped(reason) = result.outcome {
                    app_handle
                        .state::<State>()
                        .delivery_recovery
                        .forget_through(recording_id);
                    tracing::info!(target: "pipeline", reason_code = reason.code(), "Text delivery stopped before completion");
                    emit_auto_paste_failure_if_current(app_handle, app_state, recording_id,
                        "Text delivery stopped before completion. Check the destination and clipboard before pasting.");
                }
                confirmed_delivery_outcome(result.outcome)
            }
            Ok(Ok((Some(Err(e)), _))) => {
                delivery_timing.update(|r| dt::set_outcome(r, dt::Outcome::ClipboardWriteFailed));
                tracing::error!(target: "pipeline", "Text injection failed before clipboard delivery: {}", e);
                injection_outcome_code = "clipboard_write_failed";
                emit_auto_paste_failure_if_current(
                    app_handle,
                    app_state,
                    recording_id,
                    "Couldn't copy the transcription to the clipboard.",
                );
                DictationDeliveryOutcome::ClipboardWriteFailed
            }
            Ok(Ok((None, _))) => {
                delivery_timing.update(|r| {
                    dt::set_outcome(r, dt::Outcome::StaleOwner);
                    r.receiver_and_outcome_us =
                        DeliveryMeasurement::measured(dt::us(receiver_started));
                });
                tracing::info!(
                    target: "pipeline",
                    recording_id,
                    "cancelled before queued text injection"
                );
                timings.correction_ms = correction_ms;
                timings.transform_ms = transform_ms;
                timings.transform_stages = transform_stages;
                timings.file_output_ms = file_output_ms;
                timings.paste_ms = t_inject.elapsed().as_millis() as u64;
                return Ok(PipelineResult {
                    final_input: None,
                    text: String::new(),
                    timings,
                    terminal: PipelineTerminal::Cancelled(PerformanceStageV1::ClipboardPaste),
                    history: None,
                });
            }
            Ok(Err(_)) => {
                app_handle
                    .state::<State>()
                    .delivery_recovery
                    .forget_through(recording_id);
                delivery_timing.update(|r| dt::set_outcome(r, dt::Outcome::ReplyLost));
                tracing::warn!(target: "pipeline", "Text injection sender dropped");
                emit_auto_paste_failure_if_current(
                    app_handle,
                    app_state,
                    recording_id,
                    "Text delivery did not finish. Check the clipboard before pasting.",
                );
                DictationDeliveryOutcome::Unconfirmed
            }
            Err(_) => {
                app_handle
                    .state::<State>()
                    .delivery_recovery
                    .forget_through(recording_id);
                delivery_timing.update(|r| dt::set_outcome(r, dt::Outcome::Timeout));
                tracing::warn!(target: "pipeline", "Text injection timed out");
                emit_auto_paste_failure_if_current(
                    app_handle,
                    app_state,
                    recording_id,
                    "Text delivery timed out. Check the clipboard before pasting.",
                );
                DictationDeliveryOutcome::Unconfirmed
            }
        };
        tracing::info!(
            target: "pipeline",
            event_code = "pipeline.smart_destination_delivery",
            recording_id,
            accepted_route,
            delivery_route,
            outcome_code = injection_outcome_code,
            "smart destination delivery"
        );
        timings.delivery_confirmed = delivery_confirms_paste(delivery_outcome);
        // Hold the recording-state lock across ownership validation and event
        // publication. A concurrent cancel changes Processing to Idle under
        // this same lock before marking the ID cancelled, and a new start also
        // advances the ID under it, so neither can interleave a stale outcome.
        {
            let dictation = app_state.dictation.lock_or_recover();
            if let Some(owner_recording_id) = delivery_event_recording_id(
                recording_id,
                app_state.recording_id.load(Ordering::SeqCst),
                app_state.is_cancelled(recording_id),
                dictation.status,
            ) {
                let _ = app_handle.emit(
                    "dictation-delivery-outcome",
                    serde_json::json!({
                        "recordingId": owner_recording_id,
                        "outcome": delivery_outcome.as_event_value(),
                    }),
                );
            }
        }
        delivery_timing.update(|r| {
            r.receiver_and_outcome_us = DeliveryMeasurement::measured(dt::us(receiver_started))
        });
        // Opt-in mirror to NotchPill: best-effort local file write. It runs
        // *after* the clipboard write so disk I/O can never delay the primary
        // output path, and off the pipeline thread so it stays out of paste_ms.
        //
        if delivery.mirror_to_notchpill && !app_state.is_cancelled(recording_id) {
            let caption = text.clone();
            let mirror_app_handle = app_handle.clone();
            tokio::task::spawn_blocking(move || {
                let state = mirror_app_handle.state::<State>();
                let installed = super::integrations::notchpill_installed();
                let prepared = installed
                    .then(|| injector::prepare_mirrored_caption(&caption))
                    .flatten();
                let _ =
                    with_current_recording_generation(&state.app_state, recording_id, move || {
                        if installed {
                            if let Some(prepared) = prepared {
                                let _ = prepared.publish();
                            }
                        } else {
                            // The preference may survive an uninstall so it can
                            // resume after a reinstall, but no speech should
                            // remain mirrored while the companion app is absent.
                            injector::remove_published_mirrored_caption();
                        }
                    });
            });
        }
    }
    if text.is_empty() {
        delivery_timing.update(|r| dt::set_outcome(r, dt::Outcome::NoText));
    }
    let paste_ms = t_inject.elapsed().as_millis() as u64;
    gaps.delivery_us = micros(t_inject);
    tracing::info!(target: "pipeline", "inject (clipboard + paste): {:?}", t_inject.elapsed());

    timings.correction_ms = correction_ms;
    timings.transform_ms = transform_ms;
    timings.transform_stages = transform_stages;
    timings.file_output_ms = file_output_ms;
    timings.paste_ms = paste_ms;
    gaps.ended_at = Some(std::time::Instant::now());
    timings.gaps = gaps;
    let terminal = if text.trim().is_empty() {
        PipelineTerminal::NoSpeech(backend_empty_error.unwrap_or(DictationErrorCode::EmptyOutput))
    } else {
        PipelineTerminal::Success
    };
    Ok(PipelineResult {
        final_input: Some(samples_for_transcription),
        text,
        timings,
        terminal,
        history: Some(history),
    })
    // _guard drops here, setting status to Idle
}

#[tauri::command]
pub async fn init_dictation(_state: tauri::State<'_, State>) -> Result<serde_json::Value, String> {
    tracing::info!(target: "pipeline", "init_dictation");
    Ok(serde_json::json!({
        "type": "initialized",
        "state": "idle"
    }))
}

#[tauri::command]
pub async fn process_audio(
    app_handle: tauri::AppHandle,
    audio_data: String,
    state: tauri::State<'_, State>,
) -> Result<serde_json::Value, String> {
    let transition = crate::commands::microphone_preview::transition_after_stopping_preview(
        &app_handle,
        state.inner(),
    )
    .await?;
    if state.app_state.meeting_blocks_asr() {
        return Err("Cannot process audio while a meeting transcript is active.".to_string());
    }
    let (rid, delivery_target, smart_enabled) = {
        let mut dictation = state.app_state.dictation.lock_or_recover();
        // Same mutual exclusion as start_native_recording: this legacy base64
        // path also runs the shared Whisper backend, so refuse while a file
        // transcription holds the slot. Checked under the dictation lock.
        if state.app_state.file_transcribing.load(Ordering::SeqCst) {
            tracing::warn!(target: "pipeline", "process_audio: blocked — file transcription in progress");
            return Err(
                "Cannot process audio while a file transcription is in progress.".to_string(),
            );
        }
        if state.benchmark.is_running() {
            tracing::warn!(target: "pipeline", "process_audio: blocked — benchmark in progress");
            return Err("Cannot process audio while a benchmark is in progress.".to_string());
        }

        if dictation.status != DictationStatus::Idle {
            return Err("Cannot process audio while live dictation is active.".to_string());
        }
        let recording_id = state.app_state.next_recording_id();
        transition_dictation_status(recording_id, &mut dictation, DictationStatus::Processing);
        let delivery_target = crate::frontmost::capture_delivery_target_snapshot();
        (
            recording_id,
            delivery_target,
            smart_destination_eligible(&dictation),
        )
    };
    drop(transition);
    keyboard::set_processing(true);
    let _ = app_handle.emit(
        "dictation-generation-started",
        serde_json::json!({ "recordingId": rid }),
    );
    let _ = app_handle.emit("recording-status-changed", "processing");
    // Legacy process_audio has a single API acceptance boundary: audio has
    // already been recorded by its caller. Freeze its destination here, when
    // processing is accepted; the native recording path freezes at start.
    let smart_destination =
        crate::smart_destination::capture(&app_handle, &delivery_target, smart_enabled).await;
    let capture_is_current = {
        let dictation = state.app_state.dictation.lock_or_recover();
        recording_capture_is_current(
            rid,
            state.app_state.recording_id.load(Ordering::SeqCst),
            state.app_state.is_cancelled(rid),
            dictation.status,
        )
    };
    if !capture_is_current {
        tracing::info!(
            target: "pipeline",
            recording_id = rid,
            "process_audio: smart destination capture superseded"
        );
        return Ok(serde_json::json!({
            "type": "transcription",
            "text": "",
            "state": "idle"
        }));
    }
    let destination = select_recording_destination(&delivery_target, smart_destination);
    let context = resolve_recording_context(&state.app_state, destination);
    if smart_enabled {
        emit_smart_destination(
            &app_handle,
            &state.app_state,
            rid,
            &context,
            &delivery_target,
        );
    }
    if let Err(error) = state.performance.begin_dictation_diagnosed(
        rid,
        runtime_identity(&context.transcription.model_name, ModelWarmStateV1::Unknown),
    ) {
        emit_performance_store_begin_failure(rid, &error);
    }
    let mut performance_guard = state.performance.guard(
        RunCorrelationV1::Dictation { recording_id: rid },
        PerformanceStageV1::CaptureFinalization,
    );

    // Guard resets status to Idle if decode/parse fails before reaching the pipeline
    let mut guard = IdleGuard::new(&state.app_state, rid);

    // Phase: Audio parse (base64 decode + WAV to samples)
    let t_parse = std::time::Instant::now();
    let wav_bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &audio_data)
        .map_err(|e| {
            guard.finish(|_| {
                let _ = app_handle.emit("recording-status-changed", "idle");
            });
            format!("Failed to decode base64: {}", e)
        })?;
    let samples = transcriber::parse_wav_to_samples(&wav_bytes).inspect_err(|_e| {
        guard.finish(|_| {
            let _ = app_handle.emit("recording-status-changed", "idle");
        });
    })?;
    tracing::info!(target: "pipeline", "audio parse (base64 + WAV): {:?}", t_parse.elapsed());
    performance_guard.record(StageTimingV1::measured(
        PerformanceStageV1::CaptureFinalization,
        t_parse.elapsed().as_millis() as u64,
    ));
    performance_guard.enter(PerformanceStageV1::Vad);

    // Pipeline has its own guard, so disarm this one
    guard.disarm();

    let t_total = std::time::Instant::now();
    let pipeline_result = run_transcription_pipeline(
        &samples,
        &app_handle,
        &state.app_state,
        rid,
        &mut performance_guard,
        Arc::clone(&context),
        // Imported audio has no live stop transition: its snapshot is already
        // taken at the accepted Idle -> Processing transition.
        PipelineStopContext::default(),
    )
    .await;
    // Only emit idle if this recording wasn't cancelled/superseded.
    // Hold the dictation lock across the check+emit to prevent a concurrent
    // start from interleaving a "recording" status between our check and emit.
    {
        let _dictation = state.app_state.dictation.lock_or_recover();
        if state.app_state.recording_id.load(Ordering::SeqCst) == rid {
            keyboard::set_processing(false);
            let _ = app_handle.emit("recording-status-changed", "idle");
        }
    }
    let pipeline = pipeline_result?;
    let text = pipeline.text;
    let timings = pipeline.timings;

    let total_ms = t_total.elapsed().as_millis() as u64;
    let audio_secs = samples.len() as f64 / 16_000.0;
    let word_count = if text.trim().is_empty() {
        0
    } else {
        text.split_whitespace().count()
    };
    let char_count = text.len();
    let model_name = context.transcription.model_name.clone();
    let backend_name = model_runtime::model_definition(&model_name)?
        .backend
        .as_str()
        .to_string();
    tracing::info!(
        target: "pipeline",
        recording_id = rid,
        vad_ms = timings.vad_ms,
        model_load_ms = timings.model_load_ms,
        decode_ms = timings.decode_ms,
        inference_ms = timings.inference_ms,
        correction_ms = timings.correction_ms,
        paste_ms = timings.paste_ms,
        total_ms = total_ms,
        audio_secs = audio_secs,
        word_count = word_count,
        char_count = char_count,
        rss_before_mb = timings.rss_before_mb,
        rss_after_mb = timings.rss_after_mb,
        model = model_name.as_str(),
        backend = backend_name.as_str(),
        "transcription complete"
    );

    let outcome = match pipeline.terminal {
        PipelineTerminal::Success => RunOutcomeV1::Success,
        PipelineTerminal::NoSpeech(_) => RunOutcomeV1::NoSpeech,
        PipelineTerminal::Cancelled(stage) => RunOutcomeV1::Cancelled { stage },
    };
    let warm_state = timings.warm_state.unwrap_or(ModelWarmStateV1::Unknown);
    let _ = performance_guard.finish(
        outcome,
        pipeline_stages(&timings, total_ms),
        Some(ContentFreeInputSummaryV1::audio_with_output(
            (audio_secs * 1_000.0).round() as u64,
            text.len(),
        )),
        Some(runtime_identity(&model_name, warm_state)),
    );

    Ok(serde_json::json!({
        "type": "transcription",
        "text": text
    }))
}

#[tauri::command]
pub async fn get_status(state: tauri::State<'_, State>) -> Result<serde_json::Value, String> {
    let dictation = state.app_state.dictation.lock_or_recover();
    Ok(serde_json::json!({
        "type": "status",
        "state": dictation.status,
        "model": dictation.model_name,
        "language": dictation.language
    }))
}

/// Privacy-safe shape for configuration telemetry. User-entered values (profile
/// identities, vocabulary, command text, and filesystem paths) must never be
/// formatted into a tracing message or field because pretty logs retain strings.
#[derive(Debug, PartialEq, Eq)]
struct ConfigurationLogMetadata {
    option_count: u64,
    app_profile_count: u64,
    custom_vocabulary_present: bool,
    vocabulary_entry_count: u64,
    output_directory_present: bool,
}

impl ConfigurationLogMetadata {
    fn from_options(options: &serde_json::Value) -> Self {
        Self {
            option_count: options.as_object().map_or(0, |object| object.len() as u64),
            app_profile_count: options
                .get("appProfiles")
                .and_then(serde_json::Value::as_array)
                .map_or(0, |profiles| profiles.len() as u64),
            custom_vocabulary_present: options
                .get("customVocabulary")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|vocabulary| !vocabulary.trim().is_empty())
                || options
                    .get("vocabularyEntries")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|entries| !entries.is_empty()),
            vocabulary_entry_count: options
                .get("vocabularyEntries")
                .and_then(serde_json::Value::as_array)
                .map_or(0, |entries| entries.len() as u64),
            output_directory_present: options
                .get("outputDir")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|directory| !directory.trim().is_empty()),
        }
    }
}

fn parse_writing_style(value: Option<&serde_json::Value>) -> Option<crate::state::WritingStyle> {
    match value.and_then(serde_json::Value::as_str) {
        Some("conversational") => Some(crate::state::WritingStyle::Conversational),
        Some("polished") => Some(crate::state::WritingStyle::Polished),
        Some("code_technical") => Some(crate::state::WritingStyle::CodeTechnical),
        Some("verbatim") => Some(crate::state::WritingStyle::Verbatim),
        Some("notes") => Some(crate::state::WritingStyle::Notes),
        // Missing, null, malformed, and the legacy-facing Inherit spelling all
        // preserve the existing resolver path.
        _ => None,
    }
}

struct StagedVocabularyConfiguration {
    entries: Option<Vec<crate::state::VocabularyEntry>>,
}

impl StagedVocabularyConfiguration {
    fn commit(self, dictation: &mut crate::state::DictationState) {
        if let Some(entries) = self.entries {
            dictation.custom_vocabulary =
                crate::vocabulary_alias::prompt_terms(&entries, None, &dictation.app_profiles);
            dictation.vocabulary_entries = entries;
        }
    }
}

fn legacy_vocabulary_entries(value: &str) -> Vec<crate::state::VocabularyEntry> {
    parse_vocab_terms(value)
        .into_iter()
        .enumerate()
        .map(|(index, written)| crate::state::VocabularyEntry {
            id: format!("legacy-{index}"),
            written,
            aliases: Vec::new(),
            enabled: true,
            scope: crate::state::VocabularyScope::Global,
        })
        .collect()
}

fn stage_vocabulary_configuration(
    options: &serde_json::Value,
    dictation: &crate::state::DictationState,
) -> Result<StagedVocabularyConfiguration, String> {
    let entries = options
        .get("vocabularyEntries")
        .map(|value| {
            serde_json::from_value::<Vec<crate::state::VocabularyEntry>>(value.clone())
                .map_err(|_| "Vocabulary entries have an invalid shape.".to_string())
        })
        .transpose()?
        .or_else(|| {
            options
                .get("customVocabulary")
                .and_then(serde_json::Value::as_str)
                .map(legacy_vocabulary_entries)
        });

    let effective_entries = entries.as_deref().unwrap_or(&dictation.vocabulary_entries);
    crate::vocabulary_alias::validate_entries(effective_entries)?;

    Ok(StagedVocabularyConfiguration { entries })
}

pub(crate) fn sanitize_app_profiles(
    profiles: &[serde_json::Value],
) -> Vec<crate::state::AppProfile> {
    profiles
        .iter()
        .filter_map(|p| {
            let bundle_id = p
                .get("bundleId")
                .and_then(|v| v.as_str())?
                .trim()
                .to_string();
            if bundle_id.is_empty() {
                return None;
            }
            let label = p
                .get("label")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            // null/absent -> None (use global); otherwise the boolean override.
            let auto_paste_override = p.get("autoPasteOverride").and_then(|v| v.as_bool());
            let cleanup_override = p.get("cleanupOverride").and_then(|v| v.as_bool());
            let cli_formatting_override = p.get("cliFormattingOverride").and_then(|v| v.as_bool());
            let smart_formatting_override =
                p.get("smartFormattingOverride").and_then(|v| v.as_bool());
            let writing_style = parse_writing_style(p.get("writingStyle"));
            let ide_context_enabled = p
                .get("ideContextEnabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let ide_project_roots = p
                .get("ideProjectRoots")
                .and_then(|value| value.as_array())
                .map(|roots| {
                    let roots = roots
                        .iter()
                        .filter_map(|root| root.as_str())
                        .map(str::trim)
                        .filter(|root| !root.is_empty())
                        .map(str::to_string)
                        .collect::<Vec<_>>();
                    crate::ide_context::normalize_config_roots(&roots)
                })
                .unwrap_or_default();
            Some(crate::state::AppProfile {
                bundle_id,
                label,
                auto_paste_override,
                cleanup_override,
                cli_formatting_override,
                smart_formatting_override,
                writing_style,
                ide_context_enabled,
                ide_project_roots,
            })
        })
        .collect()
}

#[tauri::command]
pub async fn configure_dictation(
    options: serde_json::Value,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, State>,
) -> Result<serde_json::Value, String> {
    crate::audio::prepare::configuration_changed();
    crate::launch::frontend_configuring();
    let log_metadata = ConfigurationLogMetadata::from_options(&options);
    tracing::info!(
        target: "pipeline",
        option_count = log_metadata.option_count,
        app_profile_count = log_metadata.app_profile_count,
        custom_vocabulary_present = log_metadata.custom_vocabulary_present,
        vocabulary_entry_count = log_metadata.vocabulary_entry_count,
        output_directory_present = log_metadata.output_directory_present,
        "configure_dictation"
    );

    let model = options
        .get("model")
        .and_then(|v| v.as_str())
        .map(String::from);
    let language = options
        .get("language")
        .and_then(|v| v.as_str())
        .map(String::from);
    if let Some(requested) = model.as_deref() {
        let definition = model_runtime::model_definition(requested)?;
        if !model_runtime::model_supported(definition) {
            return Err("This model is not supported on the current platform".to_string());
        }
    }

    let mut dictation = state.app_state.dictation.lock_or_recover();
    let backend_change_can_apply_now = dictation.status == DictationStatus::Idle;
    let staged_vocabulary = stage_vocabulary_configuration(&options, &dictation)?;

    let mut model_change_guard = if model
        .as_deref()
        .is_some_and(|requested| requested != dictation.model_name)
    {
        if !state.benchmark.try_start_shared_backend_change() {
            return Err(
                "Wait for the benchmark or current model preparation to finish before changing models."
                    .to_string(),
            );
        }
        crate::meeting_suggestions::busy_changed(&app_handle);
        Some(SharedBackendChangeGuard(
            state.benchmark.clone(),
            Some(app_handle.clone()),
        ))
    } else {
        None
    };

    // Commit vocabulary only after the requested entries have validated.
    staged_vocabulary.commit(&mut dictation);

    let model_changed = if let Some(m) = model {
        if m != dictation.model_name {
            dictation.model_name = m;
            true
        } else {
            false
        }
    } else {
        false
    };

    if let Some(l) = language {
        dictation.language = l;
    }

    if let Some(enabled) = options
        .get("smartDestinationEnabled")
        .and_then(|v| v.as_bool())
    {
        dictation.smart_destination_enabled = enabled;
    }
    if let Some(auto_paste) = options.get("autoPaste").and_then(|v| v.as_bool()) {
        dictation.auto_paste = auto_paste;
    }
    if let Some(mirror) = options.get("mirrorToNotchPill").and_then(|v| v.as_bool()) {
        dictation.mirror_to_notchpill = mirror;
        // Disabled or unavailable mirroring removes what it left behind. The
        // file holds a verbatim record of the last thing the user said, so
        // leaving it on disk would keep speech around past consent -- the same
        // reasoning that makes it 0600 in the first place. Best-effort and
        // errors swallowed, exactly like the write.
        if !mirror || !super::integrations::notchpill_installed() {
            injector::remove_mirrored_caption();
        }
    }

    if let Some(delay) = options.get("autoPasteDelayMs").and_then(|v| v.as_u64()) {
        dictation.auto_paste_delay_ms = delay.min(500);
    }

    if let Some(sensitivity) = options.get("vadSensitivity").and_then(|v| v.as_u64()) {
        dictation.vad_sensitivity = (sensitivity as u32).clamp(0, 100);
    }

    if let Some(sp) = options.get("smartPunctuation").and_then(|v| v.as_bool()) {
        dictation.smart_punctuation = sp;
    }

    if let Some(vc) = options
        .get("voiceCommandsEnabled")
        .and_then(|v| v.as_bool())
    {
        dictation.voice_commands_enabled = vc;
    }

    if let Some(save_transcript) = options.get("saveTranscript").and_then(|v| v.as_bool()) {
        dictation.save_transcript = save_transcript;
    }

    if let Some(save_audio) = options.get("saveAudio").and_then(|v| v.as_bool()) {
        dictation.save_audio = save_audio;
    }

    if let Some(output_dir) = options.get("outputDir").and_then(|v| v.as_str()) {
        dictation.output_dir = output_dir.to_string();
    }

    // Per-app profiles carry nullable delivery/transformation overrides. A
    // missing/null value means "no override". Entries without a bundleId are
    // skipped. Replaces the whole list when the key is present.
    if let Some(profiles) = options.get("appProfiles").and_then(|v| v.as_array()) {
        dictation.app_profiles = sanitize_app_profiles(profiles);
        let requests = state
            .app_state
            .ide_context
            .lock_or_recover()
            .reconcile_profiles(&dictation.app_profiles);
        for request in requests {
            schedule_ide_scan(app_handle.clone(), request);
        }
    }

    if let Some(cleanup_enabled) = options.get("cleanupEnabled").and_then(|v| v.as_bool()) {
        dictation.cleanup_enabled = cleanup_enabled;
    }

    if let Some(enabled) = options
        .get("smartFormattingEnabled")
        .and_then(|v| v.as_bool())
    {
        dictation.smart_formatting_enabled = enabled;
    }

    if let Some(v) = options.get("cleanupRemoveFiller").and_then(|v| v.as_bool()) {
        dictation.cleanup_remove_filler = v;
    }

    if let Some(v) = options.get("cleanupCapitalize").and_then(|v| v.as_bool()) {
        dictation.cleanup_capitalize = v;
    }

    // Code-aware vocabulary: a toggle plus a folder to scan for identifiers.
    // Changing either invalidates the cached prompt so the next transcription
    // (or the explicit prebuild below) rescans. Disabling clears the cache so we
    // don't hold a stale prompt in memory.
    let mut code_vocab_dirty = false;
    if let Some(enabled) = options.get("codeVocabEnabled").and_then(|v| v.as_bool()) {
        if enabled != dictation.code_vocab_enabled {
            dictation.code_vocab_enabled = enabled;
            code_vocab_dirty = true;
        }
    }
    if let Some(folder) = options.get("codeVocabFolder").and_then(|v| v.as_str()) {
        if folder != dictation.code_vocab_folder {
            dictation.code_vocab_folder = folder.to_string();
            code_vocab_dirty = true;
        }
    }
    if code_vocab_dirty {
        // Invalidate the cached prompt; do NOT walk the tree synchronously here.
        // The explicit `scan_code_vocab` command owns the (single) walk for a
        // folder pick and caches the result, so prebuilding under the lock here
        // would just duplicate that walk — and choosing a folder fires both this
        // configure path and a scan concurrently. If no scan ever runs (folder set
        // programmatically), `resolve_code_vocab_prompt` lazily builds + caches the
        // prompt on the first transcription. Net: exactly one walk per folder.
        dictation.code_vocab_prompt = None;
        dictation.code_vocab_scan_id = None;
    }

    // Post-model correction toggles.
    if let Some(v) = options.get("correctionEnabled").and_then(|v| v.as_bool()) {
        dictation.correction_enabled = v;
    }
    if let Some(v) = options.get("correctionFuzzy").and_then(|v| v.as_bool()) {
        dictation.correction_fuzzy = v;
    }

    if let Some(v) = options
        .get("homophoneContextEnabled")
        .and_then(|v| v.as_bool())
    {
        dictation.homophone_context_enabled = v;
    }

    // Streaming setting (E1 incremental VAD). Applies to the next recording.
    // Configure runs at every launch and settings change, so it never fetches
    // the model: the toggle's own Download/Retry actions do, on user request.
    // The session is released on a blocking worker, not under this lock.
    if let Some(v) = options
        .get("streamingPipelineEnabled")
        .and_then(|v| v.as_bool())
    {
        let was_enabled = std::mem::replace(&mut dictation.streaming_pipeline_enabled, v);
        if was_enabled && !v {
            // Fire and forget: `unload` takes the session lock, which must
            // never be waited on while the dictation lock is held.
            drop(tauri::async_runtime::spawn_blocking(
                crate::vad_stream::unload,
            ));
        }
    }

    // Rebuild the correction matcher from the (now-updated) unified vocab +
    // correction settings. Built here on settings-change, never per-utterance.
    rebuild_correction_matcher(&state.app_state, &dictation);
    state.app_state.bump_settings_revision();
    crate::smart_destination::set_enabled(smart_destination_eligible(&dictation));

    if let Some(enabled) = options
        .get("captureStandbyEnabled")
        .and_then(|v| v.as_bool())
    {
        audio_lifecycle::configure_standby(
            enabled,
            options
                .get("captureStandbySeconds")
                .and_then(|v| v.as_u64())
                .unwrap_or(300)
                .min(900) as u32,
            options
                .get("capturePreRollMs")
                .and_then(|v| v.as_u64())
                .unwrap_or(450)
                .min(450) as u32,
        );
    }

    if let Some(idle_timeout) = options.get("idleTimeoutMinutes").and_then(|v| v.as_i64()) {
        *state.app_state.model_idle_policy.lock_or_recover() =
            crate::state::ModelIdlePolicy::from_setting(idle_timeout);
    }

    // If model changed, swap/reset the backend so the next transcription loads
    // the right engine for the selected model.
    let mut idle_preparation = None;
    if model_changed && backend_change_can_apply_now {
        let new_model = dictation.model_name.clone();
        if model_runtime::model_definition(&new_model)?.warm_on_startup {
            idle_preparation = Some(new_model.clone());
        }
        drop(dictation); // Release dictation lock first
        state
            .app_state
            .model_runtime
            .select_model(Some(&app_handle), &new_model)?;
    } else if model_changed {
        tracing::info!(
            target: "pipeline",
            "model backend change deferred until the next recording generation"
        );
    }

    if let Some(model_name) = idle_preparation {
        // Treat warmup as activity so an already-expired idle timer cannot
        // immediately release the model this preparation is about to load.
        *state.app_state.last_transcription_at.lock_or_recover() = Some(std::time::Instant::now());
        spawn_idle_model_preparation(
            app_handle,
            model_name,
            model_change_guard
                .take()
                .expect("model changes hold the shared backend lease"),
        );
    }

    Ok(serde_json::json!({
        "type": "configured"
    }))
}

/// Live progress emitted (throttled) while [`scan_code_vocab`] walks a folder.
/// Field names are part of the frontend contract — do not rename.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VocabScanProgress {
    scan_id: String,
    current_path: String,
    files_read: usize,
    dirs_skipped: usize,
    terms_so_far: usize,
    done: bool,
    adopted: bool,
}

/// One ranked vocabulary term surfaced to the frontend pop-out: the written form
/// plus how many times it appeared across the scan. Serialized camelCase (already
/// single-word, so `term`/`freq` are unchanged). Rank is the array index + 1.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RankedTermJson {
    term: String,
    freq: u32,
}

/// Result of a [`scan_code_vocab`] run, returned to the frontend. Field names are
/// part of the frontend contract — do not rename.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VocabScanSummary {
    files: usize,
    skipped: usize,
    terms: usize,
    bytes: u64,
    capped: bool,
    ms: u64,
    /// Top ~12 written forms — a preview of what the scan biases toward.
    sample_terms: Vec<String>,
    /// Full ranked list of terms actually kept (<= [`CORRECTION_TERMS`]), ordered
    /// by descending frequency. Rank = array index + 1. Powers the "all scanned
    /// terms" pop-out and is the exact list fed to Smart Correction.
    ranked_terms: Vec<RankedTermJson>,
    /// How many of `ranked_terms` feed Whisper's initial prompt =
    /// min([`WHISPER_PROMPT_TERMS`], ranked_terms.len()). The first `whisper_count`
    /// entries are the Whisper budget; the rest are correction-only.
    whisper_count: usize,
    /// Whether this result was adopted into the live dictation settings. False
    /// means a newer scan or settings change superseded it while it was walking.
    adopted: bool,
}

/// Register an explicit scan as the latest settings intent before walking. This
/// makes the command own the folder transition even when the frontend's
/// configure call is still in flight, while the scan id lets later changes
/// supersede it deterministically.
fn begin_code_vocab_scan(app_state: &AppState, scan_id: &str, folder: &str) {
    let mut dictation = app_state.dictation.lock_or_recover();
    let folder_changed = dictation.code_vocab_folder != folder;
    let was_disabled = !dictation.code_vocab_enabled;
    dictation.code_vocab_enabled = true;
    dictation.code_vocab_folder = folder.to_string();
    dictation.code_vocab_scan_id = Some(scan_id.to_string());
    if folder_changed || was_disabled {
        dictation.code_vocab_prompt = None;
        rebuild_correction_matcher(app_state, &dictation);
        app_state.bump_settings_revision();
    }
}

/// Adopt a completed scan only when it is still the latest run and the settings
/// still target the folder it walked. Returns false for overlapping scans and
/// for enable/folder changes made during the walk.
fn complete_code_vocab_scan(
    app_state: &AppState,
    scan_id: &str,
    folder: &str,
    prompt: String,
) -> bool {
    let mut dictation = app_state.dictation.lock_or_recover();
    let is_active = dictation.code_vocab_scan_id.as_deref() == Some(scan_id);
    let adopted =
        is_active && dictation.code_vocab_enabled && dictation.code_vocab_folder == folder;
    if adopted {
        dictation.code_vocab_prompt = Some(prompt);
        dictation.code_vocab_scan_id = None;
        rebuild_correction_matcher(app_state, &dictation);
        app_state.bump_settings_revision();
    } else if is_active {
        dictation.code_vocab_scan_id = None;
    }
    adopted
}

/// Invalidate one explicit scan without disturbing a newer overlapping scan.
fn cancel_code_vocab_scan_id(app_state: &AppState, scan_id: &str) -> bool {
    let mut dictation = app_state.dictation.lock_or_recover();
    if dictation.code_vocab_scan_id.as_deref() != Some(scan_id) {
        return false;
    }
    dictation.code_vocab_scan_id = None;
    true
}

#[tauri::command]
pub fn cancel_code_vocab_scan(state: tauri::State<'_, State>, scan_id: String) -> bool {
    cancel_code_vocab_scan_id(&state.app_state, &scan_id)
}

/// Emit a throttled `vocab-scan-progress` tick carrying the live running counts
/// and the path being read/skipped. `force` bypasses the throttle (used for skip
/// rows so struck-through dependency/build dirs always render live). Updates
/// `*last_emit` when it fires. A free fn (not a closure) so both walk callbacks
/// can call it without one capturing the other and tripping the borrow checker.
#[allow(clippy::too_many_arguments)]
fn emit_scan_progress(
    handle: &tauri::AppHandle,
    scan_id: &str,
    last_emit: &mut std::time::Instant,
    current_path: String,
    files_read: usize,
    dirs_skipped: usize,
    terms_so_far: usize,
    force: bool,
) {
    if force
        || last_emit.elapsed().as_millis() >= VOCAB_PROGRESS_INTERVAL_MS
        || files_read.is_multiple_of(VOCAB_PROGRESS_EVERY_FILES)
    {
        *last_emit = std::time::Instant::now();
        let _ = handle.emit(
            "vocab-scan-progress",
            VocabScanProgress {
                scan_id: scan_id.to_string(),
                current_path,
                files_read,
                dirs_skipped,
                terms_so_far,
                done: false,
                adopted: false,
            },
        );
    }
}

/// Number of top written forms surfaced in [`VocabScanSummary::sample_terms`].
const VOCAB_SAMPLE_TERMS: usize = 12;
/// Emit a progress event at most this often (alongside the per-N-files gate) so a
/// fast walk doesn't flood the event channel.
const VOCAB_PROGRESS_INTERVAL_MS: u128 = 50;
/// Also emit progress every this many files, so a slow-but-steady walk still
/// reports even within one throttle window.
const VOCAB_PROGRESS_EVERY_FILES: usize = 10;

/// Scan `folder` for code identifiers off the UI thread, emitting throttled
/// `vocab-scan-progress` events as it walks, and return a [`VocabScanSummary`].
///
/// On a non-empty result this also adopts the scan: it stores the folder, enables
/// code-aware vocabulary, caches the built prompt, and rebuilds the correction
/// matcher — mirroring `configure_dictation` so the scan takes effect immediately
/// without a second round-trip. The filesystem walk and prompt build run inside
/// `spawn_blocking`; no std mutex is held across an `.await`.
#[tauri::command]
pub async fn scan_code_vocab(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, State>,
    folder: String,
    scan_id: String,
) -> Result<VocabScanSummary, String> {
    let folder_trimmed = folder.trim().to_string();
    if folder_trimmed.is_empty() {
        return Err("No folder selected to scan.".to_string());
    }
    if scan_id.trim().is_empty() {
        return Err("Missing scan id.".to_string());
    }

    begin_code_vocab_scan(&state.app_state, &scan_id, &folder_trimmed);

    tracing::info!(target: "pipeline", "scan_code_vocab: start");

    // Walk + prompt build are blocking (filesystem + CPU). Run them off the async
    // runtime; the closure emits throttled progress as files are read.
    let emit_handle = app_handle.clone();
    let emit_scan_id = scan_id.clone();
    let scan_folder = folder_trimmed.clone();
    let t_start = std::time::Instant::now();
    let (prompt, summary) = tokio::task::spawn_blocking(move || {
        let path = std::path::Path::new(&scan_folder);

        // Running scan counters shared across the file and skip callbacks. Interior
        // mutability (Cell/RefCell) lets BOTH `FnMut` closures capture the same
        // state by shared reference — two closures can't each capture the same
        // locals by `&mut`, which is why this isn't plain `let mut`.
        let files_read = std::cell::Cell::new(0usize);
        let dirs_skipped = std::cell::Cell::new(0usize);
        let last_emit = std::cell::Cell::new(std::time::Instant::now());
        // Running distinct-term count (case-insensitive) surfaced by the walk's
        // accumulator, so the live `terms_so_far` count tracks the eventual prompt
        // size instead of sitting at zero until the walk finishes. The walk already
        // folds each file in, so we read this count rather than re-extracting (which
        // would double the tokenization CPU across a 1000-file/32MB scan).
        let terms_so_far = std::cell::Cell::new(0usize);
        let walk_start = std::time::Instant::now();

        // Throttled emit reading the shared counters; `force` bypasses the throttle.
        let emit = |current_path: String, force: bool| {
            let mut le = last_emit.get();
            emit_scan_progress(
                &emit_handle,
                &emit_scan_id,
                &mut le,
                current_path,
                files_read.get(),
                dirs_skipped.get(),
                terms_so_far.get(),
                force,
            );
            last_emit.set(le);
        };

        let outcome = crate::vocab::collect_source_files(
            path,
            |file_path, distinct_terms| {
                files_read.set(files_read.get() + 1);
                // Track the live distinct-term tally the walker already computed
                // (no second extraction). Clamp it to CORRECTION_TERMS — the same
                // ceiling the final summary.terms is capped at — so the live counter
                // and the settled "done" total share one ceiling and the number
                // never visibly drops from e.g. "1,832 terms" back to "500 terms"
                // at completion. This is only the live counter; the authoritative
                // ranked term count still comes from the summary.
                terms_so_far.set(distinct_terms.min(CORRECTION_TERMS));
                // Throttle: emit on a time window OR every N files, never per-file.
                emit(file_path.to_string_lossy().to_string(), false);
            },
            |skip_path| {
                dirs_skipped.set(dirs_skipped.get() + 1);
                // Force-emit skip rows so the struck-through dependency/build dirs
                // (node_modules, target, .git) always render live, even between
                // file reads — that live skip stream is the point of the feature.
                emit(skip_path.to_string_lossy().to_string(), true);
            },
        );

        // Rank the folded accumulator ONCE at the larger correction budget. The
        // walk dropped each file's contents as it read it, so this ranking is the
        // only term-level data we keep — bounded by unique-term count, not bytes.
        let ranked = outcome.vocab.ranked(CORRECTION_TERMS);
        // The cached prompt string IS this ranked list joined; Whisper later takes
        // its top-WHISPER_PROMPT_TERMS prefix, correction consumes the whole thing.
        let prompt = crate::vocab::ranked_terms_to_prompt(&ranked);
        // Preview: top written forms (ranking is descending-frequency, so the
        // first VOCAB_SAMPLE_TERMS ARE the strip's preview chips).
        let sample_terms: Vec<String> = ranked
            .iter()
            .take(VOCAB_SAMPLE_TERMS)
            .map(|r| r.term.clone())
            .collect();
        // Full ranked list for the pop-out (<= CORRECTION_TERMS), plus how many of
        // them feed Whisper (min(96, kept)).
        let ranked_terms: Vec<RankedTermJson> = ranked
            .iter()
            .map(|r| RankedTermJson {
                term: r.term.clone(),
                freq: r.freq,
            })
            .collect();
        let whisper_count = ranked_terms.len().min(WHISPER_PROMPT_TERMS);
        let terms = ranked_terms.len();

        let summary = VocabScanSummary {
            files: outcome.files_read,
            skipped: outcome.dirs_skipped,
            terms,
            bytes: outcome.total_bytes,
            capped: outcome.capped,
            ms: walk_start.elapsed().as_millis() as u64,
            sample_terms,
            ranked_terms,
            whisper_count,
            adopted: false,
        };
        (prompt, summary)
    })
    .await
    .map_err(|e| format!("Vocab scan task panicked: {}", e))?;

    let mut summary = summary;
    summary.adopted = complete_code_vocab_scan(&state.app_state, &scan_id, &folder_trimmed, prompt);

    // Final progress tick so the UI lands on the accurate adopted/superseded
    // state before the command result resolves.
    let _ = app_handle.emit(
        "vocab-scan-progress",
        VocabScanProgress {
            scan_id: scan_id.clone(),
            current_path: String::new(),
            files_read: summary.files,
            dirs_skipped: summary.skipped,
            terms_so_far: summary.terms,
            done: true,
            adopted: summary.adopted,
        },
    );

    tracing::info!(
        target: "pipeline",
        files = summary.files,
        skipped = summary.skipped,
        terms = summary.terms,
        bytes = summary.bytes,
        capped = summary.capped,
        adopted = summary.adopted,
        ms = t_start.elapsed().as_millis() as u64,
        "scan_code_vocab: complete"
    );

    Ok(summary)
}

/// Run a memory-only IDE project scan off the async runtime. Completion adopts
/// only the still-current generation; root/profile changes and Clear invalidate
/// the request before it can publish stale symbols.
fn schedule_ide_scan(app_handle: tauri::AppHandle, request: crate::ide_context::IdeScanRequest) {
    tracing::info!(
        target: "pipeline",
        generation = request.generation,
        roots = request.roots.len(),
        "ide_context_scan: start"
    );
    tauri::async_runtime::spawn(async move {
        let generation = request.generation;
        let roots = request.roots.clone();
        let cancellation = request.cancellation.clone();
        let result = tokio::task::spawn_blocking(move || {
            crate::ide_context::build_index_with_cancellation(
                generation,
                &roots,
                cancellation.as_ref(),
            )
        })
        .await
        .unwrap_or_else(|_| Err(crate::ide_context::IdeIndexBuildFailure::task_failed()));

        let (files, symbols, bytes, capped, ms, outcome_code) = match &result {
            Ok(build) => (
                build.stats.files as u64,
                build.stats.symbols as u64,
                build.stats.bytes,
                build.stats.capped,
                build.stats.ms,
                1u64,
            ),
            Err(failure) => (
                failure.stats.files as u64,
                failure.stats.symbols as u64,
                failure.stats.bytes,
                failure.stats.capped,
                failure.stats.ms,
                failure.outcome_code,
            ),
        };
        let state = app_handle.state::<State>();
        let adopted = state
            .app_state
            .ide_context
            .lock_or_recover()
            .complete(&request, result);
        if adopted {
            state.app_state.bump_settings_revision();
        }
        tracing::info!(
            target: "pipeline",
            generation,
            roots = request.roots.len(),
            files,
            symbols,
            bytes,
            capped,
            ms,
            adopted,
            outcome_code,
            "ide_context_scan: complete"
        );
    });
}

fn enabled_ide_profile_roots(app_state: &AppState, bundle_id: &str) -> Option<Vec<String>> {
    app_state
        .dictation
        .lock_or_recover()
        .app_profiles
        .iter()
        .find(|profile| profile.bundle_id == bundle_id)
        .filter(|profile| profile.ide_context_enabled)
        .map(|profile| profile.ide_project_roots.clone())
}

fn refresh_expired_ide_context(
    app_handle: &tauri::AppHandle,
    app_state: &AppState,
    bundle_id: Option<&str>,
) {
    let Some(bundle_id) = bundle_id else {
        return;
    };
    let Some(roots) = enabled_ide_profile_roots(app_state, bundle_id) else {
        return;
    };
    let request = app_state
        .ide_context
        .lock_or_recover()
        .refresh_if_expired(bundle_id, &roots);
    if let Some(request) = request {
        app_state.bump_settings_revision();
        schedule_ide_scan(app_handle.clone(), request);
    }
}

#[tauri::command]
pub fn get_ide_context_status(
    state: tauri::State<'_, State>,
    bundle_id: String,
) -> crate::ide_context::IdeContextStatus {
    state
        .app_state
        .ide_context
        .lock_or_recover()
        .status(bundle_id.trim())
}

#[tauri::command]
pub fn refresh_ide_context(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, State>,
    bundle_id: String,
) -> Result<crate::ide_context::IdeContextStatus, String> {
    let bundle_id = bundle_id.trim();
    let roots = enabled_ide_profile_roots(&state.app_state, bundle_id)
        .ok_or_else(|| "Enable local project context on this app profile first.".to_string())?;
    let request = state
        .app_state
        .ide_context
        .lock_or_recover()
        .begin_refresh(bundle_id, &roots)
        .ok_or_else(|| "Add at least one project root first.".to_string())?;
    state.app_state.bump_settings_revision();
    schedule_ide_scan(app_handle, request);
    Ok(state
        .app_state
        .ide_context
        .lock_or_recover()
        .status(bundle_id))
}

#[tauri::command]
pub fn clear_ide_context(
    state: tauri::State<'_, State>,
    bundle_id: String,
) -> Result<crate::ide_context::IdeContextStatus, String> {
    let bundle_id = bundle_id.trim();
    let roots = enabled_ide_profile_roots(&state.app_state, bundle_id)
        .ok_or_else(|| "Enable local project context on this app profile first.".to_string())?;
    state
        .app_state
        .ide_context
        .lock_or_recover()
        .clear(bundle_id, &roots);
    state.app_state.bump_settings_revision();
    Ok(state
        .app_state
        .ide_context
        .lock_or_recover()
        .status(bundle_id))
}

/// Receive generation-scoped lifecycle notifications from the app-lifetime
/// audio supervisor. The supervisor owns Core Audio and thread teardown; this
/// bridge owns the dictation state/event/performance contract.
pub(crate) fn handle_audio_lifecycle(
    app_handle: tauri::AppHandle,
    recording_id: u64,
    event: AudioLifecycleEvent,
) {
    let became_ready = event == AudioLifecycleEvent::Ready;
    let partial_app = app_handle.clone();
    handle_audio_lifecycle_with(
        app_handle,
        recording_id,
        event,
        |app_handle, recording_id| {
            tauri::async_runtime::spawn(async move {
                let state = app_handle.state::<State>();
                if stop_native_recording_for(
                    app_handle.clone(),
                    state,
                    Some(recording_id),
                    audio_lifecycle::DictationStopTrigger::Other,
                )
                .await
                .is_err()
                {
                    tracing::error!(
                        target: "pipeline",
                        recording_id,
                        "interrupted recording transcription failed"
                    );
                }
            });
        },
    );
    if became_ready {
        let register_app = partial_app.clone();
        spawn_vad_stream(partial_app.clone(), recording_id);
        spawn_dictation_partial_ticker(partial_app, recording_id, move || {
            let state = register_app.state::<State>();
            let dictation = state.app_state.dictation.lock_or_recover();
            register_partial_ticker_locked(&state.app_state, &dictation, recording_id)
        });
    }
}

fn handle_audio_lifecycle_with<R: tauri::Runtime>(
    app_handle: tauri::AppHandle<R>,
    recording_id: u64,
    event: AudioLifecycleEvent,
    transcribe_interruption: impl FnOnce(tauri::AppHandle<R>, u64),
) {
    let state = app_handle.state::<State>();
    if let AudioLifecycleEvent::InitializationFailed { kind, .. } = &event {
        state
            .performance
            .production_capture_failure(recording_id, *kind);
    }
    let is_current = || state.app_state.recording_id.load(Ordering::SeqCst) == recording_id;

    match event {
        AudioLifecycleEvent::StartupDiagnostic(diagnostic) => {
            if matches!(
                diagnostic,
                audio::AudioStartupDiagnostic::GestureAccepted { .. }
                    | audio::AudioStartupDiagnostic::GesturePcm { .. }
                    | audio::AudioStartupDiagnostic::GestureReady { .. }
            ) {
                tracing::info!(target: "audio", event_code = "audio.gesture_start_timing", recording_id,
                    timing = ?diagnostic, "content-free physical gesture startup timing");
            }
            state
                .performance
                .observe_production_capture(recording_id, diagnostic);
        }
        AudioLifecycleEvent::Accepted => {
            state.app_state.dictation_telemetry.accepted(recording_id);
            state.dictation_diagnostics.claim(recording_id);
            if !is_current() {
                emit_dictation_terminal(
                    &state,
                    recording_id,
                    DictationTerminalOutcome::Superseded,
                    DictationErrorCode::StaleOwner,
                );
            }
        }
        AudioLifecycleEvent::Ready => {
            let ready_received_at = std::time::Instant::now();
            let mut dictation = state.app_state.dictation.lock_or_recover();
            if !is_current() || dictation.status != DictationStatus::Starting {
                tracing::warn!(
                    target: "audio",
                    recording_id,
                    status = ?dictation.status,
                    "stale audio readiness ignored by dictation lifecycle"
                );
                emit_dictation_terminal(
                    &state,
                    recording_id,
                    DictationTerminalOutcome::Superseded,
                    DictationErrorCode::StaleOwner,
                );
                return;
            }
            // Core Audio is intentionally started before the immutable
            // per-recording context is resolved. Keep capturing, but do not
            // publish Recording until that snapshot is installed.
            if state.app_state.active_context(recording_id).is_none() {
                tracing::info!(
                    target: "pipeline",
                    event_code = "audio.recording_status_timing",
                    recording_id,
                    context_ready = false,
                    ready_to_observation_ms = ready_received_at.elapsed().as_millis() as u64,
                    "audio ready, awaiting context"
                );
                return;
            }
            transition_dictation_status(recording_id, &mut dictation, DictationStatus::Recording);
            *state.app_state.last_transcription_at.lock_or_recover() =
                Some(std::time::Instant::now());
            let _ = app_handle.emit("recording-status-changed", "recording");
            tracing::info!(
                target: "pipeline",
                event_code = "audio.recording_status_timing",
                recording_id,
                context_ready = true,
                ready_to_status_emit_ms = ready_received_at.elapsed().as_millis() as u64,
                "recording status emitted after audio ready"
            );
        }
        AudioLifecycleEvent::StillConnecting => {
            let dictation = state.app_state.dictation.lock_or_recover();
            if is_current() && dictation.status == DictationStatus::Starting {
                let _ = app_handle.emit(
                    "audio-initialization-stalled",
                    serde_json::json!({ "recordingId": recording_id }),
                );
            }
        }
        AudioLifecycleEvent::Recovering { reason } => {
            let previous_status;
            {
                let mut dictation = state.app_state.dictation.lock_or_recover();
                if !is_current()
                    || !matches!(
                        dictation.status,
                        DictationStatus::Starting | DictationStatus::Recording
                    )
                {
                    emit_dictation_terminal(
                        &state,
                        recording_id,
                        DictationTerminalOutcome::Superseded,
                        DictationErrorCode::StaleOwner,
                    );
                    return;
                }
                previous_status = transition_dictation_status(
                    recording_id,
                    &mut dictation,
                    DictationStatus::Recovering,
                );
            }
            if reason != AudioCancelReason::RuntimeFailure {
                state.app_state.clear_active_context(recording_id);
            }
            if app_handle
                .emit("recording-status-changed", "recovering")
                .is_ok()
            {
                emit_dictation_presentation(recording_id, DictationPresentation::CleanupInProgress);
            }
            let _ = app_handle.emit(
                "audio-recovery-started",
                serde_json::json!({
                    "recordingId": recording_id,
                    "reason": reason.as_str(),
                }),
            );
            if !matches!(
                reason,
                AudioCancelReason::HardDeadline | AudioCancelReason::RuntimeFailure
            ) {
                complete_dictation_performance(
                    &state.performance,
                    recording_id,
                    RunOutcomeV1::Cancelled {
                        stage: PerformanceStageV1::CaptureFinalization,
                    },
                    Vec::new(),
                    None,
                    None,
                );
            }
            if reason == AudioCancelReason::User {
                let (outcome, error_code) = match previous_status {
                    DictationStatus::Starting => (
                        DictationTerminalOutcome::UserCancelledStarting,
                        DictationErrorCode::CancelledStarting,
                    ),
                    DictationStatus::Recording => (
                        DictationTerminalOutcome::UserCancelledRecording,
                        DictationErrorCode::CancelledRecording,
                    ),
                    _ => unreachable!("recovery only begins from starting or recording"),
                };
                emit_dictation_terminal(&state, recording_id, outcome, error_code);
                let _ = app_handle.emit(
                    "recording-cancelled",
                    serde_json::json!({ "recordingId": recording_id }),
                );
            } else if reason != AudioCancelReason::RuntimeFailure
                && reason != AudioCancelReason::HardDeadline
            {
                let error_code = match reason {
                    AudioCancelReason::DeviceChanged => DictationErrorCode::DeviceChanged,
                    AudioCancelReason::SystemSleep => DictationErrorCode::SystemSleep,
                    AudioCancelReason::SystemWake => DictationErrorCode::SystemWake,
                    _ => DictationErrorCode::RuntimeFailure,
                };
                emit_dictation_terminal(
                    &state,
                    recording_id,
                    DictationTerminalOutcome::RuntimeInterruption,
                    error_code,
                );
            }
        }
        AudioLifecycleEvent::InitializationFailed {
            error,
            kind,
            recovery_reason,
        } => {
            if !is_current() {
                emit_dictation_terminal(
                    &state,
                    recording_id,
                    DictationTerminalOutcome::Superseded,
                    DictationErrorCode::StaleOwner,
                );
                return;
            }
            let outcome = if recovery_reason == Some(AudioCancelReason::RuntimeFailure) {
                DictationTerminalOutcome::RuntimeInterruption
            } else {
                DictationTerminalOutcome::CaptureInitFailure
            };
            emit_dictation_terminal(
                &state,
                recording_id,
                outcome,
                DictationErrorCode::from_audio_failure(kind),
            );
            state.app_state.clear_active_context(recording_id);
            complete_dictation_performance(
                &state.performance,
                recording_id,
                RunOutcomeV1::Failed {
                    stage: PerformanceStageV1::CaptureFinalization,
                    error_code: StableRunErrorV1::AudioCaptureFailed,
                },
                Vec::new(),
                None,
                None,
            );
            if app_handle
                .emit("recording-initialization-failed", {
                    let presentation = DictationPresentation::InitializationFailed(kind);
                    let (status_code, action_code) = presentation.codes();
                    serde_json::json!({
                        "recordingId": recording_id,
                        "error": error,
                        "errorKind": kind.as_str(),
                        "statusCode": status_code,
                        "actionCode": action_code,
                    })
                })
                .is_ok()
            {
                emit_dictation_presentation(
                    recording_id,
                    DictationPresentation::InitializationFailed(kind),
                );
            }
        }
        AudioLifecycleEvent::CleanupFinished => {
            complete_dictation_capture_cleanup(&state.app_state, recording_id, |restore_idle| {
                let _ = app_handle.emit(
                    "recording-cleanup-finished",
                    serde_json::json!({ "recordingId": recording_id }),
                );
                if restore_idle {
                    let _ = app_handle.emit("recording-status-changed", "idle");
                }
            });
        }
        AudioLifecycleEvent::RecoveryStalled => {
            if is_current()
                && app_handle
                    .emit("recording-recovery-stalled", {
                        let (status_code, action_code) =
                            DictationPresentation::CleanupStalled.codes();
                        serde_json::json!({
                            "recordingId": recording_id,
                            "statusCode": status_code,
                            "actionCode": action_code,
                        })
                    })
                    .is_ok()
            {
                emit_dictation_presentation(recording_id, DictationPresentation::CleanupStalled);
            }
        }
        AudioLifecycleEvent::Interrupted {
            reason,
            delivered_samples,
            duration_ms,
        } => {
            if !is_current() {
                let _ = audio_lifecycle::take_interrupted_dictation(recording_id);
                emit_dictation_terminal(
                    &state,
                    recording_id,
                    DictationTerminalOutcome::Superseded,
                    DictationErrorCode::StaleOwner,
                );
                complete_dictation_performance(
                    &state.performance,
                    recording_id,
                    RunOutcomeV1::Failed {
                        stage: PerformanceStageV1::CaptureFinalization,
                        error_code: StableRunErrorV1::AudioCaptureFailed,
                    },
                    Vec::new(),
                    Some(ContentFreeInputSummaryV1::audio(duration_ms)),
                    None,
                );
                return;
            }
            let auto_transcribe = delivered_samples >= 8_000;
            if app_handle
                .emit("recording-interrupted", {
                    let presentation = DictationPresentation::Interrupted {
                        partial_transcription: auto_transcribe,
                    };
                    let (status_code, action_code) = presentation.codes();
                    serde_json::json!({
                        "recordingId": recording_id,
                        "reason": reason.as_str(),
                        "deliveredSamples": delivered_samples,
                        "durationMs": duration_ms,
                        "autoTranscribe": auto_transcribe,
                        "statusCode": status_code,
                        "actionCode": action_code,
                    })
                })
                .is_ok()
            {
                emit_dictation_presentation(
                    recording_id,
                    DictationPresentation::Interrupted {
                        partial_transcription: auto_transcribe,
                    },
                );
            }
            if auto_transcribe {
                transcribe_interruption(app_handle.clone(), recording_id);
            } else {
                let _ = audio_lifecycle::take_interrupted_dictation(recording_id);
                emit_dictation_terminal(
                    &state,
                    recording_id,
                    DictationTerminalOutcome::RuntimeInterruption,
                    DictationErrorCode::RuntimeFailure,
                );
                state.app_state.clear_active_context(recording_id);
                complete_dictation_performance(
                    &state.performance,
                    recording_id,
                    RunOutcomeV1::Failed {
                        stage: PerformanceStageV1::CaptureFinalization,
                        error_code: StableRunErrorV1::AudioCaptureFailed,
                    },
                    Vec::new(),
                    Some(ContentFreeInputSummaryV1::audio(duration_ms)),
                    None,
                );
                let mut dictation = state.app_state.dictation.lock_or_recover();
                transition_dictation_status(recording_id, &mut dictation, DictationStatus::Idle);
                drop(dictation);
                keyboard::set_processing(false);
                let _ = app_handle.emit("recording-status-changed", "idle");
            }
        }
        AudioLifecycleEvent::Idle => {
            let mut dictation = state.app_state.dictation.lock_or_recover();
            if !is_current()
                || !matches!(
                    dictation.status,
                    DictationStatus::Starting
                        | DictationStatus::Recording
                        | DictationStatus::Processing
                        | DictationStatus::Recovering
                )
            {
                emit_dictation_terminal(
                    &state,
                    recording_id,
                    DictationTerminalOutcome::Superseded,
                    DictationErrorCode::StaleOwner,
                );
                return;
            }
            emit_dictation_terminal(
                &state,
                recording_id,
                DictationTerminalOutcome::PipelineFailure,
                DictationErrorCode::RuntimeFailure,
            );
            complete_dictation_performance(
                &state.performance,
                recording_id,
                RunOutcomeV1::Failed {
                    stage: PerformanceStageV1::CaptureFinalization,
                    error_code: StableRunErrorV1::AudioCaptureFailed,
                },
                Vec::new(),
                None,
                None,
            );
            transition_dictation_status(recording_id, &mut dictation, DictationStatus::Idle);
            state.app_state.clear_active_context(recording_id);
            keyboard::set_processing(false);
            let _ = app_handle.emit("recording-status-changed", "idle");
        }
    }
}

/// Capture completion is independent of ASR completion and stall guidance.
/// Hold the generation/status lock through publication so a newer recording
/// cannot be reset by the old owner's trailing unscoped Idle event.
pub(crate) fn complete_dictation_capture_cleanup(
    app_state: &AppState,
    recording_id: u64,
    publish: impl FnOnce(bool),
) {
    let mut dictation = app_state.dictation.lock_or_recover();
    if app_state.recording_id.load(Ordering::SeqCst) != recording_id {
        return;
    }
    let restore_idle = matches!(
        dictation.status,
        DictationStatus::Idle | DictationStatus::Recovering
    );
    if restore_idle {
        transition_dictation_status(recording_id, &mut dictation, DictationStatus::Idle);
    }
    publish(restore_idle);
}

fn publish_recording_ready_if_audio_ready(
    app_handle: &tauri::AppHandle,
    recording_id: u64,
    context_ready_at: std::time::Instant,
) -> bool {
    if !audio_lifecycle::is_dictation_recording(recording_id) {
        return false;
    }

    let state = app_handle.state::<State>();
    let mut dictation = state.app_state.dictation.lock_or_recover();
    if state.app_state.recording_id.load(Ordering::SeqCst) != recording_id
        || dictation.status != DictationStatus::Starting
        || state.app_state.active_context(recording_id).is_none()
    {
        return false;
    }

    transition_dictation_status(recording_id, &mut dictation, DictationStatus::Recording);
    *state.app_state.last_transcription_at.lock_or_recover() = Some(std::time::Instant::now());
    let _ = app_handle.emit("recording-status-changed", "recording");
    tracing::info!(
        target: "pipeline",
        event_code = "audio.recording_status_timing",
        recording_id,
        context_to_status_emit_ms = context_ready_at.elapsed().as_millis() as u64,
        "recording status emitted after context ready"
    );
    spawn_vad_stream(app_handle.clone(), recording_id);
    spawn_dictation_partial_ticker(app_handle.clone(), recording_id, || {
        register_partial_ticker_locked(&state.app_state, &dictation, recording_id)
    });
    true
}

/// A reason the shared dictation start transition refuses a new recording.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DictationStartBlocker {
    pub kind: &'static str,
    pub state: &'static str,
}

/// The admission guards shared by a confirmed dictation start and silent
/// first-tap helper preparation. Callers hold the dictation lock, so
/// `transcribe_file` cannot slip between the
/// check and the transition.
pub(crate) fn dictation_start_blocker(
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
    dictation_activity_blocker(state, status)
}

/// What currently occupies dictation, ignoring a committed relaunch. The
/// relaunch itself asks this, so its own commitment cannot read as activity.
fn dictation_activity_blocker(
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

/// Whether silent first-tap preparation may spawn an idle capture helper.
/// It shares the confirmed start's admission guards and additionally refuses
/// while a previous dictation's capture cleanup is still pending.
pub(crate) fn dictation_prepare_admitted(state: &State) -> bool {
    dictation_prepare_refusal(state).is_none()
}

/// The first content-free reason a prepared or key-down capture may not open
/// now, or `None` when it may.
pub(crate) fn dictation_prepare_refusal(state: &State) -> Option<&'static str> {
    dictation_refusal(state, true)
}

/// Whether anything dictation-related is in progress, for a permission
/// relaunch. A relaunch that has itself committed does not count.
pub(crate) fn dictation_relaunch_busy(state: &State) -> Option<&'static str> {
    dictation_refusal(state, false)
}

fn dictation_refusal(state: &State, honor_committed_relaunch: bool) -> Option<&'static str> {
    let dictation = state.app_state.dictation.lock_or_recover();
    let blocker = if honor_committed_relaunch {
        dictation_start_blocker(state, dictation.status)
    } else {
        dictation_activity_blocker(state, dictation.status)
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

#[tauri::command]
pub async fn start_native_recording(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, State>,
    device_name: Option<String>,
    smart_auto: Option<SmartAutoRequest>,
    origin: Option<String>,
    gesture_token: Option<String>,
) -> Result<serde_json::Value, String> {
    // Consume only this request's stamp before every await or rejected start.
    let gesture_claim = audio_lifecycle::gesture::claim(gesture_token.as_deref());
    // A Fn/Globe gesture may own a key-down capture. Dropping this claim on any
    // early return discards that audio immediately.
    let speculative = audio_lifecycle::speculative::claim(gesture_token.as_deref());
    // This lock covers only the synchronous ownership transition. Core Audio
    // initialization happens on the supervisor and never waits under it.
    let _transition = crate::commands::microphone_preview::transition_after_stopping_preview(
        &app_handle,
        state.inner(),
    )
    .await?;
    if keyboard::is_app_disabled() {
        tracing::info!(target: "pipeline", "start_native_recording: app disabled — ignoring");
        return Ok(serde_json::json!({ "type": "app_disabled", "state": "idle" }));
    }
    let origin = match origin.as_deref() {
        Some("hold") => "hold",
        _ => "toggle",
    };
    if smart_auto.is_some() && device_name.is_some() {
        return Err("Smart Auto cannot be combined with a fixed microphone.".to_string());
    }
    // Check and update status in one lock; assign recording ID in the same
    // critical section so no concurrent cancel/start can slip between them.
    let (
        rid,
        gesture,
        performance_started_at_ms,
        smart_enabled,
        delivery_target,
        selected_device_name,
        smart_auto_reason,
    ) = {
        let mut dictation = state.app_state.dictation.lock_or_recover();
        if let Some(blocker) = dictation_start_blocker(state.inner(), dictation.status) {
            tracing::warn!(target: "pipeline", reason = blocker.kind, "start_native_recording: blocked");
            return Ok(serde_json::json!({
                "type": blocker.kind,
                "state": blocker.state
            }));
        }
        match dictation.status {
            DictationStatus::Starting
            | DictationStatus::Recording
            | DictationStatus::Recovering
            | DictationStatus::Processing => {
                // dictation_start_blocker already refused every non-idle
                // status under this lock.
                return Err("Dictation is not idle.".to_string());
            }
            DictationStatus::Idle => {
                if audio_lifecycle::is_dictation_cleanup_pending(
                    state.app_state.recording_id.load(Ordering::SeqCst),
                ) {
                    // The result has finished, but its capture owner still
                    // needs cleanup. Keep that generation for failure guidance
                    // and let confirmed exit restore Idle independently.
                    let recording_id = state.app_state.recording_id.load(Ordering::SeqCst);
                    transition_dictation_status(
                        recording_id,
                        &mut dictation,
                        DictationStatus::Recovering,
                    );
                    let _ = app_handle.emit("recording-status-changed", "recovering");
                    return Ok(
                        serde_json::json!({ "type": "audio_recovering", "state": "recovering" }),
                    );
                }
                // This is a cache-only policy read while the capture lifecycle
                // is Idle. Its stable ID is passed to the supervisor below,
                // which freezes it for the whole generation.
                let (selected_device_name, device_selection, smart_auto_reason) =
                    match smart_auto.as_ref() {
                        Some(request) => {
                            let selection = crate::audio_inventory::resolve_smart_auto(request)?;
                            (
                                Some(selection.device_id),
                                "smart_auto",
                                Some(selection.reason.as_str()),
                            )
                        }
                        None => (
                            device_name.clone(),
                            if device_name.is_some() {
                                "explicit"
                            } else {
                                "system_default"
                            },
                            None,
                        ),
                    };
                let rid = state.app_state.next_recording_id();
                let gesture = gesture_claim.accepted(origin == "hold");
                crate::audio::prepare::capture_claimed(
                    crate::audio::prepare::CaptureClaim::Dictation(gesture.timing.map(|g| {
                        crate::audio::prepare::PrepareIdentity {
                            token: g.candidate.token,
                            listener_generation: keyboard::listener_generation(),
                            topology_generation: audio_lifecycle::input_topology_generation(),
                            configuration_generation:
                                crate::audio::prepare::configuration_generation(),
                        }
                    })),
                );
                let performance_started_at_ms =
                    accept_live_dictation_request(rid, &mut dictation, |status_at_acceptance| {
                        debug_assert_eq!(status_at_acceptance, DictationStatus::Idle);
                        tracing::info!(
                            target: "pipeline",
                            event_code = "pipeline.dictation_requested",
                            slo_contract = DICTATION_SLO_CONTRACT,
                            device_selection,
                            recording_id = rid,
                            origin,
                            "start_native_recording"
                        );
                    });
                // Freeze the native delivery target immediately after the
                // accepted Idle -> Starting transition, under the same
                // ownership lock. This startup work is therefore included in
                // request-to-first-PCM SLO timing.
                let delivery_target = crate::frontmost::capture_delivery_target_snapshot();
                (
                    rid,
                    gesture,
                    performance_started_at_ms,
                    smart_destination_eligible(&dictation),
                    delivery_target,
                    selected_device_name,
                    smart_auto_reason,
                )
            }
        }
    };
    // A Fn key-down capture that already retains PCM is adopted with no
    // connecting phase: Recording follows as soon as the context is installed.
    let speculative_flowing = speculative.is_flowing();
    let mut gesture = gesture;
    gesture.speculative_capture = speculative.consume();
    // Publish Starting before the worker can report Ready. Otherwise a very
    // fast device open could emit Recording first and this command would then
    // overwrite the frontend with a stale Starting event.
    let _ = app_handle.emit(
        "dictation-generation-started",
        serde_json::json!({ "recordingId": rid }),
    );
    if !speculative_flowing {
        let _ = app_handle.emit("recording-status-changed", "starting");
    }
    // Reserve the generation before the worker is allowed to emit lifecycle
    // events. Terminal persistence can now wait for the asynchronous begin
    // without holding capture ownership or the recording transition.
    state.performance.register_production(
        rid,
        if smart_auto.is_some() {
            crate::performance_metrics::production::MicrophoneSelectionV1::SmartAuto
        } else if device_name.is_some() {
            crate::performance_metrics::production::MicrophoneSelectionV1::Explicit
        } else {
            crate::performance_metrics::production::MicrophoneSelectionV1::SystemDefault
        },
        crate::audio_inventory::production_device_kind(selected_device_name.as_deref()),
    );
    dictation_performance_begins().register(rid);
    if let Err(error) = audio_lifecycle::start_dictation_recording(
        app_handle.clone(),
        selected_device_name,
        rid,
        origin,
        gesture,
    ) {
        dictation_performance_begins().abort(rid, &state.performance);
        tracing::error!(target: "audio", "start_native_recording: audio failed: {}", error);
        state.app_state.clear_active_context(rid);
        let mut dictation = state.app_state.dictation.lock_or_recover();
        if state.app_state.recording_id.load(Ordering::SeqCst) == rid {
            transition_dictation_status(rid, &mut dictation, DictationStatus::Idle);
        }
        let _ = app_handle.emit("recording-status-changed", "idle");
        emit_dictation_terminal(
            &state,
            rid,
            DictationTerminalOutcome::CaptureInitFailure,
            DictationErrorCode::RuntimeFailure,
        );
        return match error {
            AudioStartError::AlreadyStarting => Ok(serde_json::json!({
                "type": "already_starting",
                "state": "idle"
            })),
            AudioStartError::AudioRecovering => Ok(serde_json::json!({
                "type": "audio_recovering",
                "state": "idle"
            })),
            other => Err(other.to_string()),
        };
    }
    tracing::info!(
        target: "pipeline",
        smart_auto_reason = smart_auto_reason.unwrap_or("not_used"),
        "start_native_recording: starting"
    );

    // Resolve profile and IDE context from the same native sample frozen at
    // acceptance. A later focus change cannot splice another application's
    // context into the recording while retaining the original paste target.
    let context_resolution_started = std::time::Instant::now();
    let captured_smart_destination =
        crate::smart_destination::capture(&app_handle, &delivery_target, smart_enabled).await;
    let capture_is_current = {
        let dictation = state.app_state.dictation.lock_or_recover();
        recording_capture_is_current(
            rid,
            state.app_state.recording_id.load(Ordering::SeqCst),
            state.app_state.is_cancelled(rid),
            dictation.status,
        )
    };
    let smart_destination = if capture_is_current {
        captured_smart_destination
    } else if smart_enabled {
        // A late result from a failed/cancelled start must not select a profile
        // or retain a native field capability for that inactive generation.
        crate::smart_destination::Snapshot::Unavailable
    } else {
        crate::smart_destination::Snapshot::Disabled
    };
    let destination = select_recording_destination(&delivery_target, smart_destination);
    let app_identity = crate::frontmost::profile_identity_from_delivery_target(&destination.target);
    refresh_expired_ide_context(
        &app_handle,
        &state.app_state,
        app_identity.bundle_id.as_deref(),
    );
    let mut context = resolve_recording_context(&state.app_state, destination);
    // Install the resource in the immutable recording snapshot before publishing
    // it. The live-dictation engine currently refuses generation and preparation
    // (#1081); if re-enabled, preparation waits for Activate below.
    if context.transformations.correction_enabled
        && context.transformations.homophone_context_enabled
    {
        Arc::make_mut(&mut context).homophone_engine =
            Some(crate::homophone_context::system_engine());
    }
    if smart_enabled && capture_is_current {
        emit_smart_destination(
            &app_handle,
            &state.app_state,
            rid,
            &context,
            &delivery_target,
        );
    }
    state.performance.production_context(rid, &context);
    let context_action = {
        let dictation = state.app_state.dictation.lock_or_recover();
        let action = native_start_context_action(
            state.app_state.recording_id.load(Ordering::SeqCst) == rid,
            dictation.status,
        );
        if matches!(
            action,
            NativeStartContextAction::Activate
                | NativeStartContextAction::FlushLifecycleTerminal {
                    install_context: true
                }
        ) {
            state
                .app_state
                .set_active_context(rid, Arc::clone(&context));
        }
        (action, dictation.status)
    };
    match context_action {
        (NativeStartContextAction::Activate, _) => {}
        (NativeStartContextAction::FlushLifecycleTerminal { .. }, status) => {
            let runtimes =
                runtime_identity(&context.transcription.model_name, ModelWarmStateV1::Unknown);
            tracing::info!(
                target: "pipeline",
                recording_id = rid,
                ?status,
                "flushing reserved diagnostics after audio lifecycle terminal"
            );
            drop(_transition);
            spawn_native_dictation_performance_begin(
                state.performance.clone(),
                rid,
                runtimes,
                performance_started_at_ms,
            );
            return Ok(serde_json::json!({
                "type": "audio_recovering",
                "state": match status {
                    DictationStatus::Idle => "idle",
                    DictationStatus::Recovering => "recovering",
                    _ => unreachable!("lifecycle flush only follows idle or recovering"),
                }
            }));
        }
        (NativeStartContextAction::Superseded, status) => {
            dictation_performance_begins().abort(rid, &state.performance);
            emit_dictation_terminal(
                &state,
                rid,
                DictationTerminalOutcome::Superseded,
                DictationErrorCode::StaleOwner,
            );
            tracing::warn!(
                target: "pipeline",
                recording_id = rid,
                ?status,
                "dictation context discarded after audio initialization ended"
            );
            return Ok(serde_json::json!({
                "type": "audio_recovering",
                "state": match status {
                    DictationStatus::Idle => "idle",
                    DictationStatus::Starting => "starting",
                    DictationStatus::Recording => "recording",
                    DictationStatus::Recovering => "recovering",
                    DictationStatus::Processing => "processing",
                }
            }));
        }
    }
    let context_ready_at = std::time::Instant::now();
    tracing::info!(
        target: "pipeline",
        event_code = "audio.recording_context_ready",
        recording_id = rid,
        context_resolution_ms = context_ready_at.duration_since(context_resolution_started).as_millis() as u64,
        frontmost_app_detected = context.app.bundle_id.is_some(),
        matched_profile = context.matched_profile.is_some(),
        writing_style = context.writing_style.as_str(),
        writing_style_code = context.writing_style.code(),
        vocabulary_version = context.vocabulary.version,
        context_reads_enabled = context.context_capture.selected_text
            || context.context_capture.surrounding_screen_text
            || context.context_capture.clipboard
            || context.context_capture.local_project_index,
        "dictation context resolved"
    );
    if let Some(engine) = &context.homophone_engine {
        // Still under recording_transition and after the exact-owner Activate
        // check. The #1081 live-dictation gate makes this a no-op while the
        // engine is unavailable. If re-enabled, async publication is confined to
        // this context's disposable owner, never app-wide recording state.
        engine.prewarm();
    }
    publish_recording_ready_if_audio_ready(&app_handle, rid, context_ready_at);
    spawn_model_preparation(
        app_handle.clone(),
        context.transcription.model_name.clone(),
        rid,
    );
    let runtimes = runtime_identity(&context.transcription.model_name, ModelWarmStateV1::Unknown);
    // Ownership is fully installed before diagnostics begin. Release the
    // transition first so Recording, stop, and cancel never wait for SQLite.
    drop(_transition);
    spawn_native_dictation_performance_begin(
        state.performance.clone(),
        rid,
        runtimes,
        performance_started_at_ms,
    );

    Ok(serde_json::json!({
        "type": "recording_starting",
        "state": "starting",
        "recordingId": rid
    }))
}

#[tauri::command]
pub async fn stop_native_recording(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, State>,
    stop_trigger: Option<audio_lifecycle::DictationStopTrigger>,
) -> Result<serde_json::Value, String> {
    stop_native_recording_for(app_handle, state, None, stop_trigger.unwrap_or_default()).await
}

fn stop_owner_matches(current_recording_id: u64, expected_recording_id: Option<u64>) -> bool {
    expected_recording_id.is_none_or(|expected| expected == current_recording_id)
}

/// Only the resolved double-tap toggle owns its stop-key diagnostics. A
/// pending stamp is consumed but discarded when an overlay, auto-stop, or
/// matching hold release wins the accepted stop.
fn attributable_stop_stamp(
    trigger: audio_lifecycle::DictationStopTrigger,
    stamp: Option<crate::shadow_measurement::StopStamp>,
) -> Option<crate::shadow_measurement::StopStamp> {
    stamp.filter(|_| trigger == audio_lifecycle::DictationStopTrigger::Toggle)
}

/// Stop only the recording generation captured by the caller. User commands
/// pass no expectation; asynchronous lifecycle continuations must pass the
/// generation that scheduled them so they cannot stop a newer recording.
async fn stop_native_recording_for(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, State>,
    expected_recording_id: Option<u64>,
    stop_trigger: audio_lifecycle::DictationStopTrigger,
) -> Result<serde_json::Value, String> {
    // The `releaseToStop` endpoint: entry, before any lock wait (#826).
    let stop_entered_at = std::time::Instant::now();
    let transition = state.app_state.recording_transition.lock().await;
    let current_recording_id = state.app_state.recording_id.load(Ordering::SeqCst);
    if !stop_owner_matches(current_recording_id, expected_recording_id) {
        let stale_recording_id = expected_recording_id.expect("mismatch requires an expectation");
        let _ = audio_lifecycle::take_interrupted_dictation(stale_recording_id);
        emit_dictation_terminal(
            &state,
            stale_recording_id,
            DictationTerminalOutcome::Superseded,
            DictationErrorCode::StaleOwner,
        );
        return Ok(serde_json::json!({
            "type": "recording_superseded",
            "state": "idle"
        }));
    }
    // Owns the live preview's hide from the accepted stop: queued behind the
    // paste by delivery, or hidden at once when this function or the pipeline
    // returns without one.
    let mut preview_hide = None;
    // Atomic check-and-set + generation capture in a single lock.
    let (status, rid, interrupted_capture, stop_delivery_target) = {
        let mut dictation = state.app_state.dictation.lock_or_recover();
        let rid = current_recording_id;
        match dictation.status {
            DictationStatus::Processing => {
                return Ok(serde_json::json!({
                    "type": "already_processing",
                    "state": "processing"
                }))
            }
            DictationStatus::Idle => {
                tracing::warn!(target: "pipeline", "stop_native_recording: not recording");
                return Ok(serde_json::json!({
                    "type": "not_recording",
                    "state": "idle"
                }));
            }
            // A cancelled start never delivers, so it needs no stop anchor.
            DictationStatus::Starting => (DictationStatus::Starting, rid, None, None),
            DictationStatus::Recovering => {
                if let Some(capture) = audio_lifecycle::take_interrupted_dictation(rid) {
                    preview_hide = crate::commands::dictation_preview::PreviewHideOwner::claim(
                        &app_handle,
                        rid,
                    );
                    transition_dictation_status(rid, &mut dictation, DictationStatus::Processing);
                    // Freeze the native delivery target at the accepted stop
                    // transition, under the same ownership lock that commits
                    // Processing.
                    (
                        DictationStatus::Processing,
                        rid,
                        Some(capture),
                        Some(crate::frontmost::capture_delivery_target_snapshot()),
                    )
                } else {
                    return Ok(serde_json::json!({
                        "type": "audio_recovering",
                        "state": "recovering"
                    }));
                }
            }
            DictationStatus::Recording => {
                // Before status leaves Recording: the ticker then sees this
                // claim whenever it sees the stop, and leaves the hide to it.
                preview_hide =
                    crate::commands::dictation_preview::PreviewHideOwner::claim(&app_handle, rid);
                transition_dictation_status(rid, &mut dictation, DictationStatus::Processing);
                // Freeze the native delivery target at the accepted stop
                // transition, under the same ownership lock that commits
                // Processing.
                (
                    DictationStatus::Recording,
                    rid,
                    None,
                    Some(crate::frontmost::capture_delivery_target_snapshot()),
                )
            }
        }
    };
    // Claim the release at the accepted transition, before teardown, so a
    // later release queued behind this stop cannot replace it.
    let hotkey_release = crate::hotkey_release::take_for(rid, stop_entered_at);
    let stop_stamp = crate::shadow_measurement::take_for(rid, stop_entered_at);
    // A matching rdev release takes precedence even if another stop arrived
    // through the webview at the same time. Unknown sources retain speech.
    // Taken from the trigger as requested: `resolve` turns any stamped stop into
    // a hold release, so a stale stamp must be ruled out before it.
    let tail_anchor = stop_trigger.tail_anchor(hotkey_release);
    let stop_trigger = stop_trigger.resolve(hotkey_release);
    let stop_stamp = attributable_stop_stamp(stop_trigger, stop_stamp);
    if status == DictationStatus::Starting {
        drop(transition);
        // Cancel the exact owner even if readiness races with this command
        // after the status snapshot. A starting-only cancellation could
        // otherwise no-op while the command falsely reports cancellation and
        // leaves a newly live stream running.
        if let Err(error) = audio_lifecycle::cancel_dictation_capture(rid, AudioCancelReason::User)
        {
            emit_dictation_terminal(
                &state,
                rid,
                DictationTerminalOutcome::PipelineFailure,
                DictationErrorCode::RuntimeFailure,
            );
            return Err(error.to_string());
        }
        return Ok(serde_json::json!({
            "type": "recording_cancelled",
            "state": "recovering"
        }));
    }
    let context = match state.app_state.active_context(rid) {
        Some(context) => context,
        None => {
            emit_dictation_terminal(
                &state,
                rid,
                DictationTerminalOutcome::PipelineFailure,
                DictationErrorCode::MissingContext,
            );
            let mut dictation = state.app_state.dictation.lock_or_recover();
            if state.app_state.recording_id.load(Ordering::SeqCst) == rid {
                transition_dictation_status(rid, &mut dictation, DictationStatus::Idle);
            }
            keyboard::set_processing(false);
            let _ = app_handle.emit("recording-status-changed", "idle");
            drop(transition);
            complete_dictation_performance(
                &state.performance,
                rid,
                RunOutcomeV1::Failed {
                    stage: PerformanceStageV1::CaptureFinalization,
                    error_code: StableRunErrorV1::InternalEarlyExit,
                },
                Vec::new(),
                None,
                None,
            );
            return Err(format!("Missing dictation context for recording {rid}"));
        }
    };
    keyboard::set_processing(true);
    tracing::info!(target: "pipeline", "stop_native_recording: stopping");
    let _ = app_handle.emit("recording-status-changed", "processing");

    // Guard resets status to Idle if stop_recording fails or samples are empty;
    // disarmed before handing off to run_transcription_pipeline (which has its own guard)
    let mut guard = IdleGuard::new(&state.app_state, rid);

    // Phase: Audio teardown + 16kHz resample
    let t_total = std::time::Instant::now();
    // Processing is already committed, so concurrent calls cannot claim the
    // microphone. Release the transition guard before the supervisor wait.
    drop(transition);
    let interruption_error = interrupted_capture
        .as_ref()
        .map(|capture| DictationErrorCode::from_interruption_reason(&capture.reason));
    let interruption = interrupted_capture.as_ref().map(|capture| {
        serde_json::json!({
            "reason": capture.reason.clone(),
            "deliveredSamples": capture.samples.len(),
            "durationMs": capture.duration_ms,
        })
    });
    let samples = if let Some(capture) = interrupted_capture {
        capture.samples
    } else {
        audio_lifecycle::stop_dictation_recording(rid, stop_trigger, tail_anchor).map_err(|e| {
            emit_dictation_terminal(
                &state,
                rid,
                DictationTerminalOutcome::StopFailure,
                DictationErrorCode::StopFinalizationFailed,
            );
            tracing::error!(target: "audio", "stop_native_recording: stop_recording failed: {}", e);
            guard.finish(|_| {
                let _ = app_handle.emit("recording-status-changed", "idle");
            });
            complete_dictation_performance(
                &state.performance,
                rid,
                RunOutcomeV1::Failed {
                    stage: PerformanceStageV1::CaptureFinalization,
                    error_code: StableRunErrorV1::AudioCaptureFailed,
                },
                Vec::new(),
                None,
                None,
            );
            e
        })?
    };
    state
        .performance
        .production_sample_count(rid, samples.len());
    let mut performance_guard = NativeDictationPerformanceGuard::new(
        state.performance.clone(),
        rid,
        PerformanceStageV1::CaptureFinalization,
    );
    if let Some(release) = hotkey_release {
        performance_guard.record(release.release_to_stop());
    }
    if let Some(stamp) = stop_stamp {
        for stage in stamp.stages(stop_entered_at) {
            performance_guard.record(stage);
        }
    }
    tracing::info!(
        target: "pipeline",
        event_code = "pipeline.dictation_stop_handoff",
        recording_id = rid,
        audio_ms = samples.len() as u64 / 16,
        interrupted = interruption_error.is_some(),
        "dictation capture handed off for processing"
    );
    tracing::info!(target: "pipeline", "audio teardown + resample: {:?}", t_total.elapsed());
    let capture_finalization_us = micros(t_total);
    performance_guard.record(StageTimingV1::measured(
        PerformanceStageV1::CaptureFinalization,
        t_total.elapsed().as_millis() as u64,
    ));
    let handoff_started_at = std::time::Instant::now();

    if samples.is_empty() {
        emit_dictation_terminal(
            &state,
            rid,
            interruption_error
                .map(|_| DictationTerminalOutcome::RuntimeInterruption)
                .unwrap_or(DictationTerminalOutcome::NoSpeech),
            interruption_error.unwrap_or(DictationErrorCode::EmptyAudio),
        );
        tracing::info!(target: "pipeline", "stop_native_recording: no audio captured");
        guard.finish(|_| {
            let _ = app_handle.emit("recording-status-changed", "idle");
        });
        performance_guard.finish(
            RunOutcomeV1::NoSpeech,
            vec![StageTimingV1::measured(
                PerformanceStageV1::TotalProcessing,
                t_total.elapsed().as_millis() as u64,
            )],
            Some(ContentFreeInputSummaryV1::audio(samples.len() as u64 / 16)),
            None,
        );
        return Ok(serde_json::json!({
            "type": "transcription",
            "text": "",
            "state": "idle"
        }));
    }

    /// Minimum recording duration to process. Recordings shorter than this
    /// are discarded as phantom triggers (e.g. from residual key presses).
    const MIN_RECORDING_SAMPLES: usize = 4_800; // 0.3s at 16kHz

    if samples.len() < MIN_RECORDING_SAMPLES {
        emit_dictation_terminal(
            &state,
            rid,
            interruption_error
                .map(|_| DictationTerminalOutcome::RuntimeInterruption)
                .unwrap_or(DictationTerminalOutcome::TooShort),
            interruption_error.unwrap_or(DictationErrorCode::BelowMinimumDuration),
        );
        tracing::info!(target: "pipeline", "stop_native_recording: recording too short ({}ms), discarding",
            samples.len() / 16); // samples / 16_000 * 1000
        guard.finish(|_| {
            let _ = app_handle.emit("recording-status-changed", "idle");
        });
        performance_guard.finish(
            RunOutcomeV1::NoSpeech,
            vec![StageTimingV1::measured(
                PerformanceStageV1::TotalProcessing,
                t_total.elapsed().as_millis() as u64,
            )],
            Some(ContentFreeInputSummaryV1::audio(samples.len() as u64 / 16)),
            None,
        );
        return Ok(serde_json::json!({
            "type": "transcription",
            "text": "",
            "state": "idle"
        }));
    }

    // Hand off status management to the pipeline's own guard
    guard.disarm();
    performance_guard.enter(PerformanceStageV1::Vad);

    let pipeline_result = run_transcription_pipeline(
        &samples,
        &app_handle,
        &state.app_state,
        rid,
        &mut performance_guard,
        Arc::clone(&context),
        PipelineStopContext {
            delivery_target: stop_delivery_target,
            preview_hide: preview_hide.take(),
        },
    )
    .await;
    let pipeline_returned_at = std::time::Instant::now();
    // Only emit idle if this recording wasn't cancelled/superseded by a new one.
    // Hold the dictation lock across the check+emit to prevent a concurrent
    // start from interleaving a "recording" status between our check and emit.
    {
        let _dictation = state.app_state.dictation.lock_or_recover();
        if state.app_state.recording_id.load(Ordering::SeqCst) == rid {
            keyboard::set_processing(false);
            let _ = app_handle.emit("recording-status-changed", "idle");
        }
    }
    let idle_block_ended_at = std::time::Instant::now();
    let idle_status_emit_us = micros_between(pipeline_returned_at, idle_block_ended_at);
    let pipeline = match pipeline_result {
        Ok(result) => result,
        Err(error) => {
            emit_dictation_terminal(
                &state,
                rid,
                interruption_error
                    .map(|_| DictationTerminalOutcome::RuntimeInterruption)
                    .unwrap_or(DictationTerminalOutcome::PipelineFailure),
                interruption_error.unwrap_or(DictationErrorCode::TranscriptionFailed),
            );
            tracing::error!(
                target: "pipeline",
                recording_id = rid,
                "dictation pipeline failed"
            );
            return Err(error);
        }
    };
    let idle_observed_at = std::time::Instant::now();
    let history = pipeline.history;
    let text = pipeline.text;
    let timings = pipeline.timings;

    let delivered_at = std::time::Instant::now();
    let total_ms = delivered_at.duration_since(t_total).as_millis() as u64;
    let audio_secs = samples.len() as f64 / 16_000.0;
    let word_count = if text.trim().is_empty() {
        0
    } else {
        text.split_whitespace().count()
    };
    let char_count = text.len();
    let model_name = context.transcription.model_name.clone();
    let backend_name = model_runtime::model_definition(&model_name)
        .map(|model| model.backend.as_str())
        .unwrap_or("unknown")
        .to_string();

    let (terminal_outcome, terminal_error) = if let Some(error_code) = interruption_error {
        (DictationTerminalOutcome::RuntimeInterruption, error_code)
    } else {
        match pipeline.terminal {
            PipelineTerminal::Success => {
                (DictationTerminalOutcome::Success, DictationErrorCode::None)
            }
            PipelineTerminal::NoSpeech(error_code) => {
                (DictationTerminalOutcome::NoSpeech, error_code)
            }
            PipelineTerminal::Cancelled(_) => (
                DictationTerminalOutcome::UserCancelledProcessing,
                DictationErrorCode::CancelledProcessing,
            ),
        }
    };
    let successful_capture = history.as_ref().map(|history| SuccessfulPrivateCapture {
        raw_text: &history.raw_text,
        final_text: &text,
        model_id: &model_name,
        total_ms,
    });
    emit_dictation_terminal_with_capture(
        &state,
        rid,
        terminal_outcome,
        terminal_error,
        char_count as u64,
        successful_capture,
    );

    emit_native_dictation_metrics(NativeDictationMetrics {
        recording_id: rid,
        outcome: terminal_outcome,
        timings: &timings,
        total_ms,
        audio_secs,
        word_count,
        char_count,
        model_name: &model_name,
        backend_name: &backend_name,
    });
    emit_dictation_stage_gaps(
        rid,
        &timings.gaps,
        &StopPathMarks {
            t_total,
            capture_finalization_us,
            handoff_started_at,
            pipeline_returned_at,
            idle_status_emit_us,
            idle_block_ended_at,
            delivered_at,
        },
    );

    let outcome = match pipeline.terminal {
        PipelineTerminal::Success => RunOutcomeV1::Success,
        PipelineTerminal::NoSpeech(_) => RunOutcomeV1::NoSpeech,
        PipelineTerminal::Cancelled(stage) => RunOutcomeV1::Cancelled { stage },
    };
    let warm_state = timings.warm_state.unwrap_or(ModelWarmStateV1::Unknown);
    let mut stages = pipeline_stages(&timings, total_ms);
    stages.extend(release_to_paste_stage(
        &outcome,
        timings.delivery_confirmed,
        hotkey_release,
        delivered_at,
    ));
    let recording_secs = samples.len() / 16_000;
    if let Some(filtered) = pipeline
        .final_input
        .filter(|_| timings.delivery_confirmed && matches!(outcome, RunOutcomeV1::Success))
    {
        register_shadow_measurement(
            &app_handle,
            rid,
            context.clone(),
            crate::shadow_measurement::ShadowAudio {
                original: samples,
                filtered,
                stamp: stop_stamp,
                boundary: crate::shadow_measurement::PostDeliveryBoundary {
                    idle_observed_at,
                    delivered_at,
                },
            },
        );
    }
    performance_guard.finish(
        outcome,
        stages,
        Some(ContentFreeInputSummaryV1::audio_with_output(
            (audio_secs * 1_000.0).round() as u64,
            text.len(),
        )),
        Some(runtime_identity(&model_name, warm_state)),
    );

    // Broadcast transcription result to all windows (so the main window can update
    // its history even when recording was initiated from the overlay).
    if !text.is_empty() {
        let history = history.expect("non-empty completed transcription has history metadata");
        let _ = app_handle.emit(
            "transcription-complete",
            serde_json::json!({
                "recordingId": rid,
                "text": text,
                "rawText": history.raw_text,
                "duration": recording_secs,
                "interrupted": interruption,
                "recording": {
                    "recordingId": rid,
                    "modelId": model_name,
                    "profileId": context.matched_profile.as_ref().map(|profile| profile.label.as_str()),
                    "stages": history.stages,
                }
            }),
        );
    }

    Ok(serde_json::json!({
        "type": "transcription",
        "text": text,
        "state": "idle"
    }))
}

/// Cancel an in-progress recording or transcription.
///
/// - **Starting**: requests cancellation and reports Recovering while retaining
///   exclusive ownership until the audio thread exits, then returns to Idle.
/// - **Recording**: follows the same strict Recovering-to-Idle ownership
///   transition; captured samples are discarded.
/// - **Processing**: marks the current recording_id as cancelled so the
///   pipeline discards its result at the next checkpoint; immediately
///   emits idle status so the UI resets without waiting for whisper.
/// - **Idle**: no-op.
///
/// With `expected_recording_id`, cancels only that recording; a newer or
/// older generation is a no-op.
#[tauri::command]
pub async fn cancel_native_recording(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, State>,
    expected_recording_id: Option<u64>,
) -> Result<(), String> {
    let transition = state.app_state.recording_transition.lock().await;
    let (prev_status, rid) = {
        let mut dictation = state.app_state.dictation.lock_or_recover();
        let prev = dictation.status;
        let rid = state.app_state.recording_id.load(Ordering::SeqCst);
        // A deferred hold release cancels only the recording its own start
        // created, never a newer one.
        if !stop_owner_matches(rid, expected_recording_id) {
            return Ok(());
        }
        match prev {
            DictationStatus::Idle => return Ok(()),
            DictationStatus::Starting | DictationStatus::Recording => {}
            DictationStatus::Recovering => return Ok(()),
            DictationStatus::Processing => {
                transition_dictation_status(rid, &mut dictation, DictationStatus::Idle);
            }
        }
        (prev, rid)
    };
    if matches!(
        prev_status,
        DictationStatus::Starting | DictationStatus::Recording
    ) {
        drop(transition);
        if let Err(error) = audio_lifecycle::cancel_dictation_capture(rid, AudioCancelReason::User)
        {
            emit_dictation_terminal(
                &state,
                rid,
                DictationTerminalOutcome::PipelineFailure,
                DictationErrorCode::RuntimeFailure,
            );
            return Err(error.to_string());
        }
        return Ok(());
    }
    state.app_state.clear_active_context(rid);

    match prev_status {
        DictationStatus::Processing => {
            // Mark current recording as cancelled — pipeline will check at next checkpoint.
            state.app_state.cancel_recording(rid);
            emit_dictation_terminal(
                &state,
                rid,
                DictationTerminalOutcome::UserCancelledProcessing,
                DictationErrorCode::CancelledProcessing,
            );
            tracing::info!(target: "pipeline", "cancel_native_recording: processing cancelled (recording_id={})", rid);
        }
        DictationStatus::Idle
        | DictationStatus::Starting
        | DictationStatus::Recording
        | DictationStatus::Recovering => unreachable!(),
    }

    // Always emit feedback so the UI resets, even if stop_recording failed
    keyboard::set_processing(false);
    let _ = app_handle.emit("recording-status-changed", "idle");
    let _ = app_handle.emit(
        "recording-cancelled",
        serde_json::json!({ "recordingId": rid }),
    );

    // Keep the transition fence through the old generation's unscoped UI
    // events so a newer start cannot publish `starting` and then be reset by
    // this cancellation's trailing `idle`. Diagnostics completion is already
    // asynchronous, so releasing immediately before it never waits on SQLite.
    drop(transition);
    complete_dictation_performance(
        &state.performance,
        rid,
        RunOutcomeV1::Cancelled {
            stage: PerformanceStageV1::InferenceDecode,
        },
        Vec::new(),
        None,
        None,
    );

    Ok(())
}

/// Cancel only an in-flight microphone initialization after an environment
/// change. A live recording is deliberately left alone.
#[tauri::command]
pub fn cancel_audio_initialization(reason: String) -> Result<(), String> {
    let reason = match reason.as_str() {
        "device_changed" => AudioCancelReason::DeviceChanged,
        _ => return Err("Unsupported audio initialization cancellation reason".to_string()),
    };
    audio_lifecycle::cancel_starting_for_environment_change(reason);
    Ok(())
}

/// Apply the same local alias matcher used by live dictation without recording,
/// persisting, copying, or logging any user text.
#[tauri::command]
pub async fn preview_vocabulary_aliases(
    entries: Vec<crate::state::VocabularyEntry>,
    text: String,
    cli_formatting: bool,
) -> Result<String, String> {
    crate::vocabulary_alias::validate_entries(&entries)?;
    let matchers =
        crate::vocabulary_alias::CorrectionMatcherSet::build(&[], &entries, &[], false, false);
    let corrected = matchers.select(None).apply(&text);
    if cli_formatting {
        Ok(crate::cli_command::canonicalize_cli(
            &corrected,
            crate::cli_command::CliFormattingMode::Auto,
            &crate::cli_command::CliLexicon::from_context(None, &[]),
        ))
    } else {
        Ok(corrected)
    }
}

/// Transcribe an existing audio file (WAV/MP3/M4A) through the same Whisper
/// backend and settings as live dictation.
///
/// Unlike live recording, this path deliberately does **not** inject/paste the
/// result or drive the Idle/Recording/Processing status machine — a file
/// transcription is a one-shot "give me the text" action whose result is shown
/// in the UI. It decodes, downmixes to mono, resamples to 16kHz, applies VAD,
/// transcribes, and returns the text plus the decoded audio duration (seconds).
#[tauri::command]
pub async fn transcribe_file(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, State>,
    file_path: String,
) -> Result<serde_json::Value, String> {
    let transition = crate::commands::microphone_preview::transition_after_stopping_preview(
        &app_handle,
        state.inner(),
    )
    .await?;
    if state.app_state.meeting_blocks_asr() {
        return Err(
            "Wait for the meeting transcript to finish before transcribing a file.".to_string(),
        );
    }
    // Mutual exclusion with live dictation: both share one Whisper backend.
    // Claim the slot first (so a racing `start_native_recording` is blocked),
    // then refuse if a live recording/processing is already underway. The guard
    // releases the claim on every return path below.
    if state
        .app_state
        .file_transcribing
        .swap(true, Ordering::SeqCst)
    {
        return Err("Already transcribing a file.".to_string());
    }
    drop(transition);
    let _file_guard = FileTranscribeGuard {
        app_state: &state.app_state,
        app_handle: Some(app_handle.clone()),
    };
    let _ = app_handle.emit("file-transcription-status-changed", true);
    {
        let dictation = state.app_state.dictation.lock_or_recover();
        if state.benchmark.is_running() {
            return Err("Wait for the benchmark to finish before transcribing a file.".to_string());
        }
        #[cfg(feature = "internal-benchmark")]
        if state.corpus.is_active() {
            return Err("Finish the corpus recording before transcribing a file.".to_string());
        }

        if dictation.status != DictationStatus::Idle {
            return Err(
                "Can't transcribe a file while recording or processing live audio. \
                 Stop the current recording first."
                    .to_string(),
            );
        }
    }
    let file_run_id = state.app_state.next_file_run_id();
    if let Err(error) = state
        .performance
        .begin_file_transcription(file_run_id, Vec::new())
    {
        tracing::warn!(
            target: "system",
            file_run_id,
            "performance file run start failed: {}",
            error
        );
    }
    let mut performance_guard = state.performance.guard(
        RunCorrelationV1::FileTranscription { file_run_id },
        PerformanceStageV1::FileDecode,
    );
    let total_started = std::time::Instant::now();

    // Log only the extension as a structured field — never the raw path, which
    // would carry the user's home dir/username into telemetry (release builds
    // strip string fields from pipeline events, but not the message text).
    let ext = std::path::Path::new(&file_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    tracing::info!(target: "pipeline", file_run_id, ext = ext, "transcribe_file: start");

    // Phase: decode + downmix + resample to 16kHz mono (off the async runtime).
    let t_decode = std::time::Instant::now();
    let path_for_decode = file_path.clone();
    let samples =
        tokio::task::spawn_blocking(move || audio_decode::decode_to_mono_16k(&path_for_decode))
            .await
            .map_err(|e| format!("Decode task panicked: {}", e))??;
    if samples.is_empty() {
        return Err("No audio samples decoded from file".to_string());
    }
    // Precise seconds (f64): integer division floors sub-second clips to 0.
    let duration_secs = samples.len() as f64 / 16_000.0;
    tracing::info!(target: "pipeline", "transcribe_file: decoded {} samples (~{:.1}s) in {:?}",
        samples.len(), duration_secs, t_decode.elapsed());
    let file_decode_ms = t_decode.elapsed().as_millis() as u64;
    performance_guard.record(StageTimingV1::measured(
        PerformanceStageV1::FileDecode,
        file_decode_ms,
    ));

    // Read the settings shared with live dictation in one lock.
    let (model_name, language, vad_sensitivity, custom_vocabulary, smart_punctuation) = {
        let dictation = state.app_state.dictation.lock_or_recover();
        (
            dictation.model_name.clone(),
            dictation.language.clone(),
            dictation.vad_sensitivity,
            dictation.custom_vocabulary.clone(),
            dictation.smart_punctuation,
        )
    };
    let _ = state.performance.update_active(
        &RunCorrelationV1::FileTranscription { file_run_id },
        |active| {
            active.runtimes = runtime_identity(&model_name, ModelWarmStateV1::Unknown);
        },
    );

    // Phase: VAD — skip silence, best-effort with fallback to full audio (mirrors live).
    performance_guard.enter(PerformanceStageV1::Vad);
    let vad_started = std::time::Instant::now();
    let (samples_for_transcription, vad_trimmed) = if !vad::is_enabled(vad_sensitivity) {
        tracing::info!(target: "pipeline", "transcribe_file: VAD disabled for lowest-latency transcription");
        (samples.clone(), false)
    } else {
        let vad_threshold = vad::threshold_for_sensitivity(vad_sensitivity);
        match vad::vad_model_path() {
            Some(vad_path) if vad_path.exists() => {
                let vad_path_str = vad_path.to_string_lossy().to_string();
                let samples_owned = samples.clone();
                let vad_result = tokio::task::spawn_blocking(move || {
                    vad::filter_speech(&vad_path_str, &samples_owned, vad_threshold)
                })
                .await
                .unwrap_or_else(|e| Err(format!("VAD task panicked: {}", e)));

                match vad_result {
                    Ok(vad::VadResult::NoSpeech) => {
                        tracing::info!(target: "pipeline", "transcribe_file: VAD detected no speech");
                        let _ = performance_guard.finish(
                            RunOutcomeV1::NoSpeech,
                            vec![
                                StageTimingV1::measured(
                                    PerformanceStageV1::Vad,
                                    vad_started.elapsed().as_millis() as u64,
                                ),
                                StageTimingV1::measured(
                                    PerformanceStageV1::TotalProcessing,
                                    total_started.elapsed().as_millis() as u64,
                                ),
                            ],
                            Some(ContentFreeInputSummaryV1::audio(
                                (duration_secs * 1_000.0).round() as u64,
                            )),
                            Some(runtime_identity(&model_name, ModelWarmStateV1::Unknown)),
                        );
                        return Ok(serde_json::json!({
                            "type": "file_transcription", "text": "", "duration": duration_secs
                        }));
                    }
                    Ok(vad::VadResult::Speech(trimmed)) => {
                        let vad_trimmed = trimmed.len() != samples.len();
                        (trimmed, vad_trimmed)
                    }
                    Err(e) => {
                        tracing::warn!(target: "pipeline", "transcribe_file: VAD failed ({}), proceeding without filtering", e);
                        (samples.clone(), false)
                    }
                }
            }
            _ => {
                // VAD model missing — kick off a background download for next time.
                let handle = app_handle.clone();
                tokio::spawn(async move {
                    if let Err(e) = super::models::ensure_vad_model(&handle).await {
                        tracing::warn!(target: "pipeline", "transcribe_file: VAD model download failed ({})", e);
                    }
                });
                (samples.clone(), false)
            }
        }
    };
    let vad_ms = vad_started.elapsed().as_millis() as u64;

    // Phase: transcription (lazy model load), mirroring run_transcription_pipeline.
    performance_guard.enter(PerformanceStageV1::InferenceDecode);
    let t_transcribe = std::time::Instant::now();
    let sanitized = custom_vocabulary.replace('\0', "");
    // Imported files have no frontmost-app context, so developer vocabulary is
    // not applied. Explicit preferred terms remain in `custom_vocabulary`.
    let code_vocab = resolve_code_vocab_prompt(&state.app_state, None);
    let prompt = combine_prompts(&sanitized, &code_vocab);
    let mut decode_ms = 0;
    let (text, load_report) = state.app_state.model_runtime.with_ready_backend(
        Some(&app_handle),
        &model_name,
        PreparationReason::FileTranscription,
        |backend| {
            let decode_started = std::time::Instant::now();
            let result = transcribe_with_coreml_vad_retry(
                backend,
                &model_name,
                &samples_for_transcription,
                &samples,
                vad_trimmed,
                &language,
                prompt.as_deref(),
                smart_punctuation,
            );
            decode_ms = decode_started.elapsed().as_millis() as u64;
            result
        },
    )?;
    let model_load_ms = load_report.load_ms;
    // Imported files retain their existing raw-ASR output. They still pass through
    // the same authoritative transformation entry point with every stage disabled,
    // leaving delivery/UI behavior byte-for-byte unchanged.
    let transform_context = crate::transcript_transform::TranscriptContext {
        session_id: state.app_state.next_transcript_session_id(),
        source: crate::transcript_transform::TranscriptSource::File,
        context_handle: None,
        cli_formatting_mode: crate::cli_command::CliFormattingMode::Auto,
        stages: crate::transcript_transform::TranscriptStageConfig::verbatim(),
    };
    performance_guard.enter(PerformanceStageV1::TranscriptTransform);
    let transform_started = std::time::Instant::now();
    let transformed = crate::transcript_transform::transform_transcript(
        text,
        &transform_context,
        crate::transcript_transform::TranscriptTransformResources::empty(),
    )
    .map_err(|error| error.to_string())?;
    let transform_ms = transform_started.elapsed().as_millis() as u64;
    let transform_stages = transformed
        .stages
        .iter()
        .filter_map(transcript_stage_timing)
        .collect::<Vec<_>>();
    let text = transformed.text;

    *state.app_state.last_transcription_at.lock_or_recover() = Some(std::time::Instant::now());

    let word_count = if text.trim().is_empty() {
        0
    } else {
        text.split_whitespace().count()
    };
    tracing::info!(
        target: "pipeline",
        file_run_id,
        model_load_ms,
        decode_ms,
        inference_ms = t_transcribe.elapsed().as_millis() as u64,
        audio_secs = duration_secs,
        word_count = word_count,
        model = model_name.as_str(),
        "file transcription complete"
    );

    let warm_state = if load_report.cache_hit {
        ModelWarmStateV1::Warm
    } else {
        ModelWarmStateV1::ColdLoaded
    };
    let mut stages = vec![
        StageTimingV1::measured(PerformanceStageV1::FileDecode, file_decode_ms),
        StageTimingV1::measured(PerformanceStageV1::Vad, vad_ms),
        StageTimingV1::measured(PerformanceStageV1::InferenceQueue, load_report.lock_wait_ms),
        StageTimingV1::measured(PerformanceStageV1::ModelLoad, model_load_ms),
        StageTimingV1::measured(PerformanceStageV1::InferenceDecode, decode_ms),
        StageTimingV1::measured(PerformanceStageV1::TranscriptTransform, transform_ms),
        StageTimingV1::measured(PerformanceStageV1::FileReturn, 0),
        StageTimingV1::measured(
            PerformanceStageV1::TotalProcessing,
            total_started.elapsed().as_millis() as u64,
        ),
    ];
    stages.extend(transform_stages);
    let _ = performance_guard.finish(
        RunOutcomeV1::Success,
        stages,
        Some(ContentFreeInputSummaryV1::audio_with_output(
            (duration_secs * 1_000.0).round() as u64,
            text.len(),
        )),
        Some(runtime_identity(&model_name, warm_state)),
    );

    Ok(serde_json::json!({
        "type": "file_transcription",
        "fileRunId": file_run_id,
        "text": text,
        "duration": duration_secs
    }))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::VecDeque;

    #[test]
    fn a_committed_permission_relaunch_refuses_a_dictation_start() {
        let state = crate::State::for_test();
        assert_eq!(dictation_start_blocker(&state, DictationStatus::Idle), None);
        assert_eq!(dictation_prepare_refusal(&state), None);
        let _committed = crate::commands::permissions::CommittedRelaunchForTest::raise();
        assert_eq!(
            dictation_start_blocker(&state, DictationStatus::Idle),
            Some(DictationStartBlocker {
                kind: "busy_restarting",
                state: "idle"
            })
        );
        // Key-down admission refuses too, but the relaunch's own commitment is
        // not "activity" to the relaunch gate.
        assert!(dictation_prepare_refusal(&state).is_some());
        assert_eq!(dictation_relaunch_busy(&state), None);
    }

    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use tauri::Listener;
    use tracing_subscriber::layer::{Context, SubscriberExt};
    use tracing_subscriber::{Layer, Registry};

    #[derive(Clone, Default)]
    struct CompletionCaptureLayer(Arc<Mutex<Vec<String>>>);

    impl<S: tracing::Subscriber> Layer<S> for CompletionCaptureLayer {
        fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
            struct Visitor(Vec<String>);

            impl tracing::field::Visit for Visitor {
                fn record_debug(
                    &mut self,
                    field: &tracing::field::Field,
                    value: &dyn std::fmt::Debug,
                ) {
                    self.0.push(format!("{}={value:?}", field.name()));
                }
            }

            let mut visitor = Visitor(Vec::new());
            event.record(&mut visitor);
            self.0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .extend(visitor.0);
        }
    }

    fn capture_native_completion(
        outcome: DictationTerminalOutcome,
        char_count: usize,
    ) -> Vec<String> {
        let layer = CompletionCaptureLayer::default();
        let captured = Arc::clone(&layer.0);
        let subscriber = Registry::default().with(layer);
        let timings = PipelineTimings {
            vad_ms: 7,
            inference_ms: 180,
            paste_ms: 12,
            ..PipelineTimings::default()
        };

        tracing::subscriber::with_default(subscriber, || {
            emit_native_dictation_metrics(NativeDictationMetrics {
                recording_id: 41,
                outcome,
                timings: &timings,
                total_ms: 220,
                audio_secs: 1.5,
                word_count: 3,
                char_count,
                model_name: "test-model",
                backend_name: "test-backend",
            });
        });

        let fields = captured
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        fields
    }

    #[test]
    fn stopped_paste_is_unconfirmed_and_never_clipboard_write_failed() {
        for reason in [
            injector::authority::AttemptStop::Cancelled,
            injector::authority::AttemptStop::Expired,
        ] {
            assert_eq!(
                confirmed_delivery_outcome(injector::InjectionOutcome::Stopped(reason)),
                DictationDeliveryOutcome::Unconfirmed
            );
        }
    }

    #[test]
    fn native_completion_emitter_declares_aggregator_contract() {
        let fields = capture_native_completion(DictationTerminalOutcome::Success, 18);
        assert!(fields.contains(&"event_code=\"pipeline.dictation_completed\"".to_string()));
        assert!(fields.contains(&"recording_id=41".to_string()));
        assert!(fields.contains(&"total_ms=220".to_string()));
        assert!(fields.contains(&"char_count=18".to_string()));
    }

    #[test]
    fn native_partial_interruption_emits_neutral_metrics() {
        let fields = capture_native_completion(DictationTerminalOutcome::RuntimeInterruption, 18);
        assert!(fields.contains(&"event_code=\"pipeline.dictation_metrics\"".to_string()));
        assert!(!fields.contains(&"event_code=\"pipeline.dictation_completed\"".to_string()));
        assert!(fields.contains(&"char_count=18".to_string()));
        assert!(fields.contains(&"total_ms=220".to_string()));
    }

    #[test]
    fn dictation_partial_window_is_bounded_to_the_trailing_twenty_seconds() {
        let samples: Vec<f32> = (0..PARTIAL_WINDOW_SAMPLES + 37)
            .map(|index| index as f32)
            .collect();
        let window = dictation_partial_window(&samples);
        assert_eq!(window.len(), PARTIAL_WINDOW_SAMPLES);
        assert_eq!(window[0], 37.0);
        let short = vec![0.0; PARTIAL_MIN_SAMPLES];
        assert_eq!(dictation_partial_window(&short).len(), short.len());
    }

    #[test]
    fn dictation_partial_ownership_is_single_flight_and_generation_gated() {
        let app_state = AppState::default();
        let recording_id = 41;
        app_state.recording_id.store(recording_id, Ordering::SeqCst);
        app_state.dictation.lock_or_recover().status = DictationStatus::Recording;
        assert!(try_begin_dictation_partial(&app_state, recording_id));
        assert!(!try_begin_dictation_partial(&app_state, recording_id));
        finish_dictation_partial(&app_state, recording_id);
        assert!(try_begin_dictation_partial(&app_state, recording_id));
        app_state
            .recording_id
            .store(recording_id + 1, Ordering::SeqCst);
        finish_dictation_partial(&app_state, recording_id);
        assert!(!try_begin_dictation_partial(&app_state, recording_id));
    }

    pub(crate) fn partial_lease_fixture() -> (tauri::App<tauri::test::MockRuntime>, impl Send) {
        let recording_id = 41;
        let app_state = AppState::default();
        app_state.recording_id.store(recording_id, Ordering::SeqCst);
        app_state.dictation.lock_or_recover().status = DictationStatus::Recording;
        assert!(try_begin_dictation_partial(&app_state, recording_id));
        let app = tauri::test::mock_builder()
            .manage(State {
                app_state,
                benchmark: Arc::new(crate::benchmark::BenchmarkCoordinator::new()),
                microphone_startup_benchmark: crate::commands::microphone_startup_benchmark::MicrophoneStartupBenchmarkState::default(),
                #[cfg(feature = "internal-benchmark")]
                corpus: crate::commands::corpus::CorpusRecorderState::default(),
                meeting_store: crate::meeting_store::MeetingStore::default(),
                meetings: crate::meeting_capture::MeetingCoordinator::default(),
                meeting_suggestions: crate::meeting_suggestions::MeetingSuggestions::default(),
                delivery_recovery: crate::delivery_recovery::DeliveryRecoveryState::default(),
                capture_health: crate::capture_health::CaptureHealthDiagnostics::default(),
                performance: crate::performance_metrics::PerformanceMetrics::default(),

                dictation_diagnostics: crate::dictation_diagnostics::DictationDiagnostics::default(
                ),
                notch_info: std::sync::Mutex::new(None),
                display_snapshot: std::sync::Mutex::new(None),
                overlay_calibration: std::sync::Mutex::new(
                    crate::commands::overlay::OverlayCalibrationState::default(),
                ),
                overlay_frame: std::sync::Mutex::new(None),
                overlay_layout: std::sync::Mutex::new(crate::commands::overlay::OverlayLayout::default()),

            })
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("build mock Tauri app");
        let lease = DictationPartialLease {
            app: app.handle().clone(),
            recording_id,
        };
        (app, lease)
    }

    #[test]
    fn stopped_partial_cannot_publish_or_release_a_new_recording_lease() {
        let app_state = AppState::default();
        app_state.recording_id.store(41, Ordering::SeqCst);
        app_state.dictation.lock_or_recover().status = DictationStatus::Recording;
        assert!(try_begin_dictation_partial(&app_state, 41));
        app_state.dictation.lock_or_recover().status = DictationStatus::Processing;
        assert!(!dictation_partial_is_current(&app_state, 41));
        app_state.recording_id.store(42, Ordering::SeqCst);
        app_state.dictation.lock_or_recover().status = DictationStatus::Recording;
        assert!(!dictation_partial_is_current(&app_state, 41));
        // A stale decode still owns the global slot until it actually finishes.
        assert!(!try_begin_dictation_partial(&app_state, 42));
        finish_dictation_partial(&app_state, 41);
        assert!(try_begin_dictation_partial(&app_state, 42));
        finish_dictation_partial(&app_state, 41);
        assert!(!try_begin_dictation_partial(&app_state, 42));
        app_state.cancel_recording(42);
        assert!(!dictation_partial_is_current(&app_state, 42));
        finish_dictation_partial(&app_state, 42);
    }

    #[test]
    fn concurrent_preview_ticks_admit_exactly_one_decode() {
        let app_state = Arc::new(AppState::default());
        app_state.recording_id.store(7, Ordering::SeqCst);
        app_state.dictation.lock_or_recover().status = DictationStatus::Recording;
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let state = app_state.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    try_begin_dictation_partial(&state, 7)
                })
            })
            .collect();
        let admitted = workers
            .into_iter()
            .map(|worker| usize::from(worker.join().unwrap()))
            .sum::<usize>();
        assert_eq!(admitted, 1);
        finish_dictation_partial(&app_state, 7);
        assert!(try_begin_dictation_partial(&app_state, 7));
    }

    #[test]
    fn delivery_snapshot_survives_completion_before_diagnostics_begin() {
        let temp = tempfile::tempdir().unwrap();
        let performance = crate::performance_metrics::PerformanceMetrics::default();
        performance
            .initialize(temp.path().join("diagnostics"), None)
            .unwrap();
        let coordinator = DictationPerformanceBeginCoordinator::default();
        let recording_id = 891;
        coordinator.register(recording_id);
        let timing = dt::DeliveryTiming::new(std::time::Instant::now());
        timing.update(|r| dt::set_outcome(r, dt::Outcome::TargetChanged));
        let mut completion = failed_capture_completion();
        completion.delivery_timing = Some(Box::new(timing.close()));
        assert!(matches!(
            coordinator.request_completion(recording_id, completion),
            DictationPerformanceCompletionAction::Deferred
        ));
        performance
            .begin_dictation(recording_id, Vec::new())
            .unwrap();
        let resolution = coordinator
            .resolve_begin(recording_id, true, &performance)
            .unwrap();
        flush_dictation_performance_begin(&coordinator, &performance, recording_id, resolution);
        let run = performance.list(10).unwrap().runs.remove(0);
        assert_eq!(
            run.delivery_timing.unwrap().outcome,
            DeliveryMeasurement::measured(dt::Outcome::TargetChanged)
        );
    }

    fn failed_capture_completion() -> DictationPerformanceCompletion {
        DictationPerformanceCompletion {
            delivery_timing: None,
            outcome: RunOutcomeV1::Failed {
                stage: PerformanceStageV1::CaptureFinalization,
                error_code: StableRunErrorV1::AudioCaptureFailed,
            },
            stages: Vec::new(),
            input: None,
            runtimes: None,
        }
    }

    fn wait_for_completed_runs(
        performance: &crate::performance_metrics::PerformanceMetrics,
        expected: usize,
    ) -> Vec<crate::performance_metrics::PerformanceRunV1> {
        for _ in 0..100 {
            let runs = performance.list(10).expect("list completed runs").runs;
            if runs.len() == expected {
                return runs;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("timed out waiting for {expected} completed diagnostics runs");
    }

    fn register_pending_shadow_permit(
        performance: &crate::performance_metrics::PerformanceMetrics,
        recording_id: u64,
    ) {
        let permit = crate::shadow_measurement::ShadowPermit::acquire()
            .expect("the previous run must have released its shadow permit");
        performance.register_shadow(
            recording_id,
            Box::new(move |_, _| {
                let _permit = permit;
                panic!("an abandoned run must never execute its shadow job");
            }),
        );
    }

    #[test]
    fn failed_begin_releases_shadow_registered_before_failed_resolution() {
        let coordinator = DictationPerformanceBeginCoordinator::default();
        let performance = crate::performance_metrics::PerformanceMetrics::default();
        let recording_id = 536_010;
        coordinator.register(recording_id);
        // Reproduce the old failed-begin worker's early discard, then let the
        // pipeline register and defer completion before begin failure resolves.
        performance.discard_shadow(recording_id);
        register_pending_shadow_permit(&performance, recording_id);
        assert!(matches!(
            coordinator.request_completion(recording_id, failed_capture_completion()),
            DictationPerformanceCompletionAction::Deferred
        ));
        assert!(coordinator
            .resolve_begin(recording_id, false, &performance)
            .is_none());
        assert!(!coordinator.begin_is_pending(recording_id));
        let next = crate::shadow_measurement::ShadowPermit::acquire();
        assert!(
            next.is_some(),
            "failed begin must release the abandoned job's global permit"
        );
        drop(next);
        // The opposite ordering also releases: once failure has retracted the
        // deferral entry, a late pipeline must try persistence, not defer.
        register_pending_shadow_permit(&performance, recording_id);
        let late = match coordinator.request_completion(recording_id, failed_capture_completion()) {
            DictationPerformanceCompletionAction::Persist(completion) => completion,
            _ => panic!("a late completion cannot defer after failed begin"),
        };
        assert!(
            persist_dictation_performance_completion(&performance, recording_id, late).is_err()
        );
        assert!(crate::shadow_measurement::ShadowPermit::acquire().is_some());
    }

    #[test]
    fn abort_releases_pending_shadow_with_or_without_deferred_completion() {
        for deferred in [false, true] {
            let coordinator = DictationPerformanceBeginCoordinator::default();
            let performance = crate::performance_metrics::PerformanceMetrics::default();
            let recording_id = 536_011;
            coordinator.register(recording_id);
            register_pending_shadow_permit(&performance, recording_id);
            if deferred {
                assert!(matches!(
                    coordinator.request_completion(recording_id, failed_capture_completion()),
                    DictationPerformanceCompletionAction::Deferred
                ));
            }
            coordinator.abort(recording_id, &performance);
            assert!(!coordinator.begin_is_pending(recording_id));
            assert!(crate::shadow_measurement::ShadowPermit::acquire().is_some());
        }
    }

    #[test]
    fn completed_run_releases_shadow_registered_after_persistence() {
        let root = tempfile::tempdir().unwrap();
        let performance = crate::performance_metrics::PerformanceMetrics::default();
        performance
            .initialize(root.path().to_path_buf(), None)
            .unwrap();
        let coordinator = DictationPerformanceBeginCoordinator::default();
        let recording_id = 536_012;
        coordinator.register(recording_id);
        performance
            .begin_dictation(recording_id, Vec::new())
            .unwrap();
        coordinator
            .resolve_begin(recording_id, true, &performance)
            .unwrap();
        let completion =
            match coordinator.request_completion(recording_id, failed_capture_completion()) {
                DictationPerformanceCompletionAction::Persist(completion) => completion,
                _ => panic!("ready begin must claim completion"),
            };
        persist_dictation_performance_completion(&performance, recording_id, completion).unwrap();
        register_pending_shadow_permit(&performance, recording_id);
        coordinator.finish_completion(recording_id, &performance);
        assert!(crate::shadow_measurement::ShadowPermit::acquire().is_some());
    }

    #[test]
    fn ignored_terminal_releases_late_shadow_after_persistence_failure() {
        let performance = crate::performance_metrics::PerformanceMetrics::default();
        let coordinator = dictation_performance_begins();
        let recording_id = 536_013;
        coordinator.register(recording_id);
        coordinator
            .resolve_begin(recording_id, true, &performance)
            .unwrap();
        let first = failed_capture_completion();
        let original_outcome = first.outcome.clone();
        let claimed = match coordinator.request_completion(recording_id, first) {
            DictationPerformanceCompletionAction::Persist(completion) => completion,
            _ => panic!("ready begin must claim completion"),
        };
        assert!(
            persist_dictation_performance_completion(&performance, recording_id, claimed).is_err()
        );
        register_pending_shadow_permit(&performance, recording_id);
        complete_dictation_performance(
            &performance,
            recording_id,
            RunOutcomeV1::NoSpeech,
            Vec::new(),
            None,
            None,
        );
        assert_eq!(
            coordinator.claimed_outcome(recording_id),
            Some(original_outcome)
        );
        assert!(crate::shadow_measurement::ShadowPermit::acquire().is_some());
        coordinator.finish_completion(recording_id, &performance);
    }

    #[test]
    fn terminal_before_diagnostics_begin_is_deferred_and_keeps_first_outcome() {
        let performance = crate::performance_metrics::PerformanceMetrics::default();
        let coordinator = DictationPerformanceBeginCoordinator::default();
        let recording_id = 536_001;
        coordinator.register(recording_id);

        assert!(matches!(
            coordinator.request_completion(recording_id, failed_capture_completion()),
            DictationPerformanceCompletionAction::Deferred
        ));
        assert!(matches!(
            coordinator.request_completion(
                recording_id,
                DictationPerformanceCompletion {
                    delivery_timing: None,
                    outcome: RunOutcomeV1::Cancelled {
                        stage: PerformanceStageV1::CaptureFinalization,
                    },
                    stages: Vec::new(),
                    input: None,
                    runtimes: None,
                },
            ),
            DictationPerformanceCompletionAction::AlreadyCompleting
        ));

        let resolution = coordinator
            .resolve_begin(recording_id, true, &performance)
            .expect("the first terminal waits for begin");
        let pending = resolution.completion.expect("queued terminal outcome");
        assert!(matches!(
            pending.outcome,
            RunOutcomeV1::Failed {
                error_code: StableRunErrorV1::AudioCaptureFailed,
                ..
            }
        ));
        assert!(coordinator.begin_is_pending(recording_id));
        coordinator.finish_completion(recording_id, &performance);
        assert!(!coordinator.begin_is_pending(recording_id));
    }

    #[test]
    fn failed_capture_queued_before_begin_persists_once_after_begin() {
        assert_eq!(
            native_start_context_action(true, DictationStatus::Idle),
            NativeStartContextAction::FlushLifecycleTerminal {
                install_context: false,
            },
            "current-generation Idle must flush, not abort, its reserved begin"
        );
        assert_eq!(
            native_start_context_action(true, DictationStatus::Recovering),
            NativeStartContextAction::FlushLifecycleTerminal {
                install_context: true,
            },
            "Recovering keeps context for an interrupted transcription"
        );
        let root = tempfile::tempdir().expect("temporary diagnostics root");
        let performance = crate::performance_metrics::PerformanceMetrics::default();
        performance
            .initialize(root.path().to_path_buf(), None)
            .expect("initialize performance diagnostics");
        let recording_id = 536_002;
        let coordinator = DictationPerformanceBeginCoordinator::default();
        coordinator.register(recording_id);
        assert!(matches!(
            coordinator.request_completion(recording_id, failed_capture_completion()),
            DictationPerformanceCompletionAction::Deferred
        ));

        performance
            .begin_dictation(recording_id, Vec::new())
            .expect("begin delayed diagnostics row");
        let resolution = coordinator
            .resolve_begin(recording_id, true, &performance)
            .expect("queued terminal outcome");
        let pending = resolution.completion.expect("queued terminal completion");
        persist_dictation_performance_completion(&performance, recording_id, pending)
            .expect("persist queued terminal");
        coordinator.finish_completion(recording_id, &performance);

        let runs = wait_for_completed_runs(&performance, 1);
        assert_eq!(runs.len(), 1);
        assert_eq!(
            runs[0].outcome,
            RunOutcomeV1::Failed {
                stage: PerformanceStageV1::CaptureFinalization,
                error_code: StableRunErrorV1::AudioCaptureFailed,
            }
        );
    }

    #[test]
    fn terminal_after_diagnostics_begin_can_persist_immediately() {
        let performance = crate::performance_metrics::PerformanceMetrics::default();
        let coordinator = DictationPerformanceBeginCoordinator::default();
        let recording_id = 536_003;
        coordinator.register(recording_id);
        assert!(coordinator
            .resolve_begin(recording_id, true, &performance)
            .is_some());
        assert!(coordinator.begin_is_pending(recording_id));
        let claimed =
            match coordinator.request_completion(recording_id, failed_capture_completion()) {
                DictationPerformanceCompletionAction::Persist(completion) => completion,
                _ => panic!("Ready must claim the first terminal for persistence"),
            };
        assert!(coordinator.completion_is_claimed(recording_id));
        let unavailable = crate::performance_metrics::PerformanceMetrics::default();
        assert!(
            persist_dictation_performance_completion(&unavailable, recording_id, claimed).is_err(),
            "the failure seam must exercise a real rejected persistence attempt"
        );
        assert!(matches!(
            coordinator.request_completion(
                recording_id,
                DictationPerformanceCompletion {
                    delivery_timing: None,
                    outcome: RunOutcomeV1::Cancelled {
                        stage: PerformanceStageV1::CaptureFinalization,
                    },
                    stages: Vec::new(),
                    input: None,
                    runtimes: None,
                }
            ),
            DictationPerformanceCompletionAction::AlreadyCompleting
        ));
        // A failed persistence attempt deliberately leaves the exact first
        // completion claimed; only a successful persist removes the entry.
        assert!(coordinator.completion_is_claimed(recording_id));
        assert!(matches!(
            coordinator.claimed_outcome(recording_id),
            Some(RunOutcomeV1::Failed {
                error_code: StableRunErrorV1::AudioCaptureFailed,
                ..
            })
        ));
        coordinator.finish_completion(recording_id, &performance);
        assert!(!coordinator.begin_is_pending(recording_id));
    }

    #[test]
    fn deferred_native_guard_retains_capture_finalization_timing() {
        let root = tempfile::tempdir().expect("temporary diagnostics root");
        let performance = crate::performance_metrics::PerformanceMetrics::default();
        performance
            .initialize(root.path().to_path_buf(), None)
            .expect("initialize performance diagnostics");
        let recording_id = 536_004;
        let coordinator = dictation_performance_begins();
        coordinator.register(recording_id);

        let mut guard = NativeDictationPerformanceGuard::new(
            performance.clone(),
            recording_id,
            PerformanceStageV1::CaptureFinalization,
        );
        guard.record(StageTimingV1::measured(
            PerformanceStageV1::CaptureFinalization,
            37,
        ));
        guard.finish(
            RunOutcomeV1::NoSpeech,
            vec![StageTimingV1::measured(
                PerformanceStageV1::TotalProcessing,
                41,
            )],
            Some(ContentFreeInputSummaryV1::audio(100)),
            None,
        );

        performance
            .begin_dictation(recording_id, Vec::new())
            .expect("begin delayed diagnostics row");
        let resolution = coordinator
            .resolve_begin(recording_id, true, &performance)
            .expect("native terminal waits for begin");
        flush_dictation_performance_begin(coordinator, &performance, recording_id, resolution);

        let runs = performance.list(10).expect("list completed runs").runs;
        let capture = runs[0]
            .stages
            .iter()
            .find(|stage| stage.stage == PerformanceStageV1::CaptureFinalization)
            .expect("capture finalization stage");
        assert_eq!(
            capture.duration_ms,
            crate::performance_metrics::MeasurementV1::measured(37)
        );
    }

    #[test]
    fn begin_registration_cleans_up_if_worker_unwinds() {
        let performance = crate::performance_metrics::PerformanceMetrics::default();
        let coordinator = DictationPerformanceBeginCoordinator::default();
        let recording_id = 536_005;
        coordinator.register(recording_id);
        register_pending_shadow_permit(&performance, recording_id);

        let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _registration = DictationPerformanceBeginRegistration::new(
                &coordinator,
                recording_id,
                &performance,
            );
            panic!("forced begin worker unwind");
        }));

        assert!(unwind.is_err());
        assert!(crate::shadow_measurement::ShadowPermit::acquire().is_some());
        assert!(!coordinator.begin_is_pending(recording_id));
        assert!(matches!(
            coordinator.request_completion(recording_id, failed_capture_completion()),
            DictationPerformanceCompletionAction::Persist(_)
        ));
    }

    #[test]
    fn delayed_begin_flushes_latest_stage_and_preserves_request_timestamp() {
        let root = tempfile::tempdir().expect("temporary diagnostics root");
        let performance = crate::performance_metrics::PerformanceMetrics::default();
        performance
            .initialize(root.path().to_path_buf(), None)
            .expect("initialize performance diagnostics");
        let coordinator = DictationPerformanceBeginCoordinator::default();
        let recording_id = 536_006;
        let requested_at_ms = 1_725_000_000_123_i64;
        coordinator.register(recording_id);
        assert!(coordinator.defer_stage(recording_id, PerformanceStageV1::Vad));

        let active = performance
            .begin_dictation_diagnosed_at(recording_id, Vec::new(), requested_at_ms)
            .expect("begin delayed diagnostics row");
        assert_eq!(active.started_at_ms, requested_at_ms);
        let resolution = coordinator
            .resolve_begin(recording_id, true, &performance)
            .expect("successful delayed begin");
        assert_eq!(resolution.latest_stage, Some(PerformanceStageV1::Vad));
        flush_dictation_performance_begin(&coordinator, &performance, recording_id, resolution);
        drop(performance);

        let restarted = crate::performance_metrics::PerformanceMetrics::default();
        restarted
            .initialize(root.path().to_path_buf(), None)
            .expect("recover interrupted diagnostics row");
        let runs = restarted.list(10).expect("list recovered run").runs;
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].started_at_ms, requested_at_ms);
        assert!(matches!(
            runs[0].outcome,
            RunOutcomeV1::Interrupted {
                stage: PerformanceStageV1::Vad,
                ..
            }
        ));
    }

    #[test]
    fn model_preparation_starts_at_accepted_start_and_stops_during_recovery() {
        assert!(status_allows_model_preparation(DictationStatus::Starting));
        assert!(status_allows_model_preparation(DictationStatus::Recording));
        assert!(!status_allows_model_preparation(
            DictationStatus::Recovering
        ));
        assert!(!status_allows_model_preparation(DictationStatus::Idle));
        assert!(!status_allows_model_preparation(
            DictationStatus::Processing
        ));
    }

    #[test]
    fn reliability_codes_are_bounded_and_actions_follow_existing_presentations() {
        assert_eq!(DICTATION_SLO_CONTRACT, 1);
        assert_eq!(dictation_status_code(DictationStatus::Idle), "idle");
        assert_eq!(dictation_status_code(DictationStatus::Starting), "starting");
        assert_eq!(
            dictation_status_code(DictationStatus::Recording),
            "recording"
        );
        assert_eq!(
            dictation_status_code(DictationStatus::Recovering),
            "recovering"
        );
        assert_eq!(
            dictation_status_code(DictationStatus::Processing),
            "processing"
        );
        assert_eq!(
            DictationPresentation::CleanupInProgress.codes(),
            ("microphone_cleanup_in_progress", "wait")
        );
        assert_eq!(
            DictationPresentation::InitializationFailed(audio::AudioFailureKind::PermissionDenied)
                .codes(),
            (
                "microphone_initialization_failed",
                "open_microphone_settings"
            )
        );
        assert_eq!(
            DictationPresentation::InitializationFailed(audio::AudioFailureKind::BackendError)
                .codes(),
            ("microphone_initialization_failed", "retry")
        );
        assert_eq!(
            DictationPresentation::InitializationFailed(audio::AudioFailureKind::DeviceUnavailable)
                .codes(),
            ("microphone_initialization_failed", "choose_microphone")
        );
        assert_eq!(
            DictationPresentation::CleanupStalled.codes(),
            ("microphone_cleanup_stalled", "restart_app")
        );
        assert_eq!(
            DictationPresentation::Interrupted {
                partial_transcription: false,
            }
            .codes(),
            ("microphone_interrupted", "retry")
        );
        assert_eq!(
            DictationPresentation::Interrupted {
                partial_transcription: true,
            }
            .codes(),
            ("microphone_interrupted", "wait_for_partial_transcription")
        );
    }

    #[test]
    fn reliability_transition_seam_mutates_once_and_returns_prior_state() {
        let mut dictation = crate::state::DictationState::default();
        assert_eq!(
            transition_dictation_status(71, &mut dictation, DictationStatus::Starting),
            DictationStatus::Idle
        );
        assert_eq!(dictation.status, DictationStatus::Starting);
        assert_eq!(
            transition_dictation_status(71, &mut dictation, DictationStatus::Recovering),
            DictationStatus::Starting
        );
        assert_eq!(dictation.status, DictationStatus::Recovering);
        assert_eq!(
            transition_dictation_status(71, &mut dictation, DictationStatus::Idle),
            DictationStatus::Recovering
        );
        assert_eq!(dictation.status, DictationStatus::Idle);
    }

    #[test]
    fn accepted_request_is_published_before_the_starting_transition() {
        let mut dictation = crate::state::DictationState::default();
        let mut requested = false;

        let performance_started_at_ms =
            accept_live_dictation_request(72, &mut dictation, |status_at_acceptance| {
                assert_eq!(status_at_acceptance, DictationStatus::Idle);
                requested = true;
            });

        assert!(requested);
        assert!(performance_started_at_ms > 0);
        assert_eq!(dictation.status, DictationStatus::Starting);
    }

    #[test]
    fn only_the_resolved_toggle_stop_owns_a_published_stamp() {
        use crate::shadow_measurement::{self, StopPress, StopStamp};
        use audio_lifecycle::DictationStopTrigger;
        let entered = std::time::Instant::now();
        let released = entered - std::time::Duration::from_millis(12);
        let stamp = StopStamp {
            press: StopPress {
                recording_id: 1_082,
                pressed_at: released - std::time::Duration::from_millis(80),
                sample: Some(16_000),
                preview_held_backend: true,
            },
            released_at: released,
        };
        for trigger in [
            DictationStopTrigger::Toggle,
            DictationStopTrigger::HoldRelease,
            DictationStopTrigger::Overlay,
            DictationStopTrigger::AutoStop,
            DictationStopTrigger::Other,
        ] {
            for release in [
                None,
                Some(crate::hotkey_release::HotkeyRelease::for_test(
                    released, entered,
                )),
            ] {
                // A double-tap publishes first, but this command wins the stop.
                shadow_measurement::record(stamp);
                let pending = shadow_measurement::take_for(1_082, entered);
                assert!(pending.is_some());
                let resolved = trigger.resolve(release);
                let attributed = attributable_stop_stamp(resolved, pending);
                assert_eq!(
                    attributed.is_some(),
                    trigger == DictationStopTrigger::Toggle && release.is_none(),
                    "{trigger:?}"
                );
                if let Some(attributed) = attributed {
                    assert_eq!(attributed.press.sample, Some(16_000));
                    assert_eq!(attributed.stages(entered)[0].duration_ms.value(), Some(&80));
                    assert_eq!(attributed.stages(entered)[1].duration_ms.value(), Some(&12));
                }
                assert!(
                    shadow_measurement::take_for(1_082, entered).is_none(),
                    "the losing stamp must not leak to another stop"
                );
                assert!(attributable_stop_stamp(resolved, None).is_none());
            }
        }
    }

    #[test]
    fn asynchronous_stop_targets_only_its_original_recording_generation() {
        assert!(stop_owner_matches(41, None));
        assert!(stop_owner_matches(41, Some(41)));
        assert!(!stop_owner_matches(42, Some(41)));
    }

    #[test]
    fn capture_cleanup_completion_preserves_processing_and_rejects_old_generations() {
        let state = AppState::default();
        state.recording_id.store(710, Ordering::SeqCst);
        state.dictation.lock_or_recover().status = DictationStatus::Processing;
        complete_dictation_capture_cleanup(&state, 710, |restore_idle| assert!(!restore_idle));
        assert_eq!(
            state.dictation.lock_or_recover().status,
            DictationStatus::Processing
        );
        complete_dictation_capture_cleanup(&state, 709, |_| panic!("stale cleanup cannot publish"));
        assert_eq!(
            state.dictation.lock_or_recover().status,
            DictationStatus::Processing
        );
    }

    #[tokio::test]
    async fn connecting_preview_blocks_queued_preparation_before_supervisor_startup() {
        let state = Arc::new(State {
            app_state: AppState::default(),
            benchmark: Arc::new(crate::benchmark::BenchmarkCoordinator::new()),
            microphone_startup_benchmark: Default::default(),
            #[cfg(feature = "internal-benchmark")]
            corpus: Default::default(),
            meeting_store: Default::default(),
            meetings: Default::default(),
            meeting_suggestions: Default::default(),
            delivery_recovery: Default::default(),
            capture_health: Default::default(),
            performance: Default::default(),

            dictation_diagnostics: Default::default(),
            notch_info: Default::default(),
            display_snapshot: Default::default(),
            overlay_calibration: Default::default(),
            overlay_frame: Default::default(),
            overlay_layout: Default::default(),
        });
        let transition = state.app_state.recording_transition.lock().await;
        assert!(dictation_prepare_admitted(&state));
        let preview = state.app_state.microphone_preview.claim(0).unwrap();
        assert!(state.app_state.microphone_preview.is_connecting(preview));
        assert!(!audio_lifecycle::is_audio_active());
        let queued_state = Arc::clone(&state);
        let queued = tokio::spawn(async move {
            crate::audio::prepare::admission_transition(
                &queued_state.app_state.recording_transition,
                || dictation_prepare_admitted(&queued_state),
            )
            .await
            .is_some()
        });
        drop(transition);
        assert!(!queued.await.unwrap());
    }

    #[test]
    fn runtime_failure_bridge_emits_recovery_before_failure_and_records_one_failed_run() {
        let root = tempfile::tempdir().expect("performance diagnostics temp dir");
        let performance = crate::performance_metrics::PerformanceMetrics::default();
        performance
            .initialize(root.path().to_path_buf(), None)
            .expect("initialize performance diagnostics");

        let recording_id = 406;
        performance
            .begin_dictation(recording_id, Vec::new())
            .expect("begin dictation diagnostic");

        let app_state = AppState::default();
        app_state.recording_id.store(recording_id, Ordering::SeqCst);
        app_state.dictation.lock_or_recover().status = DictationStatus::Recording;

        let app = tauri::test::mock_builder()
            .manage(State {
                app_state,
                benchmark: Arc::new(crate::benchmark::BenchmarkCoordinator::new()),
                microphone_startup_benchmark: crate::commands::microphone_startup_benchmark::MicrophoneStartupBenchmarkState::default(),
                #[cfg(feature = "internal-benchmark")]
                corpus: crate::commands::corpus::CorpusRecorderState::default(),
                meeting_store: crate::meeting_store::MeetingStore::default(),
                meetings: crate::meeting_capture::MeetingCoordinator::default(),
                meeting_suggestions: crate::meeting_suggestions::MeetingSuggestions::default(),
                delivery_recovery: crate::delivery_recovery::DeliveryRecoveryState::default(),
                capture_health: crate::capture_health::CaptureHealthDiagnostics::default(),
                performance: performance.clone(),

                dictation_diagnostics: crate::dictation_diagnostics::DictationDiagnostics::default(
                ),
                notch_info: std::sync::Mutex::new(None),
                display_snapshot: std::sync::Mutex::new(None),
                overlay_calibration: std::sync::Mutex::new(
                    crate::commands::overlay::OverlayCalibrationState::default(),
                ),
                overlay_frame: std::sync::Mutex::new(None),
                overlay_layout: std::sync::Mutex::new(crate::commands::overlay::OverlayLayout::default()),

            })
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("build mock Tauri app");
        let app_handle = app.handle().clone();

        let emitted = Arc::new(std::sync::Mutex::new(Vec::new()));
        for event_name in ["audio-recovery-started", "recording-initialization-failed"] {
            let emitted = Arc::clone(&emitted);
            app_handle.listen_any(event_name, move |_| {
                emitted
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(event_name);
            });
        }

        handle_audio_lifecycle_with(
            app_handle.clone(),
            recording_id,
            AudioLifecycleEvent::Accepted,
            |_, _| {},
        );
        handle_audio_lifecycle_with(
            app_handle.clone(),
            recording_id,
            AudioLifecycleEvent::Recovering {
                reason: AudioCancelReason::RuntimeFailure,
            },
            |_, _| {},
        );
        assert!(
            performance
                .list(10)
                .expect("list runs after recovery")
                .runs
                .is_empty(),
            "runtime recovery must not prematurely persist a cancelled terminal"
        );
        assert_eq!(
            app_handle
                .state::<State>()
                .app_state
                .dictation_telemetry
                .terminal_for_test(recording_id),
            None,
            "recovery alone is not terminal until the typed worker failure arrives"
        );

        handle_audio_lifecycle_with(
            app_handle.clone(),
            recording_id,
            AudioLifecycleEvent::InitializationFailed {
                error: "typed runtime failure".to_string(),
                kind: audio::AudioFailureKind::StreamInvalidated,
                recovery_reason: Some(AudioCancelReason::RuntimeFailure),
            },
            |_, _| {},
        );

        assert_eq!(
            *emitted
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            vec!["audio-recovery-started", "recording-initialization-failed"],
            "the production bridge must preserve recovery-before-failure order"
        );
        let runs = wait_for_completed_runs(&performance, 1);
        assert_eq!(
            runs.len(),
            1,
            "exactly one terminal diagnostic is persisted"
        );
        assert_eq!(
            runs[0].outcome,
            RunOutcomeV1::Failed {
                stage: PerformanceStageV1::CaptureFinalization,
                error_code: StableRunErrorV1::AudioCaptureFailed,
            }
        );
        assert_eq!(
            app_handle
                .state::<State>()
                .app_state
                .dictation_telemetry
                .terminal_for_test(recording_id),
            Some((
                DictationTerminalOutcome::RuntimeInterruption,
                DictationErrorCode::StreamInvalidated,
            )),
            "a later failure callback must not overwrite the first terminal"
        );
    }

    #[test]
    fn idle_after_a_timed_out_stop_clears_processing_for_the_current_recording() {
        let recording_id = 384;
        let app_state = AppState::default();
        app_state.recording_id.store(recording_id, Ordering::SeqCst);
        app_state.dictation.lock_or_recover().status = DictationStatus::Processing;
        keyboard::set_processing(true);

        let app = tauri::test::mock_builder()
            .manage(State {
                app_state,
                benchmark: Arc::new(crate::benchmark::BenchmarkCoordinator::new()),
                microphone_startup_benchmark: crate::commands::microphone_startup_benchmark::MicrophoneStartupBenchmarkState::default(),
                #[cfg(feature = "internal-benchmark")]
                corpus: crate::commands::corpus::CorpusRecorderState::default(),
                meeting_store: crate::meeting_store::MeetingStore::default(),
                meetings: crate::meeting_capture::MeetingCoordinator::default(),
                meeting_suggestions: crate::meeting_suggestions::MeetingSuggestions::default(),
                delivery_recovery: crate::delivery_recovery::DeliveryRecoveryState::default(),
                capture_health: crate::capture_health::CaptureHealthDiagnostics::default(),
                performance: crate::performance_metrics::PerformanceMetrics::default(),

                dictation_diagnostics: crate::dictation_diagnostics::DictationDiagnostics::default(
                ),
                notch_info: std::sync::Mutex::new(None),
                display_snapshot: std::sync::Mutex::new(None),
                overlay_calibration: std::sync::Mutex::new(
                    crate::commands::overlay::OverlayCalibrationState::default(),
                ),
                overlay_frame: std::sync::Mutex::new(None),
                overlay_layout: std::sync::Mutex::new(crate::commands::overlay::OverlayLayout::default()),

            })
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("build mock Tauri app");
        let app_handle = app.handle().clone();
        let emitted_idle = Arc::new(std::sync::atomic::AtomicBool::new(false));
        {
            let emitted_idle = Arc::clone(&emitted_idle);
            app_handle.listen_any("recording-status-changed", move |event| {
                if event.payload() == "\"idle\"" {
                    emitted_idle.store(true, Ordering::SeqCst);
                }
            });
        }

        handle_audio_lifecycle_with(
            app_handle.clone(),
            recording_id,
            AudioLifecycleEvent::Accepted,
            |_, _| {},
        );
        handle_audio_lifecycle_with(
            app_handle.clone(),
            recording_id,
            AudioLifecycleEvent::Idle,
            |_, _| {},
        );

        let state = app_handle.state::<State>();
        assert_eq!(
            state.app_state.dictation.lock_or_recover().status,
            DictationStatus::Idle
        );
        assert!(!keyboard::is_processing());
        assert!(emitted_idle.load(Ordering::SeqCst));
        assert_eq!(
            state
                .app_state
                .dictation_telemetry
                .terminal_for_test(recording_id),
            Some((
                DictationTerminalOutcome::PipelineFailure,
                DictationErrorCode::RuntimeFailure,
            ))
        );
    }

    struct RetryTestBackend {
        responses: VecDeque<String>,
        sample_counts: Vec<usize>,
    }

    impl RetryTestBackend {
        fn new(responses: &[&str]) -> Self {
            Self {
                responses: responses
                    .iter()
                    .map(|response| response.to_string())
                    .collect(),
                sample_counts: Vec::new(),
            }
        }
    }

    impl transcriber::TranscriptionBackend for RetryTestBackend {
        fn name(&self) -> &str {
            "retry-test"
        }
        fn load_model(&mut self, _model_name: &str) -> Result<(), String> {
            Ok(())
        }
        fn is_model_loaded(&self, _model_name: &str) -> bool {
            true
        }
        fn transcribe(
            &mut self,
            samples: &[f32],
            _language: &str,
            _initial_prompt: Option<&str>,
            _smart_punctuation: bool,
        ) -> Result<String, String> {
            self.sample_counts.push(samples.len());
            self.responses
                .pop_front()
                .ok_or_else(|| "unexpected extra transcription attempt".to_string())
                .and_then(|response| {
                    if response == "__test_decode_failure__" {
                        Err("synthetic decode failure".into())
                    } else {
                        Ok(response)
                    }
                })
        }
        fn token_count(&self, _text: &str) -> Option<usize> {
            None
        }
        fn model_exists(&self) -> bool {
            true
        }
        fn models_dir(&self) -> Result<PathBuf, String> {
            Ok(std::env::temp_dir())
        }
        fn reset(&mut self) {}
    }

    #[test]
    fn configuration_log_metadata_never_contains_user_context_values() {
        let options = serde_json::json!({
            "appProfiles": [{
                "bundleId": "com.private.SecretApp",
                "label": "Secret profile",
                "autoPasteOverride": true,
                "cliFormattingOverride": false
            }],
            "customVocabulary": "private-customer-name",
            "vocabularyEntries": [{
                "id": "private-id",
                "written": "PrivateCanonical",
                "aliases": ["private spoken alias"],
                "enabled": true,
                "scope": { "kind": "global" }
            }],
            "outputDir": "/Users/private/CustomerFiles"
        });

        let metadata = ConfigurationLogMetadata::from_options(&options);
        assert_eq!(metadata.option_count, 4);
        assert_eq!(metadata.app_profile_count, 1);
        assert!(metadata.custom_vocabulary_present);
        assert_eq!(metadata.vocabulary_entry_count, 1);
        assert!(metadata.output_directory_present);

        let rendered = format!("{metadata:?}");
        for secret in [
            "com.private.SecretApp",
            "Secret profile",
            "private-customer-name",
            "private-id",
            "PrivateCanonical",
            "private spoken alias",
            "/Users/private/CustomerFiles",
        ] {
            assert!(
                !rendered.contains(secret),
                "telemetry metadata leaked {secret}"
            );
        }
    }

    #[test]
    fn rejected_built_in_phrase_alias_leaves_prior_backend_state_unchanged() {
        let mut dictation = crate::state::DictationState {
            custom_vocabulary: "Tauri".to_string(),
            vocabulary_entries: vec![crate::state::VocabularyEntry {
                id: "tauri".to_string(),
                written: "Tauri".to_string(),
                aliases: vec!["Tori".to_string()],
                enabled: true,
                scope: crate::state::VocabularyScope::Global,
            }],
            ..Default::default()
        };
        let before = dictation.clone();
        let options = serde_json::json!({
            "vocabularyEntries": [{
                "id": "line",
                "written": "LineBreak",
                "aliases": ["new line"],
                "enabled": true,
                "scope": { "kind": "global" }
            }],
        });

        let result = stage_vocabulary_configuration(&options, &dictation)
            .map(|staged| staged.commit(&mut dictation));

        assert!(result
            .unwrap_err()
            .contains("built-in spoken formatting phrase"));
        assert_eq!(dictation.custom_vocabulary, before.custom_vocabulary);
        assert_eq!(dictation.vocabulary_entries, before.vocabulary_entries);
    }

    #[test]
    fn writing_style_parser_accepts_only_stable_explicit_enum_values() {
        let cases = [
            (
                serde_json::json!("conversational"),
                Some(crate::state::WritingStyle::Conversational),
            ),
            (
                serde_json::json!("polished"),
                Some(crate::state::WritingStyle::Polished),
            ),
            (
                serde_json::json!("code_technical"),
                Some(crate::state::WritingStyle::CodeTechnical),
            ),
            (
                serde_json::json!("verbatim"),
                Some(crate::state::WritingStyle::Verbatim),
            ),
            (
                serde_json::json!("notes"),
                Some(crate::state::WritingStyle::Notes),
            ),
            (serde_json::Value::Null, None),
            (serde_json::json!("terminal"), None),
            (serde_json::json!(true), None),
        ];
        for (value, expected) in cases {
            assert_eq!(parse_writing_style(Some(&value)), expected);
        }
        assert_eq!(parse_writing_style(None), None);
    }

    /// A reuse session with two committed pieces over synthetic PCM, and the
    /// final VAD's kept audio for it. Segments are (A, B, C) in a 14 s clip.
    fn two_piece_reuse(complete: bool) -> (crate::stream_reuse::FinalPlan, Vec<f32>) {
        use crate::stream_reuse::{Committed, Plan, StreamReuse};
        use crate::vad_stream::StreamSnapshot;
        const A: (usize, usize) = (8_000, 72_000);
        const B: (usize, usize) = (88_000, 152_000);
        const C: (usize, usize) = (168_000, 184_000);
        let raw: Vec<f32> = (0..16_000 * 14).map(|i| (i as f32 * 0.37).sin()).collect();
        let mut reuse = StreamReuse::default();
        reuse.begin(1);
        let mut texts = [
            "First stable segment.",
            "First stable segment. Next stable segment.",
        ]
        .into_iter();
        for segments in [vec![A], vec![A, B]] {
            let scored = segments.last().unwrap().1 + 9_600;
            let Plan::Commit(plan) = reuse.plan(
                1,
                &StreamSnapshot {
                    segments,
                    scored_samples: scored,
                },
            ) else {
                panic!("expected a piece");
            };
            let piece = plan.piece(&raw[plan.raw_start..]).unwrap();
            assert_eq!(
                reuse.commit(1, plan, piece, texts.next().unwrap()),
                Committed::Stored
            );
        }
        let bounds: Vec<_> = if complete { vec![A, B] } else { vec![A, B, C] };
        let filtered: Vec<f32> = bounds
            .iter()
            .flat_map(|&(start, end)| raw[start..end].iter().copied())
            .collect();
        (
            reuse.take_final(1, Some(&bounds), &filtered).unwrap(),
            filtered,
        )
    }

    #[test]
    fn final_reuse_is_authoritative_or_falls_back_to_the_identical_whole_clip() {
        let settings = dictation_context::TranscriptionSettings {
            model_name: transcriber::COREML_MODEL_NAME.into(),
            language: "auto".into(),
            vad_sensitivity: 50,
            prompt: None,
            smart_punctuation: true,
            streaming_pipeline_enabled: true,
        };
        for tail in [
            "Next stable segment. Tail retained exactly.",
            "Next stable segment, Tail retained exactly.",
            "Next stable segment.",
            "",
            "__test_decode_failure__",
        ] {
            let (plan, filtered) = two_piece_reuse(false);
            let tail_len = filtered.len() - plan.start;
            let mut backend = RetryTestBackend::new(&[tail, "Whole clip authoritative result."]);
            let result = decode_final_transcript(
                &mut backend,
                &settings,
                &filtered,
                &filtered,
                true,
                Ok(Some(plan)),
            )
            .unwrap();
            let outcome = result.reuse.expect("a decision was taken");
            if tail == "Next stable segment. Tail retained exactly." {
                assert_eq!(
                    result.attempt.text,
                    "First stable segment. Next stable segment. Tail retained exactly."
                );
                assert_eq!(backend.sample_counts, [tail_len]);
                assert_eq!(outcome.code, "reused");
                assert_eq!(outcome.decoded_ms, (tail_len / 16) as u64);
            } else {
                assert_eq!(result.attempt.text, "Whole clip authoritative result.");
                assert_eq!(backend.sample_counts, [tail_len, filtered.len()]);
                assert_eq!(
                    outcome.code,
                    if tail == "__test_decode_failure__" {
                        "tail_decode_error"
                    } else {
                        "stitch_mismatch"
                    }
                );
            }
        }
    }

    #[test]
    fn the_stop_reuse_decision_is_silent_when_off_and_reports_why_when_inactive() {
        let state = AppState::default();
        let settings = |model: &str, sensitivity: u32, streaming: bool| {
            dictation_context::TranscriptionSettings {
                model_name: model.into(),
                language: "auto".into(),
                vad_sensitivity: sensitivity,
                prompt: None,
                smart_punctuation: true,
                streaming_pipeline_enabled: streaming,
            }
        };
        let audio = vec![0.0_f32; 16_000];
        let decide = |settings: &dictation_context::TranscriptionSettings| {
            take_stream_reuse_plan(&state, settings, 1, None, &audio)
        };
        // Setting off: no decision at all, so no telemetry and today's path.
        assert!(matches!(
            decide(&settings(transcriber::COREML_MODEL_NAME, 50, false)),
            Ok(None)
        ));
        // Setting on, but no reuse session is possible for this recording.
        for inactive in [
            settings("tiny.en", 50, true),
            settings(transcriber::COREML_MODEL_NAME, 0, true),
        ] {
            assert!(matches!(
                decide(&inactive),
                Err(crate::stream_reuse::Reason::Inactive)
            ));
        }
    }

    #[test]
    fn committed_pieces_covering_all_kept_audio_decode_nothing_at_stop() {
        let settings = dictation_context::TranscriptionSettings {
            model_name: transcriber::COREML_MODEL_NAME.into(),
            language: "auto".into(),
            vad_sensitivity: 50,
            prompt: None,
            smart_punctuation: true,
            streaming_pipeline_enabled: true,
        };
        let (plan, filtered) = two_piece_reuse(true);
        let mut backend = RetryTestBackend::new(&[]);
        let result = decode_final_transcript(
            &mut backend,
            &settings,
            &filtered,
            &filtered,
            true,
            Ok(Some(plan)),
        )
        .unwrap();
        assert_eq!(
            result.attempt.text,
            "First stable segment. Next stable segment."
        );
        assert!(backend.sample_counts.is_empty());
        assert_eq!(result.reuse.unwrap().decoded_ms, 0);
    }

    #[test]
    fn a_refused_reuse_plan_runs_the_unchanged_whole_clip_decode_once() {
        let settings = dictation_context::TranscriptionSettings {
            model_name: transcriber::COREML_MODEL_NAME.into(),
            language: "auto".into(),
            vad_sensitivity: 50,
            prompt: None,
            smart_punctuation: true,
            streaming_pipeline_enabled: true,
        };
        let filtered = vec![0.1; 32_000];
        for reason in [
            crate::stream_reuse::Reason::BoundsMismatch,
            crate::stream_reuse::Reason::Superseded,
            crate::stream_reuse::Reason::VadNotIncremental,
        ] {
            let mut backend = RetryTestBackend::new(&["Whole clip authoritative result."]);
            let result = decode_final_transcript(
                &mut backend,
                &settings,
                &filtered,
                &filtered,
                true,
                Err(reason),
            )
            .unwrap();
            assert_eq!(result.attempt.text, "Whole clip authoritative result.");
            assert_eq!(backend.sample_counts, [32_000]);
            assert_eq!(result.reuse.unwrap().code, reason.as_str());
        }
        // Setting off: no decision is reported at all.
        let mut backend = RetryTestBackend::new(&["Whole clip authoritative result."]);
        let result = decode_final_transcript(
            &mut backend,
            &settings,
            &filtered,
            &filtered,
            true,
            Ok(None),
        )
        .unwrap();
        assert!(result.reuse.is_none());
    }

    #[test]
    fn empty_coreml_result_after_vad_retries_original_audio_once() {
        let filtered = vec![0.0; 8_000];
        let original = vec![0.0; 16_000];
        let mut backend = RetryTestBackend::new(&["", "recovered words"]);
        let attempt = transcribe_with_coreml_vad_retry_report(
            &mut backend,
            transcriber::COREML_MODEL_NAME,
            &filtered,
            &original,
            true,
            "auto",
            None,
            true,
        )
        .unwrap();
        assert_eq!(attempt.text, "recovered words");
        assert_eq!(attempt.empty_error, None);
        assert_eq!(backend.sample_counts, vec![8_000, 16_000]);
    }

    #[test]
    fn still_empty_coreml_retry_is_bounded_to_two_attempts() {
        let filtered = vec![0.0; 8_000];
        let original = vec![0.0; 16_000];
        let mut backend = RetryTestBackend::new(&["", ""]);
        let attempt = transcribe_with_coreml_vad_retry_report(
            &mut backend,
            transcriber::COREML_MODEL_NAME,
            &filtered,
            &original,
            true,
            "auto",
            None,
            true,
        )
        .unwrap();
        assert!(attempt.text.is_empty());
        assert_eq!(
            attempt.empty_error,
            Some(DictationErrorCode::CoremlVadRetryExhausted)
        );
        assert_eq!(backend.sample_counts, vec![8_000, 16_000]);
    }

    #[test]
    fn empty_result_without_coreml_vad_trim_is_not_retried() {
        let samples = vec![0.0; 8_000];
        let mut non_coreml = RetryTestBackend::new(&[""]);
        let attempt = transcribe_with_coreml_vad_retry_report(
            &mut non_coreml,
            "base.en",
            &samples,
            &samples,
            true,
            "en",
            None,
            true,
        )
        .unwrap();
        assert!(attempt.text.is_empty());
        assert_eq!(attempt.empty_error, Some(DictationErrorCode::EmptyOutput));
        assert_eq!(non_coreml.sample_counts, vec![8_000]);

        let mut untrimmed_coreml = RetryTestBackend::new(&[""]);
        let text = transcribe_with_coreml_vad_retry(
            &mut untrimmed_coreml,
            transcriber::COREML_MODEL_NAME,
            &samples,
            &samples,
            false,
            "auto",
            None,
            true,
        )
        .unwrap();
        assert!(text.is_empty());
        assert_eq!(untrimmed_coreml.sample_counts, vec![8_000]);
    }

    #[test]
    fn dedupe_prompt_drops_case_insensitive_repeats() {
        // "Tauri"/"tauri" collapse to the first surface form; order preserved.
        let out = dedupe_prompt_terms("Tauri useEffect tauri useState USEEFFECT");
        assert_eq!(out, "Tauri useEffect useState");
    }

    #[test]
    fn dedupe_prompt_is_noop_when_unique() {
        let out = dedupe_prompt_terms("alpha beta gamma");
        assert_eq!(out, "alpha beta gamma");
    }

    #[test]
    fn combine_prompts_dedupes_across_sources() {
        // Cross-source overlap: the folder/builtin `code` carries "Tauri" and the
        // custom list repeats it (different case). It must appear exactly once,
        // and order keeps the user-typed custom list first (precedence under
        // Whisper's start-keeping prompt truncation), then folder/builtin (code).
        let code = "Tauri serde useEffect";
        let custom = "tauri myProject SERDE";
        let combined = combine_prompts(custom, code).unwrap();
        assert_eq!(combined, "tauri myProject SERDE useEffect");
        // Each shared term appears exactly once (case-insensitively).
        let n_tauri = combined
            .split(' ')
            .filter(|t| t.eq_ignore_ascii_case("tauri"))
            .count();
        assert_eq!(n_tauri, 1, "combined={:?}", combined);
        let n_serde = combined
            .split(' ')
            .filter(|t| t.eq_ignore_ascii_case("serde"))
            .count();
        assert_eq!(n_serde, 1, "combined={:?}", combined);
    }

    #[test]
    fn combine_prompts_handles_single_and_empty_sources() {
        assert_eq!(combine_prompts("", ""), None);
        assert_eq!(
            combine_prompts("custom dupCustom", "").as_deref(),
            Some("custom dupCustom")
        );
        assert_eq!(
            combine_prompts("", "code dupe Dupe").as_deref(),
            Some("code dupe")
        );
    }

    #[test]
    fn cached_code_vocab_prompt_is_empty_outside_explicit_technical_context() {
        let mut dictation = crate::state::DictationState {
            code_vocab_enabled: true,
            code_vocab_folder: "/project".to_string(),
            code_vocab_prompt: Some("all__ toBe".to_string()),
            ..crate::state::DictationState::default()
        };
        let ordinary = crate::state::AppProfile {
            bundle_id: "com.example.Chat".to_string(),
            label: "Chat".to_string(),
            auto_paste_override: None,
            cleanup_override: None,
            cli_formatting_override: None,
            smart_formatting_override: None,
            writing_style: None,
            ide_context_enabled: false,
            ide_project_roots: Vec::new(),
        };
        let mut technical = ordinary.clone();
        technical.bundle_id = "com.example.Editor".to_string();
        technical.writing_style = Some(crate::state::WritingStyle::CodeTechnical);
        dictation.app_profiles = vec![ordinary, technical];

        assert_eq!(
            cached_code_vocab_prompt(&dictation, None).as_deref(),
            Some("")
        );
        assert_eq!(
            cached_code_vocab_prompt(&dictation, Some("com.example.Chat")).as_deref(),
            Some("")
        );
        assert_eq!(
            cached_code_vocab_prompt(&dictation, Some("com.example.Unknown")).as_deref(),
            Some("")
        );
        let technical_prompt =
            cached_code_vocab_prompt(&dictation, Some("com.example.Editor")).unwrap();
        assert!(technical_prompt.starts_with("all__ toBe"));
        assert!(technical_prompt.contains("useEffect"));
    }

    #[test]
    fn budget_constants_decoupled_96_and_500() {
        // The two budgets are intentionally distinct; Whisper's is the smaller.
        assert_eq!(WHISPER_PROMPT_TERMS, 96);
        assert_eq!(CORRECTION_TERMS, 500);
        const { assert!(WHISPER_PROMPT_TERMS < CORRECTION_TERMS) };
    }

    #[test]
    fn whisper_prefix_takes_top_96_of_ranked_prompt() {
        // A ranked prompt longer than the Whisper budget is truncated to its first
        // WHISPER_PROMPT_TERMS terms (which, given descending-frequency ranking,
        // are the top-96). The full string is what correction consumes.
        let full: String = (0..200)
            .map(|i| format!("term{:03}", i))
            .collect::<Vec<_>>()
            .join(" ");
        let prefix = whisper_prefix(&full);
        let words: Vec<&str> = prefix.split_whitespace().collect();
        assert_eq!(
            words.len(),
            WHISPER_PROMPT_TERMS,
            "Whisper budget caps at 96"
        );
        // Prefix is the leading slice of the full ranked list.
        assert_eq!(words[0], "term000");
        assert_eq!(
            words[WHISPER_PROMPT_TERMS - 1],
            format!("term{:03}", WHISPER_PROMPT_TERMS - 1)
        );
        // Shorter-than-budget prompt passes through unchanged.
        assert_eq!(whisper_prefix("alpha beta gamma"), "alpha beta gamma");
        assert_eq!(whisper_prefix(""), "");
    }

    #[test]
    fn summary_serializes_ranked_terms_and_whisper_count_camel_case() {
        // rankedTerms / whisperCount must serialize camelCase per the frontend
        // contract; each ranked entry is { term, freq }.
        let summary = VocabScanSummary {
            files: 3,
            skipped: 1,
            terms: 2,
            bytes: 42,
            capped: false,
            ms: 7,
            sample_terms: vec!["fooBar".into()],
            ranked_terms: vec![
                RankedTermJson {
                    term: "fooBar".into(),
                    freq: 5,
                },
                RankedTermJson {
                    term: "barBaz".into(),
                    freq: 2,
                },
            ],
            whisper_count: 2,
            adopted: true,
        };
        let v = serde_json::to_value(&summary).unwrap();
        // New camelCase keys present; snake_case absent.
        assert!(v.get("rankedTerms").is_some(), "rankedTerms missing: {v}");
        assert!(v.get("whisperCount").is_some(), "whisperCount missing: {v}");
        assert!(v.get("ranked_terms").is_none(), "snake_case leaked: {v}");
        assert!(v.get("whisper_count").is_none(), "snake_case leaked: {v}");
        assert_eq!(v["whisperCount"], 2);
        assert_eq!(v["adopted"], true);
        // sampleTerms (existing field) stays camelCase too.
        assert!(v.get("sampleTerms").is_some(), "sampleTerms missing: {v}");
        // Each ranked entry carries term + freq.
        let first = &v["rankedTerms"][0];
        assert_eq!(first["term"], "fooBar");
        assert_eq!(first["freq"], 5);
    }

    #[test]
    fn progress_serializes_scan_id_camel_case() {
        let progress = VocabScanProgress {
            scan_id: "scan-42".into(),
            current_path: "/project/src/main.rs".into(),
            files_read: 1,
            dirs_skipped: 0,
            terms_so_far: 3,
            done: false,
            adopted: false,
        };
        let value = serde_json::to_value(progress).unwrap();
        assert_eq!(value["scanId"], "scan-42");
        assert!(value.get("scan_id").is_none());
    }

    #[test]
    fn newer_scan_prevents_older_result_adoption() {
        let app_state = AppState::default();
        begin_code_vocab_scan(&app_state, "scan-a", "/project");
        begin_code_vocab_scan(&app_state, "scan-b", "/project");

        assert!(!complete_code_vocab_scan(
            &app_state,
            "scan-a",
            "/project",
            "staleTerm".into(),
        ));
        assert!(complete_code_vocab_scan(
            &app_state,
            "scan-b",
            "/project",
            "currentTerm".into(),
        ));

        let dictation = app_state.dictation.lock_or_recover();
        assert_eq!(dictation.code_vocab_prompt.as_deref(), Some("currentTerm"));
        assert!(dictation.code_vocab_scan_id.is_none());
    }

    #[test]
    fn cancellation_only_invalidates_the_matching_scan() {
        let app_state = AppState::default();
        begin_code_vocab_scan(&app_state, "scan-a", "/project");

        assert!(!cancel_code_vocab_scan_id(&app_state, "scan-b"));
        assert_eq!(
            app_state
                .dictation
                .lock_or_recover()
                .code_vocab_scan_id
                .as_deref(),
            Some("scan-a"),
        );
        assert!(cancel_code_vocab_scan_id(&app_state, "scan-a"));
        assert!(!complete_code_vocab_scan(
            &app_state,
            "scan-a",
            "/project",
            "canceledTerm".into(),
        ));
        assert!(app_state
            .dictation
            .lock_or_recover()
            .code_vocab_prompt
            .is_none());
    }

    #[test]
    fn settings_changes_during_scan_report_non_adoption() {
        for change in ["disable", "folder"] {
            let app_state = AppState::default();
            begin_code_vocab_scan(&app_state, "scan-a", "/project-a");
            {
                let mut dictation = app_state.dictation.lock_or_recover();
                if change == "disable" {
                    dictation.code_vocab_enabled = false;
                } else {
                    dictation.code_vocab_folder = "/project-b".into();
                }
                dictation.code_vocab_prompt = None;
                dictation.code_vocab_scan_id = None;
            }

            assert!(
                !complete_code_vocab_scan(&app_state, "scan-a", "/project-a", "staleTerm".into(),),
                "{change} must supersede the in-flight scan",
            );
            let dictation = app_state.dictation.lock_or_recover();
            assert!(dictation.code_vocab_prompt.is_none());
        }
    }

    #[test]
    fn idle_guard_resets_status_on_drop() {
        let app_state = AppState::default();
        let rid = app_state.next_recording_id();
        {
            let mut dictation = app_state.dictation.lock().unwrap();
            dictation.status = DictationStatus::Processing;
        }
        {
            let _guard = IdleGuard::new(&app_state, rid);
            // guard drops here
        }
        let dictation = app_state.dictation.lock().unwrap();
        assert_eq!(dictation.status, DictationStatus::Idle);
    }

    #[test]
    fn idle_guard_finishes_before_notification_and_ignores_superseded_recordings() {
        let app_state = AppState::default();
        let rid = app_state.next_recording_id();
        app_state.dictation.lock_or_recover().status = DictationStatus::Processing;
        let mut guard = IdleGuard::new(&app_state, rid);
        let mut notified = false;
        guard.finish(|dictation| {
            assert_eq!(dictation.status, DictationStatus::Idle);
            notified = true;
        });
        assert!(notified);
        app_state.next_recording_id();
        app_state.dictation.lock_or_recover().status = DictationStatus::Recording;
        drop(guard);
        assert_eq!(
            app_state.dictation.lock_or_recover().status,
            DictationStatus::Recording
        );

        let mut stale = IdleGuard::new(&app_state, rid);
        stale.finish(|_| panic!("superseded recording must not publish idle"));
        assert_eq!(
            app_state.dictation.lock_or_recover().status,
            DictationStatus::Recording
        );
    }

    #[test]
    fn paused_smart_preference_preserves_per_app_auto_paste() {
        let app_state = AppState::default();
        {
            let mut dictation = app_state.dictation.lock_or_recover();
            dictation.smart_destination_enabled = true;
            dictation.auto_paste = false;
            dictation.app_profiles = vec![crate::state::AppProfile {
                bundle_id: "com.example.Editor".into(),
                label: "Editor".into(),
                auto_paste_override: Some(true),
                cleanup_override: None,
                cli_formatting_override: None,
                smart_formatting_override: None,
                writing_style: None,
                ide_context_enabled: false,
                ide_project_roots: vec![],
            }];
            assert!(!smart_destination_eligible(&dictation));
            dictation.auto_paste = true;
            assert!(smart_destination_eligible(&dictation));
            dictation.save_audio = true;
            assert!(!smart_destination_eligible(&dictation));
            dictation.save_audio = false;
            dictation.save_transcript = true;
            assert!(!smart_destination_eligible(&dictation));
            dictation.save_transcript = false;
            dictation.auto_paste = false;
        }
        let context = resolve_live_context(
            &app_state,
            &crate::frontmost::FrontmostAppIdentity {
                bundle_id: Some("com.example.Editor".into()),
                process_id: Some(42),
            },
            &crate::frontmost::DeliveryTargetSnapshot::Incomplete,
        );
        assert!(context.delivery.auto_paste);
        assert!(matches!(
            context.app.smart_destination,
            crate::smart_destination::Snapshot::Disabled
        ));
    }

    #[test]
    fn smart_prepare_fallback_keeps_the_anchored_stop_target() {
        use crate::frontmost::{
            select_delivery_anchor, AppTransitionSnapshot, DeliveryAnchor, DeliveryTargetIdentity,
            DeliveryTargetSnapshot, ProcessInstanceToken,
        };
        use crate::smart_destination::Snapshot;

        let target = |bundle_id: &str, process_id: i32, instance: u64| {
            DeliveryTargetSnapshot::Complete(DeliveryTargetIdentity {
                bundle_id: bundle_id.into(),
                process_id,
                process_instance: ProcessInstanceToken::for_test(instance),
                window_token: None,
                transitions: AppTransitionSnapshot {
                    activation_generation: 0,
                    space_generation: 0,
                },
            })
        };
        // Production 0.51.12: dictation started with a field focused in the
        // start app, the user moved to another app while speaking and stopped
        // there. The same-app fallback re-bound the paste to the start app and
        // refused it as different_application.
        let start_app = target("com.example.Start", 41, 1);
        let stop_app = target("com.example.Stop", 42, 2);
        let current_app_capture = Snapshot::Target {
            token: 7,
            revision: 1,
            identity: start_app.clone(),
            current_app: true,
        };
        let (anchored, anchor) = select_delivery_anchor(&start_app, Some(&stop_app));
        assert_eq!(anchor, DeliveryAnchor::Stop);

        let (route, delivery_target) = smart_prepare_fallback(&current_app_capture, true, anchored);
        assert!(matches!(route, Snapshot::Unverified));
        assert!(
            delivery_target == stop_app,
            "fallback must keep the stop anchor"
        );

        // Without a complete stop sample the anchor is still the start app.
        let (anchored, anchor) =
            select_delivery_anchor(&start_app, Some(&DeliveryTargetSnapshot::SelfTarget));
        assert_eq!(anchor, DeliveryAnchor::Start);
        let (route, delivery_target) = smart_prepare_fallback(&current_app_capture, true, anchored);
        assert!(matches!(route, Snapshot::Unverified));
        assert!(delivery_target == start_app);

        // A clipboard-only handoff refusal, a lost preparation reply, and a
        // media handoff stay closed.
        let (route, _) = smart_prepare_fallback(&current_app_capture, false, &stop_app);
        assert!(matches!(route, Snapshot::Unavailable));
        let media_capture = Snapshot::Target {
            token: 8,
            revision: 1,
            identity: start_app.clone(),
            current_app: false,
        };
        let (route, _) = smart_prepare_fallback(&media_capture, true, &stop_app);
        assert!(matches!(route, Snapshot::Unavailable));
    }

    #[test]
    fn smart_destination_resolves_and_freezes_the_intended_targets_profile() {
        use crate::frontmost::{
            AppTransitionSnapshot, DeliveryTargetIdentity, DeliveryTargetSnapshot,
            ProcessInstanceToken,
        };
        use crate::state::AppProfile;

        let target = |bundle_id: &str, process_id: i32, instance: u64| {
            DeliveryTargetSnapshot::Complete(DeliveryTargetIdentity {
                bundle_id: bundle_id.into(),
                process_id,
                process_instance: ProcessInstanceToken::for_test(instance),
                window_token: None,
                transitions: AppTransitionSnapshot {
                    activation_generation: 0,
                    space_generation: 0,
                },
            })
        };
        let profile = |bundle_id: &str, label: &str| AppProfile {
            bundle_id: bundle_id.into(),
            label: label.into(),
            auto_paste_override: None,
            cleanup_override: None,
            cli_formatting_override: None,
            smart_formatting_override: None,
            writing_style: None,
            ide_context_enabled: false,
            ide_project_roots: Vec::new(),
        };
        let media_target = target("com.brave.Browser", 41, 1);
        let editor_target = target("com.example.Editor", 42, 2);
        let app_state = AppState::default();
        app_state.dictation.lock_or_recover().app_profiles = vec![
            profile("com.brave.Browser", "Media browser"),
            profile("com.example.Editor", "Intended editor"),
        ];

        let smart = crate::smart_destination::Snapshot::Target {
            token: 17,
            revision: 4,
            identity: editor_target.clone(),
            current_app: false,
        };
        assert_eq!(smart_destination_status(&smart, &media_target), "recent");
        for accepted in [media_target.clone(), DeliveryTargetSnapshot::Incomplete] {
            assert_eq!(
                smart_destination_status(
                    &crate::smart_destination::Snapshot::Unverified,
                    &accepted
                ),
                "current",
                "an unproven field still uses normal auto-paste into the current app",
            );
        }
        assert_eq!(
            smart_destination_status(
                &crate::smart_destination::Snapshot::Unavailable,
                &media_target
            ),
            "unavailable",
        );
        let destination = select_recording_destination(&media_target, smart.clone());
        assert!(destination.target == editor_target);
        let context = resolve_recording_context(&app_state, destination);
        assert_eq!(context.app.bundle_id.as_deref(), Some("com.example.Editor"));
        assert_eq!(
            context
                .matched_profile
                .as_ref()
                .map(|profile| profile.label.as_str()),
            Some("Intended editor"),
        );
        assert!(context.app.delivery_target == editor_target);
        assert!(matches!(
            &context.app.smart_destination,
            crate::smart_destination::Snapshot::Target { .. }
        ));

        let disabled = select_recording_destination(
            &media_target,
            crate::smart_destination::Snapshot::Disabled,
        );
        assert!(disabled.target == media_target);
        let context = resolve_recording_context(&app_state, disabled);
        assert_eq!(
            context
                .matched_profile
                .as_ref()
                .map(|profile| profile.label.as_str()),
            Some("Media browser"),
            "the opt-out path must preserve the existing frontmost-app profile",
        );
        assert!(matches!(
            &context.app.smart_destination,
            crate::smart_destination::Snapshot::Disabled
        ));
    }

    #[test]
    fn live_context_scopes_prompt_and_matcher_to_the_apps_explicit_profile() {
        use crate::state::{AppProfile, VocabularyEntry, VocabularyScope, WritingStyle};
        let app_state = AppState::default();
        let editor = "com.example.Editor";
        let chat = "com.example.Chat";
        let profile = |bundle_id: &str, style: Option<WritingStyle>, project: bool| AppProfile {
            bundle_id: bundle_id.into(),
            label: bundle_id.into(),
            auto_paste_override: None,
            cleanup_override: None,
            cli_formatting_override: None,
            smart_formatting_override: None,
            writing_style: style,
            ide_context_enabled: project,
            ide_project_roots: if project {
                vec!["/project".into()]
            } else {
                Vec::new()
            },
        };
        {
            let mut dictation = app_state.dictation.lock_or_recover();
            dictation.code_vocab_enabled = true;
            dictation.code_vocab_folder = "/project".into();
            dictation.code_vocab_prompt = Some("all__ toBe".into());
            dictation.app_profiles = vec![
                profile(editor, Some(WritingStyle::CodeTechnical), true),
                profile(chat, Some(WritingStyle::Conversational), false),
            ];
            dictation.vocabulary_entries = vec![VocabularyEntry {
                id: "project-term".into(),
                written: "ProjectWord".into(),
                aliases: vec!["project alias".into()],
                enabled: true,
                scope: VocabularyScope::Project {
                    bundle_id: editor.into(),
                    root: "/project".into(),
                },
            }];
        }
        rebuild_correction_matcher(&app_state, &app_state.dictation.lock_or_recover());
        for bundle in [editor, chat] {
            let context = resolve_live_context(
                &app_state,
                &crate::frontmost::FrontmostAppIdentity {
                    bundle_id: Some(bundle.into()),
                    process_id: Some(1),
                },
                &crate::frontmost::DeliveryTargetSnapshot::Incomplete,
            );
            let prompt = context.transcription.prompt.as_deref().unwrap_or("");
            let matcher = context.transformations.correction_matcher.as_ref().unwrap();
            let project = bundle == editor;
            assert_eq!(
                context.context_capture.local_project_index, project,
                "{bundle}"
            );
            assert_eq!(prompt.contains("ProjectWord"), project, "{bundle}");
            assert_eq!(prompt.contains("all__"), project, "{bundle}");
            assert_eq!(
                matcher.apply("project alias"),
                if project {
                    "ProjectWord"
                } else {
                    "project alias"
                },
                "{bundle}"
            );
        }
    }

    #[test]
    fn idle_guard_disarm_prevents_reset() {
        let app_state = AppState::default();
        let rid = app_state.next_recording_id();
        {
            let mut dictation = app_state.dictation.lock().unwrap();
            dictation.status = DictationStatus::Processing;
        }
        {
            let mut guard = IdleGuard::new(&app_state, rid);
            guard.disarm();
            // guard drops here, but disarmed -- no reset
        }
        let dictation = app_state.dictation.lock().unwrap();
        assert_eq!(dictation.status, DictationStatus::Processing);
    }

    #[test]
    fn idle_guard_calls_set_processing_false() {
        keyboard::set_processing(true);
        let app_state = AppState::default();
        let rid = app_state.next_recording_id();
        {
            let _guard = IdleGuard::new(&app_state, rid);
        }
        assert!(!keyboard::is_processing());
    }

    #[test]
    fn file_transcribe_guard_clears_flag_on_drop() {
        use std::sync::atomic::Ordering;
        let app_state = AppState::default();
        app_state.file_transcribing.store(true, Ordering::SeqCst);
        {
            let _guard = FileTranscribeGuard {
                app_state: &app_state,
                app_handle: None,
            };
            // guard drops here
        }
        assert!(!app_state.file_transcribing.load(Ordering::SeqCst));
    }

    #[test]
    fn shared_backend_change_guard_releases_coordinator_on_drop() {
        let coordinator = Arc::new(crate::benchmark::BenchmarkCoordinator::new());
        assert!(coordinator.try_start_shared_backend_change());
        {
            let _guard = SharedBackendChangeGuard(coordinator.clone(), None);
            assert!(!coordinator.try_start());
        }
        assert!(coordinator.try_start());
    }

    #[test]
    fn idle_guard_skips_reset_when_recording_superseded() {
        let app_state = AppState::default();
        let rid1 = app_state.next_recording_id(); // 1
        {
            let mut dictation = app_state.dictation.lock().unwrap();
            dictation.status = DictationStatus::Recording;
        }
        // Simulate new recording starting (increments rid)
        let _rid2 = app_state.next_recording_id(); // 2
        {
            let _guard = IdleGuard::new(&app_state, rid1);
            // guard drops here, but rid1 != current_rid(2), so no reset
        }
        let dictation = app_state.dictation.lock().unwrap();
        assert_eq!(dictation.status, DictationStatus::Recording);
    }

    #[test]
    fn generation_counter_cancel_current_recording() {
        let app_state = AppState::default();
        let id = app_state.next_recording_id(); // 1
        assert!(!app_state.is_cancelled(id));
        app_state.cancel_recording(id);
        assert!(app_state.is_cancelled(id));
    }

    #[test]
    fn generation_counter_new_recording_not_cancelled() {
        let app_state = AppState::default();
        let id1 = app_state.next_recording_id(); // 1
        app_state.cancel_recording(id1);
        let id2 = app_state.next_recording_id(); // 2
        assert!(app_state.is_cancelled(id1));
        assert!(!app_state.is_cancelled(id2));
    }

    #[test]
    fn generation_counter_monotonic_ids() {
        let app_state = AppState::default();
        let id1 = app_state.next_recording_id();
        let id2 = app_state.next_recording_id();
        let id3 = app_state.next_recording_id();
        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
        assert_eq!(id3, 3);
    }

    #[test]
    fn build_code_vocab_prompt_empty_or_blank_folder_is_empty() {
        assert_eq!(build_code_vocab_prompt(""), "");
        assert_eq!(build_code_vocab_prompt("   "), "");
        assert_eq!(build_code_vocab_prompt("\t\n"), "");
    }

    #[test]
    fn build_code_vocab_prompt_scans_source_files_in_folder() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file_path = dir.path().join("widget.rs");
        std::fs::write(
            &file_path,
            "fn renderWidgetSurface() { let widgetSurfaceHandle = 1; widgetSurfaceHandle; }",
        )
        .expect("write fixture source file");

        let prompt = build_code_vocab_prompt(dir.path().to_str().expect("utf8 path"));

        assert!(
            prompt.to_lowercase().contains("widgetsurface")
                || prompt.contains("renderWidgetSurface")
                || prompt.contains("widgetSurfaceHandle"),
            "expected scanned identifiers in prompt, got: {prompt:?}"
        );
    }

    #[test]
    fn parse_vocab_terms_splits_on_commas_and_newlines_and_drops_blanks() {
        assert_eq!(
            parse_vocab_terms("API Gateway, useEffect\n\nkubectl\r\n , "),
            vec![
                "API Gateway".to_string(),
                "useEffect".to_string(),
                "kubectl".to_string(),
            ]
        );
        assert!(parse_vocab_terms("").is_empty());
        assert!(parse_vocab_terms("   ,\n\r,  ").is_empty());
    }

    #[test]
    fn parse_vocab_terms_preserves_internal_spaces() {
        assert_eq!(
            parse_vocab_terms("multi word term"),
            vec!["multi word term".to_string()]
        );
    }

    #[test]
    fn legacy_vocabulary_entries_assigns_stable_ids_and_defaults() {
        let entries = legacy_vocabulary_entries("foo, bar\nbaz");
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].id, "legacy-0");
        assert_eq!(entries[0].written, "foo");
        assert_eq!(entries[1].id, "legacy-1");
        assert_eq!(entries[1].written, "bar");
        assert_eq!(entries[2].id, "legacy-2");
        assert_eq!(entries[2].written, "baz");
        for entry in &entries {
            assert!(entry.aliases.is_empty());
            assert!(entry.enabled);
            assert_eq!(entry.scope, crate::state::VocabularyScope::Global);
        }
    }

    #[test]
    fn legacy_vocabulary_entries_empty_input_yields_no_entries() {
        assert!(legacy_vocabulary_entries("").is_empty());
        assert!(legacy_vocabulary_entries("   \n\r  ").is_empty());
    }

    fn stage_report(
        stage: &'static str,
        outcome: crate::transcript_transform::StageOutcome,
        duration_us: u64,
    ) -> crate::transcript_transform::StageReport {
        crate::transcript_transform::StageReport {
            stage,
            duration_us,
            changed: false,
            outcome,
            failure_policy: crate::transcript_transform::StageFailurePolicy::OptionalFallback,
        }
    }

    #[test]
    fn transcript_stage_timing_maps_known_stage_and_outcome() {
        use crate::transcript_transform::{StageOutcome, CLEANUP_STAGE};

        let timing =
            transcript_stage_timing(&stage_report(CLEANUP_STAGE, StageOutcome::Applied, 4_000))
                .expect("cleanup stage recognized");
        assert_eq!(timing.stage, PerformanceStageV1::Cleanup);
        assert_eq!(timing.outcome, StageOutcomeV1::Completed);
        assert_eq!(
            timing.duration_ms,
            crate::performance_metrics::MeasurementV1::measured(4)
        );
    }

    #[test]
    fn homophone_stage_timing_preserves_fallback_zero_and_disabled_semantics() {
        use crate::performance_metrics::MeasurementV1;
        use crate::transcript_transform::{StageOutcome, HOMOPHONE_CONTEXT_STAGE};
        for (outcome, micros, expected_duration, expected_outcome) in [
            (
                StageOutcome::Fallback,
                150_900,
                MeasurementV1::measured(150),
                StageOutcomeV1::Fallback,
            ),
            (
                StageOutcome::Fallback,
                30,
                MeasurementV1::measured(0),
                StageOutcomeV1::Fallback,
            ),
            (
                StageOutcome::Applied,
                90_000,
                MeasurementV1::measured(90),
                StageOutcomeV1::Completed,
            ),
            (
                StageOutcome::Skipped,
                0,
                MeasurementV1::NotApplicable,
                StageOutcomeV1::Skipped,
            ),
        ] {
            let timing =
                transcript_stage_timing(&stage_report(HOMOPHONE_CONTEXT_STAGE, outcome, micros))
                    .unwrap();
            assert_eq!(timing.stage, PerformanceStageV1::HomophoneContext);
            assert_eq!(timing.duration_ms, expected_duration);
            assert_eq!(timing.outcome, expected_outcome);
        }
    }

    #[test]
    fn transcript_stage_timing_skipped_outcome_reports_not_applicable() {
        use crate::transcript_transform::{StageOutcome, SPOKEN_STRUCTURE_STAGE};

        let timing = transcript_stage_timing(&stage_report(
            SPOKEN_STRUCTURE_STAGE,
            StageOutcome::Skipped,
            999,
        ))
        .expect("spoken structure stage recognized");
        assert_eq!(timing.outcome, StageOutcomeV1::Skipped);
        assert_eq!(
            timing.duration_ms,
            crate::performance_metrics::MeasurementV1::NotApplicable
        );
    }

    #[test]
    fn transcript_stage_timing_unknown_stage_returns_none() {
        use crate::transcript_transform::StageOutcome;

        assert!(transcript_stage_timing(&stage_report(
            "not-a-real-stage",
            StageOutcome::Applied,
            1
        ))
        .is_none());
        // The retired custom Voice Commands stage is no longer emitted or mapped.
        assert!(
            transcript_stage_timing(&stage_report("voice_commands", StageOutcome::Applied, 1))
                .is_none()
        );
    }

    #[test]
    fn release_to_paste_requires_confirmed_delivery_of_a_successful_run() {
        use crate::hotkey_release::HotkeyRelease;
        use std::time::{Duration, Instant};

        let released_at = Instant::now();
        let release = HotkeyRelease::for_test(released_at, released_at + Duration::from_millis(12));
        let delivered_at = released_at + Duration::from_millis(180);
        let stage =
            release_to_paste_stage(&RunOutcomeV1::Success, true, Some(release), delivered_at)
                .expect("confirmed delivery has a release-to-paste total");
        assert_eq!(stage.stage, PerformanceStageV1::ReleaseToPaste);
        assert_eq!(
            stage.duration_ms,
            crate::performance_metrics::MeasurementV1::measured(180)
        );

        for outcome in [
            DictationDeliveryOutcome::ClipboardOnly,
            DictationDeliveryOutcome::AutoPastePosted,
        ] {
            assert!(delivery_confirms_paste(outcome));
        }
        // Clipboard-write failure, injection timeout, and stopped delivery are
        // all reported as these two outcomes; neither enters the cohort.
        for outcome in [
            DictationDeliveryOutcome::ClipboardWriteFailed,
            DictationDeliveryOutcome::Unconfirmed,
        ] {
            assert!(!delivery_confirms_paste(outcome));
            assert_eq!(
                release_to_paste_stage(
                    &RunOutcomeV1::Success,
                    delivery_confirms_paste(outcome),
                    Some(release),
                    delivered_at,
                ),
                None
            );
        }
        assert_eq!(
            release_to_paste_stage(&RunOutcomeV1::NoSpeech, true, Some(release), delivered_at),
            None
        );
        assert_eq!(
            release_to_paste_stage(&RunOutcomeV1::Success, true, None, delivered_at),
            None
        );
        assert!(!PipelineTimings::default().delivery_confirmed);
    }

    #[test]
    fn pipeline_stages_separates_final_queue_from_preparation_and_skipped_inference() {
        let timings = PipelineTimings {
            model_queue_ms: 64,
            decode_ms: 97,
            warm_state: Some(ModelWarmStateV1::Warm),
            ..PipelineTimings::default()
        };
        let stages = pipeline_stages(&timings, 161);
        assert_eq!(
            stages
                .iter()
                .find(|stage| stage.stage == PerformanceStageV1::InferenceQueue)
                .unwrap()
                .duration_ms,
            crate::performance_metrics::MeasurementV1::measured(64),
        );
        assert!(!stages
            .iter()
            .any(|stage| stage.stage == PerformanceStageV1::ModelQueue));
        let skipped = pipeline_stages(&PipelineTimings::default(), 0);
        assert_eq!(
            skipped
                .iter()
                .find(|stage| stage.stage == PerformanceStageV1::InferenceQueue)
                .unwrap()
                .duration_ms,
            crate::performance_metrics::MeasurementV1::NotApplicable,
        );
    }

    #[test]
    fn pipeline_stages_marks_zero_file_output_as_not_applicable() {
        let timings = PipelineTimings {
            vad_ms: 10,
            model_queue_ms: 20,
            model_load_ms: 30,
            decode_ms: 40,
            transform_ms: 50,
            file_output_ms: 0,
            paste_ms: 60,
            ..PipelineTimings::default()
        };
        let stages = pipeline_stages(&timings, 500);
        let file_output = stages
            .iter()
            .find(|stage| stage.stage == PerformanceStageV1::FileOutput)
            .expect("file output stage present");
        assert_eq!(
            file_output.duration_ms,
            crate::performance_metrics::MeasurementV1::NotApplicable
        );
    }

    #[test]
    fn pipeline_stages_measures_nonzero_file_output_and_includes_transform_stages() {
        let extra = StageTimingV1::measured(PerformanceStageV1::Cleanup, 7);
        let timings = PipelineTimings {
            file_output_ms: 15,
            transform_stages: vec![extra.clone()],
            ..PipelineTimings::default()
        };
        let stages = pipeline_stages(&timings, 100);
        let file_output = stages
            .iter()
            .find(|stage| stage.stage == PerformanceStageV1::FileOutput)
            .expect("file output stage present");
        assert_eq!(
            file_output.duration_ms,
            crate::performance_metrics::MeasurementV1::measured(15)
        );
        assert!(stages.contains(&extra));
        assert_eq!(
            stages
                .iter()
                .find(|stage| stage.stage == PerformanceStageV1::TotalProcessing)
                .expect("total processing stage present")
                .duration_ms,
            crate::performance_metrics::MeasurementV1::measured(100)
        );
    }
}

// This developer-only model test is never compiled into the running app.
#[cfg(all(test, target_os = "macos", target_arch = "aarch64"))]
#[path = "recording_determinism_test.rs"]
mod offline_determinism;
