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
    pid: sysinfo::Pid,
    primed: bool,
}

impl ProcessCpuSampler {
    fn new(pid: u32) -> Self {
        Self {
            system: sysinfo::System::new(),
            pid: sysinfo::Pid::from_u32(pid),
            primed: false,
        }
    }

    fn sample(&mut self) -> (Option<f32>, Option<u64>) {
        use sysinfo::ProcessesToUpdate;
        self.system
            .refresh_processes(ProcessesToUpdate::Some(&[self.pid]), true);
        let process = self.system.process(self.pid);
        let cpu = process.map(|process| process.cpu_usage());
        let rss = process.map(|process| process.memory());
        if cpu.is_some() && !self.primed {
            self.primed = true;
            return (None, rss);
        }
        (cpu, rss)
    }
}

static PROCESS_CPU: std::sync::OnceLock<Mutex<ProcessCpuSampler>> = std::sync::OnceLock::new();
static SIDECAR_CPU: std::sync::OnceLock<Mutex<Option<ProcessCpuSampler>>> =
    std::sync::OnceLock::new();

fn measured_or_unavailable<T>(value: Option<T>) -> crate::performance_metrics::MeasurementV1<T> {
    value.map_or(
        crate::performance_metrics::MeasurementV1::Unavailable {
            reason: crate::performance_metrics::UnavailableReasonV1::SampleFailed,
        },
        crate::performance_metrics::MeasurementV1::measured,
    )
}

fn sample_sidecar(
    sidecar: &crate::llm_sidecar::LlmSidecar,
) -> crate::performance_metrics::SidecarResourceSampleV1 {
    use crate::performance_metrics::{SidecarResourceSampleV1, UnavailableReasonV1};

    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    {
        let _ = sidecar;
        return SidecarResourceSampleV1::unavailable(UnavailableReasonV1::UnsupportedPlatform);
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        let Some(pid) = sidecar.resident_pid() else {
            return SidecarResourceSampleV1::unavailable(UnavailableReasonV1::NoSamples);
        };
        sample_sidecar_pid(pid)
    }
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn sample_sidecar_pid(pid: u32) -> crate::performance_metrics::SidecarResourceSampleV1 {
    use crate::performance_metrics::{MeasurementV1, SidecarResourceSampleV1, UnavailableReasonV1};

    let mut slot = SIDECAR_CPU
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if slot.as_ref().map(|sampler| sampler.pid.as_u32()) != Some(pid) {
        *slot = Some(ProcessCpuSampler::new(pid));
    }
    let (cpu, rss) = slot.as_mut().expect("sidecar sampler initialized").sample();
    SidecarResourceSampleV1 {
        cpu_percent: cpu.map_or(
            MeasurementV1::Unavailable {
                reason: UnavailableReasonV1::SampleFailed,
            },
            MeasurementV1::measured,
        ),
        rss_bytes: rss.map_or(
            MeasurementV1::Unavailable {
                reason: UnavailableReasonV1::SampleFailed,
            },
            MeasurementV1::measured,
        ),
    }
}

pub fn sample_resources(
    sidecar: &crate::llm_sidecar::LlmSidecar,
) -> crate::performance_metrics::ResourceSampleV1 {
    use crate::performance_metrics::{
        HostResourceSampleV1, MeasurementV1, ProcessResourceSampleV1, ResourceSampleV1,
        UnavailableReasonV1, RESOURCE_SAMPLE_SCHEMA_VERSION,
    };

    let process_cpu = PROCESS_CPU
        .get_or_init(|| Mutex::new(ProcessCpuSampler::new(std::process::id())))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .sample()
        .0;
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
        sidecar_process: sample_sidecar(sidecar),
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

            let state = app_handle.state::<crate::State>();
            let sample = sample_resources(&state.transform_runtime);
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
pub fn get_resource_usage(
    state: tauri::State<'_, crate::State>,
) -> crate::performance_metrics::ResourceSampleV1 {
    sample_resources(&state.transform_runtime)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::performance_metrics::{MeasurementV1, UnavailableReasonV1};
    use crate::state::DictationStatus;

    #[test]
    fn idle_release_requires_expired_idle_status() {
        let expired = Some(std::time::Duration::from_secs(5 * 60));
        assert!(should_release_model(5, DictationStatus::Idle, expired));
        assert!(!should_release_model(
            5,
            DictationStatus::Recording,
            expired
        ));
        assert!(!should_release_model(
            5,
            DictationStatus::Processing,
            expired
        ));
    }

    #[test]
    fn nonresident_sidecar_is_never_reported_as_zero() {
        let sidecar = crate::llm_sidecar::LlmSidecar::new();
        let sample = sample_resources(&sidecar);
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        let expected = UnavailableReasonV1::NoSamples;
        #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
        let expected = UnavailableReasonV1::UnsupportedPlatform;
        assert_eq!(
            sample.sidecar_process.cpu_percent,
            MeasurementV1::Unavailable { reason: expected }
        );
        assert_eq!(
            sample.sidecar_process.rss_bytes,
            MeasurementV1::Unavailable { reason: expected }
        );
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    #[test]
    fn pid_sampler_reads_real_rss_and_primes_cpu_without_a_zero_sentinel() {
        let first = sample_sidecar_pid(std::process::id());
        assert!(matches!(
            first.rss_bytes,
            MeasurementV1::Measured { value } if value > 0
        ));
        assert_eq!(
            first.cpu_percent,
            MeasurementV1::Unavailable {
                reason: UnavailableReasonV1::SampleFailed
            }
        );
        let second = sample_sidecar_pid(std::process::id());
        assert!(matches!(second.cpu_percent, MeasurementV1::Measured { .. }));
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
