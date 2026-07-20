use crate::model_runtime::UnloadReason;
use serde::Serialize;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize)]
pub struct ResourceUsage {
    pub cpu_percent: f32,
    pub memory_mb: u64,
    pub rss_mb: u64,
    pub rust_heap_mb: u64,
    pub ffi_heap_mb: u64,
}

/// Get process RSS in megabytes via `memory-stats` (task_info on macOS).
pub fn get_process_rss_mb() -> u64 {
    memory_stats::memory_stats()
        .map(|s| s.physical_mem as u64 / 1_048_576)
        .unwrap_or(0)
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
    set_idle_timeout(app_handle.clone());

    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;

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
    });
}

// ---------------------------------------------------------------------------
// Tauri command
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn get_resource_usage() -> ResourceUsage {
    let rss_mb = get_process_rss_mb();
    ResourceUsage {
        cpu_percent: crate::platform::cpu_percent(),
        memory_mb: rss_mb,
        rss_mb,
        rust_heap_mb: crate::rust_heap_mb(),
        ffi_heap_mb: crate::ffi_heap_mb(),
    }
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
