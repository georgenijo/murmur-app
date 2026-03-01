use std::sync::Mutex;
use serde::Serialize;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

static SYS: Mutex<Option<System>> = Mutex::new(None);

#[derive(Debug, Clone, Serialize)]
pub struct ResourceUsage {
    pub cpu_percent: f32,
    pub memory_mb: u64,
}

#[tauri::command]
pub fn get_resource_usage() -> ResourceUsage {
    let mut guard = SYS.lock().unwrap_or_else(|p| p.into_inner());
    let sys = guard.get_or_insert_with(System::new);
    let pid = sysinfo::get_current_pid().expect("failed to get PID");
    let refresh = ProcessRefreshKind::new()
        .with_cpu()
        .with_memory();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        false,
        refresh,
    );
    match sys.process(pid) {
        Some(proc_) => ResourceUsage {
            cpu_percent: proc_.cpu_usage(),
            memory_mb: proc_.memory() / 1_048_576,
        },
        None => ResourceUsage {
            cpu_percent: 0.0,
            memory_mb: 0,
        },
    }
}
