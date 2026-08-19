use crate::llm_sidecar::{CancelToken, TransformError};
use crate::meeting_artifact::{
    chunk_segments, merge_artifacts, parse_artifact, render_chunk_input, SUMMARY_INSTRUCTION,
};
use crate::meeting_store::MeetingSessionStatus;
use crate::{MutexExt, State};
use serde::Serialize;
use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeetingSummaryPhase {
    #[default]
    Idle,
    Running,
    Cancelling,
    Complete,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingSummaryStatus {
    pub generation: u64,
    pub session_id: Option<String>,
    pub phase: MeetingSummaryPhase,
    pub completed_chunks: u32,
    pub total_chunks: u32,
    pub elapsed_ms: u64,
    pub peak_rss_mb: u64,
    pub error_code: Option<String>,
}

struct SummaryInner {
    status: MeetingSummaryStatus,
    cancel: Option<CancelToken>,
}

#[derive(Clone)]
pub struct MeetingSummaryCoordinator {
    inner: Arc<Mutex<SummaryInner>>,
}

impl Default for MeetingSummaryCoordinator {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(SummaryInner {
                status: MeetingSummaryStatus::default(),
                cancel: None,
            })),
        }
    }
}

impl MeetingSummaryCoordinator {
    pub fn status(&self) -> MeetingSummaryStatus {
        self.inner.lock_or_recover().status.clone()
    }

    fn publish(&self, app: &tauri::AppHandle) {
        let _ = app.emit("meeting-summary-status-changed", self.status());
    }

    fn update(&self, app: &tauri::AppHandle, update: impl FnOnce(&mut MeetingSummaryStatus)) {
        update(&mut self.inner.lock_or_recover().status);
        self.publish(app);
    }
}

fn stable_error(error: TransformError) -> &'static str {
    match error {
        TransformError::Cancelled => "cancelled",
        TransformError::NotDownloaded => "model_not_downloaded",
        TransformError::Busy | TransformError::HeavyRuntimeActive => "runtime_busy",
        TransformError::Timeout => "timeout",
        TransformError::InvalidRequest | TransformError::OutputInvalid => "invalid_output",
        _ => "generation_failed",
    }
}

struct SummaryOwnershipGuard {
    app: tauri::AppHandle,
}

impl Drop for SummaryOwnershipGuard {
    fn drop(&mut self) {
        self.app
            .state::<State>()
            .app_state
            .meeting_summary_active
            .store(false, Ordering::SeqCst);
    }
}

async fn run_summary(app: tauri::AppHandle, session_id: String, generation: u64) {
    let _ownership = SummaryOwnershipGuard { app: app.clone() };
    let started = Instant::now();
    let state = app.state::<State>();
    let repository = match state.meeting_store.repository() {
        Ok(repository) => repository,
        Err(_) => {
            state.meeting_summaries.update(&app, |status| {
                status.phase = MeetingSummaryPhase::Failed;
                status.error_code = Some("store_unavailable".into());
            });
            return;
        }
    };
    let detail = match repository.detail(&session_id) {
        Ok(detail) => detail,
        Err(_) => {
            state.meeting_summaries.update(&app, |status| {
                status.phase = MeetingSummaryPhase::Failed;
                status.error_code = Some("meeting_unavailable".into());
            });
            return;
        }
    };
    let chunks = chunk_segments(&detail.segments);
    let total_chunks = chunks.len().min(u32::MAX as usize) as u32;
    state.meeting_summaries.update(&app, |status| {
        status.total_chunks = total_chunks;
    });
    let cancel = state
        .meeting_summaries
        .inner
        .lock_or_recover()
        .cancel
        .clone()
        .unwrap_or_default();
    let mut artifacts = Vec::with_capacity(chunks.len());
    let mut peak_rss_mb = 0;
    for chunk in chunks {
        if cancel.is_cancelled() {
            state.meeting_summaries.update(&app, |status| {
                status.phase = MeetingSummaryPhase::Cancelled;
                status.error_code = None;
            });
            return;
        }
        let allowed = chunk
            .iter()
            .map(|segment| segment.id)
            .collect::<HashSet<_>>();
        let input = render_chunk_input(&chunk);
        let result = state
            .transform_runtime
            .transform(
                SUMMARY_INSTRUCTION,
                &input,
                Duration::from_secs(30),
                cancel.clone(),
            )
            .await;
        peak_rss_mb = peak_rss_mb.max(state.transform_runtime.resident_rss_mb());
        let artifact = match result {
            Ok(output) => parse_artifact(&output.output, &allowed),
            Err(error) => {
                let code = stable_error(error);
                state.meeting_summaries.update(&app, |status| {
                    status.phase = if code == "cancelled" {
                        MeetingSummaryPhase::Cancelled
                    } else {
                        MeetingSummaryPhase::Failed
                    };
                    status.error_code = (code != "cancelled").then(|| code.to_string());
                    status.elapsed_ms = started.elapsed().as_millis() as u64;
                    status.peak_rss_mb = peak_rss_mb;
                });
                return;
            }
        };
        let Some(artifact) = artifact else {
            state.meeting_summaries.update(&app, |status| {
                status.phase = MeetingSummaryPhase::Failed;
                status.error_code = Some("invalid_output".into());
                status.elapsed_ms = started.elapsed().as_millis() as u64;
                status.peak_rss_mb = peak_rss_mb;
            });
            return;
        };
        artifacts.push(artifact);
        state.meeting_summaries.update(&app, |status| {
            status.completed_chunks = status.completed_chunks.saturating_add(1);
            status.elapsed_ms = started.elapsed().as_millis() as u64;
            status.peak_rss_mb = peak_rss_mb;
        });
    }
    let Some(artifact) = merge_artifacts(artifacts) else {
        state.meeting_summaries.update(&app, |status| {
            status.phase = MeetingSummaryPhase::Failed;
            status.error_code = Some("no_transcript".into());
        });
        return;
    };
    let runtime_ms = started.elapsed().as_millis().min(u64::MAX as u128) as u64;
    if repository
        .save_artifact(&session_id, &artifact, runtime_ms, peak_rss_mb)
        .is_err()
    {
        state.meeting_summaries.update(&app, |status| {
            status.phase = MeetingSummaryPhase::Failed;
            status.error_code = Some("store_unavailable".into());
        });
        return;
    }
    state.meeting_summaries.update(&app, |status| {
        if status.generation == generation {
            status.phase = MeetingSummaryPhase::Complete;
            status.elapsed_ms = runtime_ms;
            status.peak_rss_mb = peak_rss_mb;
            status.error_code = None;
        }
    });
    tracing::info!(target: "meeting", generation, runtime_ms, peak_rss_mb, total_chunks, "meeting summary completed");
}

