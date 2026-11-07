use crate::model_runtime::UnloadReason;
use std::sync::Mutex;

pub use murmur_core::memory::{get_process_rss_bytes, get_process_rss_mb};

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
        sidecar_process: crate::performance_metrics::SidecarResourceSampleV1::unavailable(
            UnavailableReasonV1::UnsupportedPlatform,
        ),
    }
}

// ---------------------------------------------------------------------------
// Idle timeout: release whisper model after inactivity
// ---------------------------------------------------------------------------

static IDLE_TIMEOUT: Mutex<Option<IdleState>> = Mutex::new(None);

struct IdleState {
    app_handle: crate::host::Context,
}

fn should_release_model(
    release_after: Option<std::time::Duration>,
    status: crate::state::DictationStatus,
    idle_for: Option<std::time::Duration>,
) -> bool {
    release_after.is_some_and(|timeout| {
        status == crate::state::DictationStatus::Idle
            && idle_for.is_some_and(|elapsed| elapsed >= timeout)
    })
}

pub fn set_idle_timeout(app_handle: crate::host::Context) {
    if let Ok(mut guard) = IDLE_TIMEOUT.lock() {
        *guard = Some(IdleState { app_handle });
    }
}

fn check_idle_timeout() {
    use crate::host::ServiceAccess;
    use crate::MutexExt;

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

    let state = handle.services();
    if state.benchmark.is_running()
        || state
            .app_state
            .meeting_inference_active
            .load(std::sync::atomic::Ordering::SeqCst)
    {
        return;
    }
    let policy = *state.app_state.model_idle_policy.lock_or_recover();
    // Read once per check so both release gates below see the same answer.
    let release_after = policy.release_after(crate::platform::on_external_power);
    let should_release = {
        let status = state.app_state.dictation.lock_or_recover().status;
        let last = state.app_state.last_transcription_at.lock_or_recover();
        should_release_model(release_after, status, last.map(|t| t.elapsed()))
    };
    if should_release {
        // Hold the status lock through reset so a racing recording either makes
        // us skip release or starts after reset and prepares the model normally.
        let dictation = state.app_state.dictation.lock_or_recover();
        let still_idle = {
            let last = state.app_state.last_transcription_at.lock_or_recover();
            should_release_model(release_after, dictation.status, last.map(|t| t.elapsed()))
        };
        if still_idle
            && !state.benchmark.is_running()
            && !state
                .app_state
                .meeting_inference_active
                .load(std::sync::atomic::Ordering::SeqCst)
        {
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

pub fn start_heartbeat(app_handle: crate::host::Context) {
    use crate::host::ServiceAccess;

    set_idle_timeout(app_handle.clone());

    crate::host::runtime::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        let mut ticks = 0_u64;
        loop {
            interval.tick().await;
            ticks = ticks.saturating_add(1);

            let state = app_handle.services();
            let sample = sample_resources();
            if let Err(error) = state.performance.insert_resource_sample(&sample) {
                tracing::warn!(
                    target: "system",
                    diagnostics_available = false,
                    "performance resource sample not persisted: {}",
                    error
                );
            }

            if ticks.is_multiple_of(60) {
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

#[cfg_attr(feature = "tauri-host", tauri::command)]
pub fn get_resource_usage() -> crate::performance_metrics::ResourceSampleV1 {
    sample_resources()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::DictationStatus;

    #[test]
    fn idle_release_requires_expired_idle_status() {
        let five = Some(std::time::Duration::from_secs(5 * 60));
        let expired = five;
        assert!(should_release_model(five, DictationStatus::Idle, expired));
        assert!(!should_release_model(
            five,
            DictationStatus::Recording,
            expired
        ));
        assert!(!should_release_model(
            five,
            DictationStatus::Processing,
            expired
        ));
    }

    #[test]
    fn idle_release_respects_disabled_and_recent_activity() {
        let five = Some(std::time::Duration::from_secs(5 * 60));
        assert!(!should_release_model(
            None,
            DictationStatus::Idle,
            Some(std::time::Duration::from_secs(60 * 60)),
        ));
        assert!(!should_release_model(
            five,
            DictationStatus::Idle,
            Some(std::time::Duration::from_secs(5 * 60 - 1)),
        ));
        assert!(!should_release_model(five, DictationStatus::Idle, None));
    }
}
