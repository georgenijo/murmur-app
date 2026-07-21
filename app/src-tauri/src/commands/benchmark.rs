use crate::benchmark::{
    self, BenchmarkCoordinator, BenchmarkModel, BenchmarkReport, BenchmarkRequest,
};
use crate::state::DictationStatus;
use crate::{MutexExt, State};
use serde::Serialize;
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkActivity {
    benchmark_running: bool,
    file_transcribing: bool,
}

#[tauri::command]
pub fn get_benchmark_activity(state: tauri::State<'_, State>) -> BenchmarkActivity {
    BenchmarkActivity {
        benchmark_running: state.benchmark.is_running(),
        file_transcribing: state.app_state.file_transcribing.load(Ordering::SeqCst),
    }
}

#[tauri::command]
pub async fn run_benchmark(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, State>,
    request: BenchmarkRequest,
) -> Result<BenchmarkReport, String> {
    let coordinator = state.benchmark.clone();
    {
        let dictation = state.app_state.dictation.lock_or_recover();
        if dictation.status != DictationStatus::Idle {
            return Err("Stop recording before running a benchmark".to_string());
        }
        if state.app_state.file_transcribing.load(Ordering::SeqCst) {
            return Err("Wait for the file transcription to finish".to_string());
        }
        if !coordinator.try_start() {
            return Err(if coordinator.is_running() {
                "A benchmark is already running".to_string()
            } else {
                "Wait for model preparation to finish before benchmarking".to_string()
            });
        }
    }
    // Only one heavy inference runtime may be resident: stop any local-LLM
    // helper before benchmarking (fail-fast no-op while a transform is in
    // flight). The benchmark slot is already claimed above.
    state.transform_runtime.shutdown();
    let guard = BenchmarkRunGuard(coordinator.clone());
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

#[tauri::command]
pub fn cancel_benchmark(state: tauri::State<'_, State>) -> bool {
    let running = state.benchmark.is_running();
    if running {
        state.benchmark.cancel();
    }
    running
}

/// Write a benchmark report (already serialized on the frontend) to the
/// configured output folder under `file_name`. Returns the absolute path.
#[tauri::command]
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
#[tauri::command]
pub fn open_benchmark_output_folder(
    app_handle: tauri::AppHandle,
    output_dir: String,
) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let dir = crate::file_output::resolve_output_dir(&output_dir)?;
    app_handle
        .opener()
        .open_path(dir.to_string_lossy().into_owned(), None::<&str>)
        .map_err(|e| format!("Failed to open output folder: {}", e))
}
