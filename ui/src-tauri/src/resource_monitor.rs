use std::sync::Mutex;
use serde::Serialize;
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};

static SYS: Mutex<Option<System>> = Mutex::new(None);

#[derive(Debug, Clone, Serialize)]
pub struct ResourceUsage {
    pub cpu_percent: f32,
    pub memory_mb: u64,
}

#[tauri::command]
pub fn get_resource_usage() -> ResourceUsage {
    let mut guard = SYS.lock().unwrap_or_else(|p| p.into_inner());
    // Initialize with only CPU and memory subsystems to avoid loading
    // processes, disks, and network data we don't need.
    let sys = guard.get_or_insert_with(|| {
        System::new_with_specifics(
            RefreshKind::new()
                .with_cpu(CpuRefreshKind::new().with_cpu_usage())
                .with_memory(MemoryRefreshKind::new().with_ram()),
        )
    });
    // Note: the first call to refresh_cpu_usage() yields ~0% because
    // System::new_with_specifics() establishes a baseline snapshot and
    // global_cpu_usage() measures the delta since the last refresh.
    // The persistent static SYS ensures subsequent 1-second polls are accurate;
    // the initial ~0% reading is expected and harmless.
    sys.refresh_cpu_usage();
    sys.refresh_memory();
    ResourceUsage {
        cpu_percent: sys.global_cpu_usage(),
        memory_mb: sys.used_memory() / 1_048_576,
    }
}
