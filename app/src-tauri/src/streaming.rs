use crate::state::DictationStatus;
use crate::{audio, partial_transcript, transcriber, vad, MutexExt, State};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tauri::Manager;

use transcriber::chunking::{
    reconcile_overlapping_text, OVERLAP_SAMPLES, STEP_SAMPLES, WINDOW_SAMPLES,
};
const POLL_INTERVAL: Duration = Duration::from_millis(200);

#[derive(Clone)]
pub(crate) struct StreamingConfig {
    pub recording_id: u64,
    pub model_name: String,
    pub language: String,
    pub prompt: Option<String>,
    pub smart_punctuation: bool,
    pub vad_threshold: f32,
}

#[derive(Default)]
struct StreamingProgress {
    text: String,
    processed_end: usize,
    chunk_count: u32,
    vad_ms: u64,
    inference_ms: u64,
    start_to_first_partial_ms: Option<u64>,
    partial_update_count: u32,
    last_partial_at: Option<Instant>,
    reliable: bool,
    fallback_reason: Option<String>,
}

pub(crate) struct StreamingSession {
    recording_id: u64,
    config: StreamingConfig,
    stop_tx: tokio::sync::watch::Sender<bool>,
    join: tokio::task::JoinHandle<StreamingProgress>,
}

pub(crate) struct IncrementalTranscript {
    pub text: String,
    pub chunk_count: u32,
    pub vad_ms: u64,
    pub streaming_inference_ms: u64,
    pub final_chunk_ms: u64,
    pub rss_before_mb: u64,
    pub rss_after_mb: u64,
}

#[derive(Default)]
pub(crate) struct IncrementalMetrics {
    pub attempted: bool,
    pub completed: bool,
    pub fell_back: bool,
    pub start_to_first_partial_ms: Option<u64>,
    pub partial_update_count: u32,
    pub last_partial_at: Option<Instant>,
}

#[derive(Default)]
pub(crate) struct IncrementalFinalization {
    pub transcript: Option<IncrementalTranscript>,
    pub metrics: IncrementalMetrics,
}

#[derive(Default)]
struct ChunkOutput {
    text: String,
    vad_ms: u64,
    inference_ms: u64,
    rss_before_mb: u64,
    rss_after_mb: u64,
}

/// Start one sequential chunk worker for a Whisper recording. Other backends
/// deliberately retain the existing batch path.
pub(crate) async fn start_session(app_handle: tauri::AppHandle, config: StreamingConfig) {
    if !transcriber::is_whisper_model(&config.model_name) {
        return;
    }

    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let recording_id = config.recording_id;
    let recording_started_at = Instant::now();
    let task_config = config.clone();
    let join = tokio::spawn(run_session(
        app_handle.clone(),
        task_config,
        stop_rx,
        recording_started_at,
    ));
    let session = StreamingSession {
        recording_id,
        config,
        stop_tx,
        join,
    };

    let state = app_handle.state::<State>();
    let mut slot = state.app_state.streaming_session.lock().await;
    if let Some(previous) = slot.take() {
        let _ = previous.stop_tx.send(true);
        previous.join.abort();
        tracing::warn!(target: "pipeline", "incremental transcription: replaced stale session");
    }
    *slot = Some(session);
    tracing::info!(
        target: "pipeline",
        recording_id,
        window_ms = WINDOW_SAMPLES / 16,
        step_ms = STEP_SAMPLES / 16,
        overlap_ms = OVERLAP_SAMPLES / 16,
        "incremental transcription started"
    );
}