#[tauri::command]
pub async fn start_meeting_summary(
    app: tauri::AppHandle,
    session_id: String,
    state: tauri::State<'_, State>,
) -> Result<MeetingSummaryStatus, String> {
    let session_id = session_id.trim().to_string();
    let _transition = state.app_state.recording_transition.lock().await;
    if state.app_state.meeting_blocks_asr()
        || state.benchmark.is_running()
        || state.app_state.file_transcribing.load(Ordering::SeqCst)
        || state.transform_runtime.is_transform_busy()
        || state.app_state.dictation.lock_or_recover().status != crate::state::DictationStatus::Idle
    {
        return Err("Finish the active recording, meeting, benchmark, or transform first.".into());
    }
    let detail = state.meeting_store.repository()?.detail(&session_id)?;
    if !matches!(
        detail.session.status,
        MeetingSessionStatus::Complete | MeetingSessionStatus::Interrupted
    ) || detail
        .segments
        .iter()
        .all(|segment| segment.text.trim().is_empty())
    {
        return Err("This meeting does not have a completed transcript to summarize.".into());
    }
    state
        .app_state
        .meeting_summary_active
        .store(true, Ordering::SeqCst);
    state.transform_runtime.shutdown();
    let mut inner = state.meeting_summaries.inner.lock_or_recover();
    let generation = inner.status.generation.saturating_add(1);
    let cancel = CancelToken::new();
    inner.cancel = Some(cancel);
    inner.status = MeetingSummaryStatus {
        generation,
        session_id: Some(session_id.clone()),
        phase: MeetingSummaryPhase::Running,
        ..MeetingSummaryStatus::default()
    };
    let status = inner.status.clone();
    drop(inner);
    state.meeting_summaries.publish(&app);
    tauri::async_runtime::spawn(run_summary(app, session_id, generation));
    Ok(status)
}

#[tauri::command]
pub fn get_meeting_summary_status(state: tauri::State<'_, State>) -> MeetingSummaryStatus {
    state.meeting_summaries.status()
}

#[tauri::command]
pub fn cancel_meeting_summary(app: tauri::AppHandle, state: tauri::State<'_, State>) -> bool {
    let mut inner = state.meeting_summaries.inner.lock_or_recover();
    if inner.status.phase != MeetingSummaryPhase::Running {
        return false;
    }
    inner.status.phase = MeetingSummaryPhase::Cancelling;
    if let Some(cancel) = inner.cancel.as_ref() {
        cancel.cancel();
    }
    drop(inner);
    state.transform_runtime.cancel_inflight_request();
    state.meeting_summaries.publish(&app);
    true
}
