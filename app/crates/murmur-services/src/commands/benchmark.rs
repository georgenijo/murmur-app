use crate::benchmark::{
    self, BenchmarkCoordinator, BenchmarkModel, BenchmarkReport, BenchmarkRequest,
};
use crate::state::DictationStatus;
use crate::{MutexExt, State};
use serde::Serialize;
use std::sync::atomic::Ordering;
use std::sync::Arc;

struct BenchmarkRunGuard(Arc<BenchmarkCoordinator>, crate::host::Context);

impl Drop for BenchmarkRunGuard {
    fn drop(&mut self) {
        self.0.finish();
        crate::meeting_suggestions::busy_changed(&self.1);
    }
}

pub fn get_benchmark_models() -> Vec<BenchmarkModel> {
    benchmark::benchmark_models()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkActivity {
    benchmark_running: bool,
    file_transcribing: bool,
}

pub fn get_benchmark_activity(state: crate::host::StateRef<'_, State>) -> BenchmarkActivity {
    BenchmarkActivity {
        benchmark_running: state.benchmark.is_running(),
        file_transcribing: state.app_state.file_transcribing.load(Ordering::SeqCst),
    }
}

pub async fn run_benchmark(
    app_handle: crate::host::Context,
    state: crate::host::StateRef<'_, State>,
    request: BenchmarkRequest,
) -> Result<BenchmarkReport, String> {
    let coordinator = state.benchmark.clone();
    // Serialize the benchmark claim with every live microphone owner. This
    // closes the check/claim race in both directions with preview startup.
    let transition = crate::commands::microphone_preview::transition_after_stopping_preview(
        &app_handle,
        state.inner(),
    )
    .await?;
    if state.app_state.meeting_blocks_asr() {
        return Err("Wait for the meeting transcript to finish before benchmarking".to_string());
    }
    // Auto-dismiss a parked transform review, refuse on an active transform
    // (issue #338 — this path previously ignored the transform status
    // entirely and would run a benchmark right over a parked review). The
    // in-lock guard below stays as a race guard.

    {
        let dictation = state.app_state.dictation.lock_or_recover();

        if dictation.status != DictationStatus::Idle {
            return Err("Stop recording before running a benchmark".to_string());
        }
        if state.app_state.file_transcribing.load(Ordering::SeqCst) {
            return Err("Wait for the file transcription to finish".to_string());
        }
        #[cfg(feature = "internal-benchmark")]
        if state.corpus.is_active() {
            return Err("Finish the corpus recording before benchmarking".to_string());
        }
        if !coordinator.try_start() {
            return Err(if coordinator.is_running() {
                "A benchmark is already running".to_string()
            } else {
                "Wait for model preparation to finish before benchmarking".to_string()
            });
        }
    }
    drop(transition);
    crate::meeting_suggestions::busy_changed(&app_handle);
    let guard = BenchmarkRunGuard(coordinator.clone(), app_handle.clone());
    super::models::ensure_vad_model(&app_handle)
        .await
        .map_err(|error| format!("Could not prepare speech filtering: {error}"))?;

    tokio::task::spawn_blocking(move || {
        let _guard = guard;
        benchmark::run(&app_handle, &coordinator, request)
    })
    .await
    .map_err(|error| format!("Benchmark task failed: {error}"))?
}

pub fn cancel_benchmark(state: crate::host::StateRef<'_, State>) -> bool {
    let running = state.benchmark.is_running();
    if running {
        state.benchmark.cancel();
    }
    running
}

/// Write a benchmark report (already serialized on the frontend) to the
/// configured output folder under `file_name`. Returns the absolute path.
pub fn save_benchmark_report(
    report_json: String,
    output_dir: String,
    file_name: String,
) -> Result<String, String> {
    crate::file_output::write_benchmark_report(&output_dir, &file_name, &report_json)
        .map(|path| path.to_string_lossy().into_owned())
}

/// Open the benchmark output folder (creating it if needed) in the system file
/// manager, so the user can see and share saved reports.
pub fn open_benchmark_output_folder(
    app_handle: crate::host::Context,
    output_dir: String,
) -> Result<(), String> {
    let dir = crate::file_output::resolve_output_dir(&output_dir)?;

    app_handle.open_path(dir).map_err(String::from)
}