async fn run_session(
    app_handle: tauri::AppHandle,
    config: StreamingConfig,
    mut stop_rx: tokio::sync::watch::Receiver<bool>,
    recording_started_at: Instant,
) -> StreamingProgress {
    let mut progress = StreamingProgress {
        reliable: true,
        ..StreamingProgress::default()
    };
    let mut next_end = WINDOW_SAMPLES;

    loop {
        if let Ok(changed) = tokio::time::timeout(POLL_INTERVAL, stop_rx.changed()).await {
            if changed.is_err() || *stop_rx.borrow() {
                break;
            }
        }

        if !session_is_current(&app_handle, config.recording_id) {
            progress.reliable = false;
            progress.fallback_reason = Some("recording session was superseded".to_string());
            break;
        }

        let Some(available) = audio::recording_sample_count_16k() else {
            continue;
        };
        if available < next_end {
            continue;
        }

        // Never build a backlog. If capture advanced by more than one complete
        // step while the worker was busy/suspended, abandon incremental output;
        // the authoritative batch fallback will run after stop.
        if worker_fell_behind(available, next_end) {
            progress.reliable = false;
            progress.fallback_reason = Some(format!(
                "worker fell behind (available={available}, next_end={next_end})"
            ));
            break;
        }

        let window_start = next_end - WINDOW_SAMPLES;
        let Some(window) = audio::snapshot_recording_window_16k(window_start, next_end) else {
            continue;
        };
        match transcribe_window(app_handle.clone(), config.clone(), window).await {
            Ok(chunk) => {
                let reconciled = reconcile_overlapping_text(&progress.text, &chunk.text);
                if !progress.text.is_empty()
                    && !chunk.text.is_empty()
                    && reconciled.overlap_words == 0
                {
                    progress.reliable = false;
                    progress.fallback_reason = Some(format!(
                        "chunk {} had no deterministic overlap",
                        progress.chunk_count + 1
                    ));
                    break;
                }
                // Inference can outlive cancellation, a model change, or a
                // newer recording. Revalidate after reconciliation and before
                // adopting or publishing any provisional text.
                if !session_config_is_current(&app_handle, &config) {
                    progress.reliable = false;
                    progress.fallback_reason =
                        Some("recording session was superseded after inference".to_string());
                    break;
                }
                let previous_text = std::mem::replace(&mut progress.text, reconciled.text);
                progress.processed_end = next_end;
                progress.chunk_count += 1;
                progress.vad_ms = progress.vad_ms.saturating_add(chunk.vad_ms);
                progress.inference_ms = progress.inference_ms.saturating_add(chunk.inference_ms);
                if should_publish_partial(&previous_text, &progress.text) {
                    let now = Instant::now();
                    progress
                        .start_to_first_partial_ms
                        .get_or_insert_with(|| recording_started_at.elapsed().as_millis() as u64);
                    progress.partial_update_count += 1;
                    progress.last_partial_at = Some(now);
                    partial_transcript::emit_update(
                        &app_handle,
                        config.recording_id,
                        progress.text.clone(),
                        progress.chunk_count,
                        (next_end / 16) as u64,
                    );
                }
                tracing::info!(
                    target: "pipeline",
                    recording_id = config.recording_id,
                    chunk_index = progress.chunk_count,
                    window_start,
                    window_end = next_end,
                    vad_ms = chunk.vad_ms,
                    inference_ms = chunk.inference_ms,
                    "incremental chunk complete"
                );
                next_end = next_end.saturating_add(STEP_SAMPLES);
            }
            Err(error) => {
                progress.reliable = false;
                progress.fallback_reason = Some(error);
                break;
            }
        }
    }

    if !progress.reliable {
        partial_transcript::emit_clear(
            &app_handle,
            config.recording_id,
            partial_transcript::PartialTranscriptClearReason::Fallback,
        );
        tracing::warn!(
            target: "pipeline",
            recording_id = config.recording_id,
            reason = progress.fallback_reason.as_deref().unwrap_or("unknown"),
            start_to_first_partial_ms = progress.start_to_first_partial_ms.unwrap_or(0),
            had_partial = progress.start_to_first_partial_ms.is_some(),
            partial_update_count = progress.partial_update_count,
            incremental_completed = false,
            incremental_fell_back = true,
            "incremental transcription abandoned; batch fallback required"
        );
    }
    progress
}

fn session_is_current(app_handle: &tauri::AppHandle, recording_id: u64) -> bool {
    let state = app_handle.state::<State>();
    let current_id = state.app_state.recording_id.load(Ordering::SeqCst);
    let cancelled_id = state.app_state.cancelled_id.load(Ordering::SeqCst);
    let dictation = state.app_state.dictation.lock_or_recover();
    session_generation_is_current(current_id, cancelled_id, recording_id, dictation.status)
}

fn session_config_is_current(app_handle: &tauri::AppHandle, config: &StreamingConfig) -> bool {
    let state = app_handle.state::<State>();
    let current_id = state.app_state.recording_id.load(Ordering::SeqCst);
    let cancelled_id = state.app_state.cancelled_id.load(Ordering::SeqCst);
    let dictation = state.app_state.dictation.lock_or_recover();
    session_generation_is_current(
        current_id,
        cancelled_id,
        config.recording_id,
        dictation.status,
    ) && dictation.model_name == config.model_name
}

fn session_generation_is_current(
    current_id: u64,
    cancelled_id: u64,
    recording_id: u64,
    status: DictationStatus,
) -> bool {
    current_id == recording_id
        && cancelled_id < recording_id
        && matches!(
            status,
            DictationStatus::Recording | DictationStatus::Processing
        )
}

fn worker_fell_behind(available: usize, next_end: usize) -> bool {
    available >= next_end.saturating_add(STEP_SAMPLES)
}

