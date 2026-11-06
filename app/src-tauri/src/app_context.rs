//! Tauri adapters only. The core never imports the host or looks up managed state.
use murmur_core::context::{
    AppContext, BackgroundFuture, BackgroundWork, ContextError, MainThread, Paths,
    PermissionStatus, Permissions, Task,
};
use std::{path::PathBuf, sync::Arc};
use tauri::Manager;

struct TauriHost(tauri::AppHandle);

impl Paths for TauriHost {
    fn app_data_dir(&self) -> Result<PathBuf, ContextError> {
        self.0
            .path()
            .app_data_dir()
            .map_err(|_| ContextError::PathUnavailable)
    }

    fn models_dir(&self) -> Option<PathBuf> {
        // Preserve the VAD install location; do not silently migrate models.
        dirs::data_dir().map(|root| root.join("local-dictation").join("models"))
    }
}

impl Permissions for TauriHost {
    fn microphone_status(&self) -> PermissionStatus {
        match crate::commands::permissions::check_microphone_permission_status().as_str() {
            "granted" => PermissionStatus::Granted,
            "denied" => PermissionStatus::Denied,
            "notDetermined" => PermissionStatus::NotDetermined,
            _ => PermissionStatus::Unknown,
        }
    }
}

impl BackgroundWork for TauriHost {
    fn spawn(&self, future: BackgroundFuture) -> Result<(), ContextError> {
        tauri::async_runtime::spawn(future);
        Ok(())
    }

    fn spawn_blocking(&self, task: Task) -> Result<(), ContextError> {
        tauri::async_runtime::spawn_blocking(task);
        Ok(())
    }
}

impl MainThread for TauriHost {
    fn dispatch(&self, task: Task) -> Result<(), ContextError> {
        self.0
            .run_on_main_thread(task)
            .map_err(|_| ContextError::SchedulingFailed)
    }
}

pub fn new(app: tauri::AppHandle) -> AppContext {
    let host = Arc::new(TauriHost(app));
    AppContext {
        paths: host.clone(),
        permissions: host.clone(),
        background: host.clone(),
        main_thread: host,
    }
}
