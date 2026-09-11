use crate::llm_sidecar::{CancelToken, TransformError};
use crate::meeting_artifact::{
    chunk_segments, merge_artifacts, parse_artifact, render_chunk_input, SUMMARY_INSTRUCTION,
};
use crate::meeting_store::MeetingSessionStatus;
use crate::{MutexExt, State};
use serde::Serialize;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};

const SUMMARY_CHUNK_TIMEOUT: Duration = Duration::from_secs(120);

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

    /// Applies `update` only if `status.generation` still equals
    /// `generation`, then publishes when it does. Every mutation on the
    /// `run_summary` async path must go through this so a stale continuation
    /// from a superseded run can never clobber a newer run's status. Returns
    /// whether the update was applied.
    fn update_if_current(
        &self,
        app: &tauri::AppHandle,
        generation: u64,
        update: impl FnOnce(&mut MeetingSummaryStatus),
    ) -> bool {
        let applied = apply_if_current(&self.inner, generation, update);
        if applied {
            self.publish(app);
        }
        applied
    }
}

/// Pure helper behind [`MeetingSummaryCoordinator::update_if_current`]: locks
/// `inner` and applies `update` only if the stored status generation still
/// matches `generation`. Kept as a free function (no `AppHandle`) so it is
/// directly unit-testable.
fn apply_if_current(
    inner: &Mutex<SummaryInner>,
    generation: u64,
    update: impl FnOnce(&mut MeetingSummaryStatus),
) -> bool {
    let mut guard = inner.lock_or_recover();
    if guard.status.generation != generation {
        return false;
    }
    update(&mut guard.status);
    true
}

/// Release the summary ownership flag only when it still belongs to
/// `generation`. The generation comparison and flag mutation share the
/// coordinator lock, which is also held when a new generation claims
/// ownership.
fn release_if_current(
    inner: &Mutex<SummaryInner>,
    active: &AtomicBool,
    generation: u64,
) -> Result<(), u64> {
    let guard = inner.lock_or_recover();
    let current_generation = guard.status.generation;
    if current_generation != generation {
        return Err(current_generation);
    }
    active.store(false, Ordering::SeqCst);
    Ok(())
}

fn stable_error(error: TransformError) -> &'static str {
    match error {
        TransformError::Cancelled => "cancelled",
        TransformError::NotDownloaded => "model_not_downloaded",
        TransformError::Busy | TransformError::HeavyRuntimeActive => "runtime_busy",
        TransformError::Timeout => "timeout",
        TransformError::InvalidRequest => "invalid_request",
        TransformError::OutputInvalid => "invalid_output",
        _ => "generation_failed",
    }
}

struct SummaryOwnershipGuard {
    app: tauri::AppHandle,
    generation: u64,
}

impl Drop for SummaryOwnershipGuard {
    fn drop(&mut self) {
        let state = self.app.state::<State>();
        if let Err(current_generation) = release_if_current(
            &state.meeting_summaries.inner,
            &state.app_state.meeting_summary_active,
            self.generation,
        ) {
            tracing::warn!(
                target: "meeting",
                guard_generation = self.generation,
                current_generation,
                "meeting summary ownership guard dropped for a superseded generation"
            );
        } else {
            crate::smart_auto_probe::wake();
            crate::meeting_suggestions::busy_changed(&self.app);
        }
    }
}

