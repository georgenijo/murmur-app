use std::sync::Mutex;
use serde::Serialize;
use sysinfo::{CpuRefreshKind, RefreshKind, System};

static SYS: Mutex<Option<System>> = Mutex::new(None);

#[derive(Debug, Clone, Serialize)]
pub struct ResourceUsage {
    pub cpu_percent: f32,
    pub memory_mb: u64,
    pub rss_mb: u64,
    pub rust_heap_mb: u64,
}

/// Get process RSS in megabytes via `memory-stats` (task_info on macOS).
pub fn get_process_rss_mb() -> u64 {
    memory_stats::memory_stats()
        .map(|s| s.physical_mem as u64 / 1_048_576)
        .unwrap_or(0)
}

#[tauri::command]
pub fn get_resource_usage() -> ResourceUsage {
    let mut guard = SYS.lock().unwrap_or_else(|p| p.into_inner());
    let sys = guard.get_or_insert_with(|| {
        System::new_with_specifics(
            RefreshKind::new()
                .with_cpu(CpuRefreshKind::new().with_cpu_usage()),
        )
    });
    sys.refresh_cpu_usage();

    let rss_mb = get_process_rss_mb();

    ResourceUsage {
        cpu_percent: sys.global_cpu_usage(),
        memory_mb: rss_mb,
        rss_mb,
        rust_heap_mb: crate::rust_heap_mb(),
    }
}
