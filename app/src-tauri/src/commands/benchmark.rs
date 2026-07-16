use crate::benchmark::{
    self, BenchmarkCoordinator, BenchmarkModel, BenchmarkReport, BenchmarkRequest,
};
use crate::state::DictationStatus;
use crate::{MutexExt, State};
use std::sync::atomic::Ordering;
use std::sync::Arc;

struct BenchmarkRunGuard(Arc<BenchmarkCoordinator>);

impl Drop for BenchmarkRunGuard {
    fn drop(&mut self) {
        self.0.finish();
    }
}

#[tauri::command]
pub fn get_benchmark_models() -> Vec<BenchmarkModel> {
    benchmark::benchmark_models()
}

#[tauri::command]
pub async fn run_benchmark(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, State>,
    request: BenchmarkRequest,
) -> Result<BenchmarkReport, String> {
    let coordinator = state.benchmark.clone();
    let vad_threshold = {
        let dictation = state.app_state.dictation.lock_or_recover();
        if dictation.status != DictationStatus::Idle {
            return Err("Stop recording before running a benchmark".to_string());
        }
        if state.app_state.file_transcribing.load(Ordering::SeqCst) {
            return Err("Wait for the file transcription to finish".to_string());
        }
        if !coordinator.try_start() {
            return Err("A benchmark is already running".to_string());
        }
        1.0 - (dictation.vad_sensitivity as f32 / 100.0)
    };
    let guard = BenchmarkRunGuard(coordinator.clone());
    super::models::ensure_vad_model(&app_handle)
        .await
        .map_err(|error| format!("Could not prepare speech filtering: {error}"))?;

    tokio::task::spawn_blocking(move || {
        let _guard = guard;
        benchmark::run(&app_handle, &coordinator, request, vad_threshold)
    })
    .await
    .map_err(|error| format!("Benchmark task failed: {error}"))?
}

#[tauri::command]
pub fn cancel_benchmark(state: tauri::State<'_, State>) -> bool {
    let running = state.benchmark.is_running();
    if running {
        state.benchmark.cancel();
    }
    running
}