fn should_publish_partial(previous: &str, cumulative: &str) -> bool {
    !cumulative.trim().is_empty() && previous != cumulative
}

async fn transcribe_window(
    app_handle: tauri::AppHandle,
    config: StreamingConfig,
    samples: Vec<f32>,
) -> Result<ChunkOutput, String> {
    tokio::task::spawn_blocking(move || {
        if samples.is_empty() {
            return Ok(ChunkOutput::default());
        }
        let vad_path = vad::vad_model_path()
            .filter(|path| path.exists())
            .ok_or_else(|| "VAD model unavailable".to_string())?;
        let vad_started = Instant::now();
        let filtered = match vad::filter_speech(
            &vad_path.to_string_lossy(),
            &samples,
            config.vad_threshold,
        )? {
            vad::VadResult::NoSpeech => {
                return Ok(ChunkOutput {
                    vad_ms: vad_started.elapsed().as_millis() as u64,
                    ..ChunkOutput::default()
                });
            }
            vad::VadResult::Speech(samples) => samples,
        };
        let vad_ms = vad_started.elapsed().as_millis() as u64;

        let state = app_handle.state::<State>();
        if state.app_state.recording_id.load(Ordering::SeqCst) != config.recording_id
            || state.app_state.is_cancelled(config.recording_id)
        {
            return Err("recording session was superseded before inference".to_string());
        }
        {
            let dictation = state.app_state.dictation.lock_or_recover();
            if dictation.model_name != config.model_name {
                return Err("model changed during recording".to_string());
            }
        }

        let rss_before_mb = crate::resource_monitor::get_process_rss_mb();
        let inference_started = Instant::now();
        let text = {
            let mut backend = state.app_state.backend.lock_or_recover();
            if backend.name() != "whisper" {
                return Err("active backend changed during recording".to_string());
            }
            backend.load_model(&config.model_name)?;
            backend.transcribe(
                &filtered,
                &config.language,
                config.prompt.as_deref(),
                config.smart_punctuation,
            )?
        };
        let inference_ms = inference_started.elapsed().as_millis() as u64;
        let rss_after_mb = crate::resource_monitor::get_process_rss_mb();
        Ok(ChunkOutput {
            text,
            vad_ms,
            inference_ms,
            rss_before_mb,
            rss_after_mb,
        })
    })
    .await
    .map_err(|error| format!("incremental worker panicked: {error}"))?
}

/// Signal the worker before audio teardown so it cannot schedule another
/// window. The returned handle is finalized after the immutable full buffer is
/// available.
pub(crate) async fn begin_finish(
    app_state: &crate::state::AppState,
    recording_id: u64,
) -> Option<StreamingSession> {
    let mut slot = app_state.streaming_session.lock().await;
    let session = slot.take()?;
    if session.recording_id != recording_id {
        let _ = session.stop_tx.send(true);
        session.join.abort();
        return None;
    }
    let _ = session.stop_tx.send(true);
    Some(session)
}

pub(crate) async fn cancel(app_state: &crate::state::AppState, recording_id: u64) {
    let mut slot = app_state.streaming_session.lock().await;
    if let Some(session) = slot.take() {
        let _ = session.stop_tx.send(true);
        session.join.abort();
        tracing::info!(target: "pipeline", recording_id, "incremental transcription cancelled");
    }
}

pub(crate) fn discard(session: Option<StreamingSession>) {
    if let Some(session) = session {
        let _ = session.stop_tx.send(true);
        session.join.abort();
    }
}

