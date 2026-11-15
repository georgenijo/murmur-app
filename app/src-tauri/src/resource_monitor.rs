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

    /// Drop the cumulative baseline: the next sample primes a new one and
    /// reports CPU unavailable, instead of averaging over the gap.
    fn reset(&mut self) {
        self.primed = false;
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

/// Re-prime both cumulative CPU baselines. Call before sampling again after
/// samples were skipped: the next sample reports CPU unavailable (memory stays
/// measured) instead of a value averaged over the skipped interval.
fn reset_cpu_baselines() {
    if let Some(sampler) = PROCESS_CPU.get() {
        sampler
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .reset();
    }
    crate::platform::reset_cpu_baseline();
}

/// Tracks whether heartbeat samples were skipped, so the first sample after the
/// skip starts from fresh CPU baselines.
#[derive(Default)]
struct SkippedSamples {
    skipped: bool,
}

impl SkippedSamples {
    /// Report one heartbeat tick. Returns `true` when the baselines must be
    /// re-primed before sampling, that is on the first sampled tick after a skip.
    fn tick(&mut self, sampling: bool) -> bool {
        if !sampling {
            self.skipped = true;
            return false;
        }
        std::mem::take(&mut self.skipped)
    }
}

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
    app_handle: tauri::AppHandle,
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

pub fn start_heartbeat(app_handle: tauri::AppHandle) {
    use tauri::Manager;

    set_idle_timeout(app_handle.clone());

    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        let mut ticks = 0_u64;
        // One warning per outage: an unavailable store stays unavailable, so a
        // per-second warning only fills the log.
        let mut persist_failure_reported = false;
        let mut skipped_samples = SkippedSamples::default();
        loop {
            interval.tick().await;
            ticks = ticks.saturating_add(1);

            let state = app_handle.state::<crate::State>();
            let store_unavailable = state.performance.health().status
                == crate::performance_metrics::PerformanceStoreStatusV1::Unavailable;
            // Skipped samples leave the cumulative CPU baselines behind; without
            // a re-prime the first sample after recovery would average the outage.
            if skipped_samples.tick(!store_unavailable) {
                reset_cpu_baselines();
            }
            let persist_failure = if store_unavailable {
                Some("the diagnostics store is unavailable".to_string())
            } else {
                state
                    .performance
                    .insert_resource_sample(&sample_resources())
                    .err()
            };
            match persist_failure {
                Some(error) if !persist_failure_reported => {
                    persist_failure_reported = true;
                    tracing::warn!(
                        target: "system",
                        diagnostics_available = false,
                        "performance resource samples not persisted: {}",
                        error
                    );
                }
                Some(_) => {}
                None => persist_failure_reported = false,
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

#[tauri::command]
pub fn get_resource_usage() -> crate::performance_metrics::ResourceSampleV1 {
    sample_resources()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::DictationStatus;

    #[test]
    fn baselines_are_reprimed_once_when_sampling_resumes_after_an_outage() {
        let mut skipped = SkippedSamples::default();
        assert!(!skipped.tick(true), "no outage, nothing to re-prime");
        assert!(!skipped.tick(false));
        assert!(!skipped.tick(false), "still down, nothing is sampled");
        assert!(
            skipped.tick(true),
            "first sample after the outage re-primes"
        );
        assert!(!skipped.tick(true), "only the first one");
    }

    #[test]
    fn process_cpu_after_a_reset_is_unavailable_while_memory_stays_measured() {
        let mut sampler = ProcessCpuSampler::new(std::process::id());
        assert_eq!(sampler.sample().0, None, "first sample primes");
        assert!(sampler.sample().0.is_some(), "primed sampler measures");

        // An outage elapses without samples, then the heartbeat resumes.
        sampler.reset();
        let (cpu, rss) = sampler.sample();
        assert_eq!(cpu, None, "the first sample after a reset has no baseline");
        assert!(
            rss.is_some_and(|bytes| bytes > 0),
            "memory is instantaneous"
        );
        assert!(
            sampler.sample().0.is_some(),
            "the next sample measures again"
        );
    }

    #[test]
    fn resumed_sample_marks_cpu_unavailable_and_keeps_memory() {
        use crate::performance_metrics::MeasurementV1;

        // Prime both baselines, as a heartbeat running before the outage would.
        let _ = sample_resources();
        let _ = sample_resources();

        reset_cpu_baselines();
        let resumed = sample_resources();
        assert!(matches!(
            resumed.main_process.cpu_percent,
            MeasurementV1::Unavailable { .. }
        ));
        assert!(matches!(
            resumed.host.cpu_percent,
            MeasurementV1::Unavailable { .. }
        ));
        assert!(matches!(
            resumed.main_process.rss_bytes,
            MeasurementV1::Measured { .. }
        ));
        assert!(matches!(
            resumed.main_process.rust_heap_bytes,
            MeasurementV1::Measured { .. }
        ));
    }

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
