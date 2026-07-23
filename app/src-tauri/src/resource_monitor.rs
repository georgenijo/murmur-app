use crate::model_runtime::UnloadReason;
use std::sync::Mutex;

/// Get process RSS in bytes via `memory-stats` (task_info on macOS).
pub fn get_process_rss_bytes() -> Option<u64> {
    memory_stats::memory_stats().map(|statistics| statistics.physical_mem as u64)
}

/// Legacy internal helper retained for existing model/benchmark logs.
pub fn get_process_rss_mb() -> u64 {
    get_process_rss_bytes().unwrap_or(0) / 1_048_576
}

struct ProcessCpuSampler {
    system: sysinfo::System,
    primed: bool,
}

impl ProcessCpuSampler {
    fn new() -> Self {
        Self {
            system: sysinfo::System::new(),
            primed: false,
        }
    }

    fn sample(&mut self) -> Option<f32> {
        use sysinfo::{Pid, ProcessesToUpdate};

        let pid = Pid::from_u32(std::process::id());
        self.system
            .refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
        let value = self.system.process(pid).map(|process| process.cpu_usage());
        if value.is_some() && !self.primed {
            self.primed = true;
            return None;
        }
        value
    }
}

static PROCESS_CPU: std::sync::OnceLock<Mutex<ProcessCpuSampler>> = std::sync::OnceLock::new();

fn measured_or_unavailable<T>(value: Option<T>) -> crate::performance_metrics::MeasurementV1<T> {
    value.map_or(
        crate::performance_metrics::MeasurementV1::Unavailable {
            reason: crate::performance_metrics::UnavailableReasonV1::SampleFailed,
        },
        crate::performance_metrics::MeasurementV1::measured,
    )
}