async fn run_summary(app: tauri::AppHandle, session_id: String, generation: u64) {
    let _ownership = SummaryOwnershipGuard {
        app: app.clone(),
        generation,
    };
    let started = Instant::now();
    let state = app.state::<State>();
    let repository = match state.meeting_store.repository() {
        Ok(repository) => repository,
        Err(_) => {
            state
                .meeting_summaries
                .update_if_current(&app, generation, |status| {
                    status.phase = MeetingSummaryPhase::Failed;
                    status.error_code = Some("store_unavailable".into());
                });
            return;
        }
    };
    let detail = match repository.detail(&session_id) {
        Ok(detail) => detail,
        Err(_) => {
            state
                .meeting_summaries
                .update_if_current(&app, generation, |status| {
                    status.phase = MeetingSummaryPhase::Failed;
                    status.error_code = Some("meeting_unavailable".into());
                });
            return;
        }
    };
    let chunks = chunk_segments(&detail.segments);
    let total_chunks = chunks.len().min(u32::MAX as usize) as u32;
    state
        .meeting_summaries
        .update_if_current(&app, generation, |status| {
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
            state
                .meeting_summaries
                .update_if_current(&app, generation, |status| {
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
            .transform_bounded(
                SUMMARY_INSTRUCTION,
                &input,
                SUMMARY_CHUNK_TIMEOUT,
                cancel.clone(),
                256,
            )
            .await;
        peak_rss_mb = peak_rss_mb.max(state.transform_runtime.resident_rss_mb());
        let artifact = match result {
            Ok(output) => parse_artifact(&output.output, &allowed),
            Err(error) => {
                let code = stable_error(error);
                state
                    .meeting_summaries
                    .update_if_current(&app, generation, |status| {
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
            state
                .meeting_summaries
                .update_if_current(&app, generation, |status| {
                    status.phase = MeetingSummaryPhase::Failed;
                    status.error_code = Some("artifact_invalid".into());
                    status.elapsed_ms = started.elapsed().as_millis() as u64;
                    status.peak_rss_mb = peak_rss_mb;
                });
            return;
        };
        artifacts.push(artifact);
        state
            .meeting_summaries
            .update_if_current(&app, generation, |status| {
                status.completed_chunks = status.completed_chunks.saturating_add(1);
                status.elapsed_ms = started.elapsed().as_millis() as u64;
                status.peak_rss_mb = peak_rss_mb;
            });
    }
    let Some(artifact) = merge_artifacts(artifacts) else {
        state
            .meeting_summaries
            .update_if_current(&app, generation, |status| {
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
        state
            .meeting_summaries
            .update_if_current(&app, generation, |status| {
                status.phase = MeetingSummaryPhase::Failed;
                status.error_code = Some("store_unavailable".into());
            });
        return;
    }
    state
        .meeting_summaries
        .update_if_current(&app, generation, |status| {
            status.phase = MeetingSummaryPhase::Complete;
            status.elapsed_ms = runtime_ms;
            status.peak_rss_mb = peak_rss_mb;
            status.error_code = None;
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
    let _transition =
        crate::commands::microphone_preview::transition_after_stopping_preview(&app, state.inner())
            .await?;
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
    let mut inner = state.meeting_summaries.inner.lock_or_recover();
    state
        .app_state
        .meeting_summary_active
        .store(true, Ordering::SeqCst);
    state.transform_runtime.shutdown();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn status_with_generation(generation: u64) -> MeetingSummaryStatus {
        MeetingSummaryStatus {
            generation,
            ..MeetingSummaryStatus::default()
        }
    }

    #[test]
    fn apply_if_current_applies_when_generation_matches() {
        let inner = Mutex::new(SummaryInner {
            status: status_with_generation(3),
            cancel: None,
        });
        let applied = apply_if_current(&inner, 3, |status| status.completed_chunks = 5);
        assert!(applied);
        assert_eq!(inner.lock_or_recover().status.completed_chunks, 5);
    }

    #[test]
    fn apply_if_current_rejects_a_stale_generation() {
        let inner = Mutex::new(SummaryInner {
            status: status_with_generation(3),
            cancel: None,
        });
        let applied = apply_if_current(&inner, 2, |status| status.completed_chunks = 5);
        assert!(!applied);
        assert_eq!(inner.lock_or_recover().status.completed_chunks, 0);
    }

    #[test]
    fn a_stale_generation_cannot_overwrite_a_newer_runs_status() {
        // Models a slow `run_summary` continuation from generation 1 landing
        // after a newer run (generation 2) has already superseded it and
        // recorded its own progress. The stale write must be dropped rather
        // than clobbering the current run's status.
        let inner = Mutex::new(SummaryInner {
            status: status_with_generation(1),
            cancel: None,
        });

        // Generation 2 supersedes generation 1 and records progress.
        {
            let mut guard = inner.lock_or_recover();
            guard.status.generation = 2;
            guard.status.completed_chunks = 1;
            guard.status.phase = MeetingSummaryPhase::Running;
        }

        // Generation 1's stale continuation tries to mark itself Complete.
        let applied = apply_if_current(&inner, 1, |status| {
            status.phase = MeetingSummaryPhase::Complete;
            status.completed_chunks = 99;
            status.error_code = None;
        });

        assert!(!applied, "a stale generation's update must not be applied");
        let status = inner.lock_or_recover().status.clone();
        assert_eq!(status.generation, 2);
        assert_eq!(status.completed_chunks, 1);
        assert_eq!(status.phase, MeetingSummaryPhase::Running);
    }

    #[test]
    fn apply_if_current_matches_the_current_generation_after_it_moves_on() {
        // The mirror case: generation 2's own updates must still land once
        // it becomes current.
        let inner = Mutex::new(SummaryInner {
            status: status_with_generation(2),
            cancel: None,
        });
        let applied = apply_if_current(&inner, 2, |status| {
            status.phase = MeetingSummaryPhase::Complete;
        });
        assert!(applied);
        assert_eq!(
            inner.lock_or_recover().status.phase,
            MeetingSummaryPhase::Complete
        );
    }

    #[test]
    fn stale_generation_cannot_release_newer_ownership() {
        let inner = Mutex::new(SummaryInner {
            status: status_with_generation(2),
            cancel: None,
        });
        let active = AtomicBool::new(true);

        let released = release_if_current(&inner, &active, 1);

        assert_eq!(released, Err(2));
        assert!(
            active.load(Ordering::SeqCst),
            "stale cleanup must leave the newer run's ownership active"
        );
    }

    #[test]
    fn current_generation_releases_its_ownership() {
        let inner = Mutex::new(SummaryInner {
            status: status_with_generation(2),
            cancel: None,
        });
        let active = AtomicBool::new(true);

        let released = release_if_current(&inner, &active, 2);

        assert_eq!(released, Ok(()));
        assert!(
            !active.load(Ordering::SeqCst),
            "current cleanup must release its ownership"
        );
    }
}