/// Reconcile the completed during-recording prefix with one bounded final tail.
/// Returning `None` requests the unchanged full-buffer pipeline.
pub(crate) async fn finalize(
    app_handle: tauri::AppHandle,
    session: Option<StreamingSession>,
    full_samples: &[f32],
) -> IncrementalFinalization {
    let Some(session) = session else {
        return IncrementalFinalization::default();
    };
    let config = session.config.clone();
    let progress = match session.join.await {
        Ok(progress) => progress,
        Err(error) => {
            tracing::warn!(target: "pipeline", recording_id = config.recording_id, error = %error, "incremental join failed; using batch fallback");
            partial_transcript::emit_clear(
                &app_handle,
                config.recording_id,
                partial_transcript::PartialTranscriptClearReason::Fallback,
            );
            return IncrementalFinalization {
                metrics: IncrementalMetrics {
                    attempted: true,
                    fell_back: true,
                    ..IncrementalMetrics::default()
                },
                ..IncrementalFinalization::default()
            };
        }
    };
    let mut metrics = IncrementalMetrics {
        attempted: true,
        start_to_first_partial_ms: progress.start_to_first_partial_ms,
        partial_update_count: progress.partial_update_count,
        last_partial_at: progress.last_partial_at,
        ..IncrementalMetrics::default()
    };
    if !progress.reliable || progress.processed_end > full_samples.len() {
        metrics.fell_back = true;
        partial_transcript::emit_clear(
            &app_handle,
            config.recording_id,
            partial_transcript::PartialTranscriptClearReason::Fallback,
        );
        return IncrementalFinalization {
            transcript: None,
            metrics,
        };
    }
    if progress.chunk_count == 0 {
        return IncrementalFinalization {
            transcript: None,
            metrics,
        };
    }

    let tail_start = progress.processed_end.saturating_sub(OVERLAP_SAMPLES);
    let tail = full_samples[tail_start..].to_vec();
    let final_started = Instant::now();
    let final_chunk = match transcribe_window(app_handle.clone(), config.clone(), tail).await {
        Ok(chunk) => chunk,
        Err(error) => {
            tracing::warn!(target: "pipeline", recording_id = config.recording_id, error, "incremental final chunk failed; using batch fallback");
            metrics.fell_back = true;
            partial_transcript::emit_clear(
                &app_handle,
                config.recording_id,
                partial_transcript::PartialTranscriptClearReason::Fallback,
            );
            return IncrementalFinalization {
                transcript: None,
                metrics,
            };
        }
    };
    let final_chunk_ms = final_started.elapsed().as_millis() as u64;
    let reconciled = reconcile_overlapping_text(&progress.text, &final_chunk.text);
    if !progress.text.is_empty() && !final_chunk.text.is_empty() && reconciled.overlap_words == 0 {
        tracing::warn!(target: "pipeline", recording_id = config.recording_id, "incremental final chunk had no deterministic overlap; using batch fallback");
        metrics.fell_back = true;
        partial_transcript::emit_clear(
            &app_handle,
            config.recording_id,
            partial_transcript::PartialTranscriptClearReason::Fallback,
        );
        return IncrementalFinalization {
            transcript: None,
            metrics,
        };
    }
    let text = reconciled.text;
    if text.trim().is_empty() {
        metrics.fell_back = true;
        partial_transcript::emit_clear(
            &app_handle,
            config.recording_id,
            partial_transcript::PartialTranscriptClearReason::Fallback,
        );
        return IncrementalFinalization {
            transcript: None,
            metrics,
        };
    }

    metrics.completed = true;

    tracing::info!(
        target: "pipeline",
        recording_id = config.recording_id,
        chunks = progress.chunk_count + 1,
        processed_during_recording_samples = progress.processed_end,
        final_tail_samples = full_samples.len().saturating_sub(tail_start),
        streaming_inference_ms = progress.inference_ms,
        final_chunk_ms,
        "incremental transcription finalized"
    );
    IncrementalFinalization {
        transcript: Some(IncrementalTranscript {
            text,
            chunk_count: progress.chunk_count + 1,
            vad_ms: progress.vad_ms.saturating_add(final_chunk.vad_ms),
            streaming_inference_ms: progress.inference_ms,
            final_chunk_ms,
            rss_before_mb: final_chunk.rss_before_mb,
            rss_after_mb: final_chunk.rss_after_mb,
        }),
        metrics,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        session_generation_is_current, should_publish_partial, worker_fell_behind, STEP_SAMPLES,
    };
    use crate::state::DictationStatus;

    #[test]
    fn stale_or_cancelled_generation_cannot_be_current() {
        assert!(session_generation_is_current(
            8,
            7,
            8,
            DictationStatus::Recording
        ));
        assert!(!session_generation_is_current(
            9,
            7,
            8,
            DictationStatus::Recording
        ));
        assert!(!session_generation_is_current(
            8,
            8,
            8,
            DictationStatus::Recording
        ));
        assert!(!session_generation_is_current(
            8,
            7,
            8,
            DictationStatus::Idle
        ));
    }

    #[test]
    fn worker_never_queues_more_than_one_step() {
        let next_end = 160_000;
        assert!(!worker_fell_behind(next_end + STEP_SAMPLES - 1, next_end));
        assert!(worker_fell_behind(next_end + STEP_SAMPLES, next_end));
    }

    #[test]
    fn publishes_only_new_non_empty_cumulative_text() {
        assert!(should_publish_partial("", "first reliable words"));
        assert!(should_publish_partial(
            "first reliable words",
            "first reliable words then more"
        ));
        assert!(!should_publish_partial("same words", "same words"));
        assert!(!should_publish_partial("same words", "  "));
    }
}