pub fn sample_resources() -> crate::performance_metrics::ResourceSampleV1 {
    use crate::performance_metrics::{
        HostResourceSampleV1, MeasurementV1, ProcessResourceSampleV1, ResourceSampleV1,
        SidecarResourceSampleV1, UnavailableReasonV1, RESOURCE_SAMPLE_SCHEMA_VERSION,
    };

    let process_cpu = PROCESS_CPU
        .get_or_init(|| Mutex::new(ProcessCpuSampler::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .sample();
    let rust_heap = if cfg!(target_os = "macos") {
        MeasurementV1::measured(crate::rust_heap_bytes())
    } else {
        MeasurementV1::Unavailable {
            reason: UnavailableReasonV1::UnsupportedPlatform,
        }
    };
    let ffi_native_heap = if cfg!(target_os = "macos") {
        MeasurementV1::measured(crate::ffi_heap_bytes())
    } else {
        MeasurementV1::Unavailable {
            reason: UnavailableReasonV1::UnsupportedPlatform,
        }
    };

    ResourceSampleV1 {
        schema_version: RESOURCE_SAMPLE_SCHEMA_VERSION,
        observed_at_ms: chrono::Utc::now().timestamp_millis(),
        host: HostResourceSampleV1 {
            cpu_percent: measured_or_unavailable(crate::platform::cpu_percent()),
        },
        main_process: ProcessResourceSampleV1 {
            cpu_percent: measured_or_unavailable(process_cpu),
            rss_bytes: measured_or_unavailable(get_process_rss_bytes()),
            rust_heap_bytes: rust_heap,
            ffi_native_heap_bytes: ffi_native_heap,
        },
        // Phase A reserves the typed sidecar scope but does not inspect the
        // #332-owned transform runtime or llm_sidecar internals.
        sidecar_process: SidecarResourceSampleV1::dependency_pending(),
    }
}

// ---------------------------------------------------------------------------
// Idle timeout: release whisper model after inactivity
// ---------------------------------------------------------------------------

static IDLE_TIMEOUT: Mutex<Option<IdleState>> = Mutex::new(None);

struct IdleState {
    app_handle: tauri::AppHandle,
}

fn should_release_model(
    timeout_min: u32,
    status: crate::state::DictationStatus,
    idle_for: Option<std::time::Duration>,
) -> bool {
    timeout_min > 0
        && status == crate::state::DictationStatus::Idle
        && idle_for.is_some_and(|elapsed| {
            elapsed >= std::time::Duration::from_secs(timeout_min as u64 * 60)
        })
}

pub fn set_idle_timeout(app_handle: tauri::AppHandle) {
    if let Ok(mut guard) = IDLE_TIMEOUT.lock() {
        *guard = Some(IdleState { app_handle });
    }
}

fn check_idle_timeout() {
    use crate::MutexExt;
    use tauri::Manager;

    let handle = {
        let guard = match IDLE_TIMEOUT.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        match guard.as_ref() {
            Some(s) => s.app_handle.clone(),
            None => return,
        }
    };

    let state = handle.state::<crate::State>();
    if state.benchmark.is_running() {
        return;
    }
    let timeout_min = *state.app_state.idle_timeout_minutes.lock_or_recover();
    let should_release = {
        let status = state.app_state.dictation.lock_or_recover().status;
        let last = state.app_state.last_transcription_at.lock_or_recover();
        should_release_model(timeout_min, status, last.map(|t| t.elapsed()))
    };
    if should_release {
        // Hold the status lock through reset so a racing recording either makes
        // us skip release or starts after reset and prepares the model normally.
        let dictation = state.app_state.dictation.lock_or_recover();
        let still_idle = {
            let last = state.app_state.last_transcription_at.lock_or_recover();
            should_release_model(timeout_min, dictation.status, last.map(|t| t.elapsed()))
        };
        if still_idle && !state.benchmark.is_running() {
            let backend_name = state
                .app_state
                .model_runtime
                .unload(Some(&handle), UnloadReason::IdleTimeout)
                .ok()
                .flatten();
            *state.app_state.last_transcription_at.lock_or_recover() = None;
            let rss = get_process_rss_mb();
            let heap = crate::rust_heap_mb();
            let ffi = crate::ffi_heap_mb();
            tracing::info!(target: "pipeline", backend_released = backend_name.is_some(), rss_mb = rss, rust_heap_mb = heap, ffi_heap_mb = ffi, "model_idle_release");
        }
    }
}

// ---------------------------------------------------------------------------
// Heartbeat task: periodic telemetry + idle timeout check
// ---------------------------------------------------------------------------

pub fn start_heartbeat(app_handle: tauri::AppHandle) {
    use tauri::Manager;

    set_idle_timeout(app_handle.clone());

    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        let mut ticks = 0_u64;
        loop {
            interval.tick().await;
            ticks = ticks.saturating_add(1);

            let sample = sample_resources();
            let state = app_handle.state::<crate::State>();
            if let Err(error) = state.performance.insert_resource_sample(&sample) {
                tracing::warn!(
                    target: "system",
                    diagnostics_available = false,
                    "performance resource sample not persisted: {}",
                    error
                );
            }

            if ticks % 60 == 0 {
                let rss = get_process_rss_mb();
                let rust = crate::rust_heap_mb();
                let ffi = crate::ffi_heap_mb();
                tracing::info!(
                    target: "system",
                    rss_mb = rss,
                    rust_heap_mb = rust,
                    ffi_heap_mb = ffi,
                    "heartbeat"
                );

                check_idle_timeout();
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Tauri command
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn get_resource_usage() -> crate::performance_metrics::ResourceSampleV1 {
    sample_resources()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::DictationStatus;

    #[test]
    fn idle_release_requires_expired_idle_status() {
        let expired = Some(std::time::Duration::from_secs(5 * 60));
        assert!(should_release_model(5, DictationStatus::Idle, expired));
        assert!(!should_release_model(5, DictationStatus::Recording, expired));
        assert!(!should_release_model(5, DictationStatus::Processing, expired));
    }

    #[test]
    fn idle_release_respects_disabled_and_recent_activity() {
        assert!(!should_release_model(
            0,
            DictationStatus::Idle,
            Some(std::time::Duration::from_secs(60 * 60)),
        ));
        assert!(!should_release_model(
            5,
            DictationStatus::Idle,
            Some(std::time::Duration::from_secs(5 * 60 - 1)),
        ));
        assert!(!should_release_model(5, DictationStatus::Idle, None));
    }
}
