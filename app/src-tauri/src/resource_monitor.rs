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
// macOS-native CPU tracking via host_statistics64 (replaces sysinfo crate)
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
mod cpu {
    use std::os::raw::c_int;

    #[repr(C)]
    #[derive(Default)]
    struct HostCpuLoadInfo {
        ticks: [u32; 4], // user, system, idle, nice
    }

    const HOST_CPU_LOAD_INFO: c_int = 3;
    const HOST_CPU_LOAD_INFO_COUNT: u32 = 4; // 4 × u32

    const KERN_SUCCESS: c_int = 0;

    unsafe extern "C" {
        fn mach_host_self() -> u32;
        fn mach_task_self() -> u32;
        fn mach_port_deallocate(task: u32, name: u32) -> c_int;
        fn host_statistics64(
            host: u32,
            flavor: c_int,
            info: *mut HostCpuLoadInfo,
            count: *mut u32,
        ) -> c_int;
    }

    struct CpuSnapshot {
        user: u64,
        system: u64,
        idle: u64,
    }

    fn snapshot() -> CpuSnapshot {
        let mut info = HostCpuLoadInfo::default();
        let mut count = HOST_CPU_LOAD_INFO_COUNT;
        let host = unsafe { mach_host_self() };
        let kr = unsafe {
            host_statistics64(host, HOST_CPU_LOAD_INFO, &mut info, &mut count)
        };
        unsafe { mach_port_deallocate(mach_task_self(), host) };
        if kr != KERN_SUCCESS {
            return CpuSnapshot { user: 0, system: 0, idle: 0 };
        }
        CpuSnapshot {
            user: info.ticks[0] as u64 + info.ticks[3] as u64, // user + nice
            system: info.ticks[1] as u64,
            idle: info.ticks[2] as u64,
        }
    }

    static PREV: std::sync::Mutex<Option<CpuSnapshot>> = std::sync::Mutex::new(None);

    pub fn cpu_percent() -> f32 {
        let cur = snapshot();
        let mut prev = PREV.lock().unwrap_or_else(|p| p.into_inner());
        let pct = if let Some(ref p) = *prev {
            let d_user = cur.user.wrapping_sub(p.user);
            let d_sys = cur.system.wrapping_sub(p.system);
            let d_idle = cur.idle.wrapping_sub(p.idle);
            let total = d_user + d_sys + d_idle;
            if total > 0 {
                ((d_user + d_sys) as f64 / total as f64 * 100.0) as f32
            } else {
                0.0
            }
        } else {
            0.0
        };
        *prev = Some(cur);
        pct
    }
}

#[cfg(not(target_os = "macos"))]
mod cpu {
    pub fn cpu_percent() -> f32 { 0.0 }
}

// ---------------------------------------------------------------------------
// Idle timeout: release whisper model after inactivity
// ---------------------------------------------------------------------------

static IDLE_TIMEOUT: Mutex<Option<IdleState>> = Mutex::new(None);

struct IdleState {
    app_handle: tauri::AppHandle,
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
    let timeout_min = *state.app_state.idle_timeout_minutes.lock_or_recover();
    if timeout_min == 0 {
        return;
    }

    let threshold = std::time::Duration::from_secs(timeout_min as u64 * 60);
    let should_release = {
        let last = state.app_state.last_transcription_at.lock_or_recover();
        last.map_or(false, |t: std::time::Instant| t.elapsed() >= threshold)
    };
    if should_release {
        let mut backend = state.app_state.backend.lock_or_recover();
        // Re-check after acquiring the lock (a transcription may have started)
        let still_idle = {
            let last = state.app_state.last_transcription_at.lock_or_recover();
            last.map_or(false, |t: std::time::Instant| t.elapsed() >= threshold)
        };
        if still_idle {
            backend.reset();
            *state.app_state.last_transcription_at.lock_or_recover() = None;
            let rss = get_process_rss_mb();
            let heap = crate::rust_heap_mb();
            let ffi = crate::ffi_heap_mb();
            tracing::info!(target: "pipeline", rss_mb = rss, rust_heap_mb = heap, ffi_heap_mb = ffi, "whisper_idle_release");
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
        cpu_percent: cpu::cpu_percent(),
        memory_mb: rss_mb,
        rss_mb,
        rust_heap_mb: crate::rust_heap_mb(),
        ffi_heap_mb: crate::ffi_heap_mb(),
    }
}
