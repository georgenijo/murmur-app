use std::sync::Mutex;
use serde::Serialize;
use sysinfo::System;

static SYS: Mutex<Option<System>> = Mutex::new(None);

#[derive(Debug, Clone, Serialize)]
pub struct ResourceUsage {
    pub cpu_percent: f32,
    pub memory_mb: u64,
}

#[tauri::command]
pub fn get_resource_usage() -> ResourceUsage {
    let mut guard = SYS.lock().unwrap_or_else(|p| p.into_inner());
    let sys = guard.get_or_insert_with(System::new_all);
    sys.refresh_cpu_usage();
    sys.refresh_memory();
    ResourceUsage {
        cpu_percent: sys.global_cpu_usage(),
        memory_mb: sys.used_memory() / 1_048_576,
    }
}
