//! Tauri adapters only. The core never imports the host or looks up managed state.
use murmur_core::context::{
    AppContext, BackgroundFuture, BackgroundWork, ContextError, MainThread, Paths,
    PermissionStatus, Permissions, Task,
};
use std::{path::PathBuf, sync::Arc};
use tauri::Manager;

#[cfg(target_os = "macos")]
#[path = "main_thread_queue.rs"]
mod main_thread_queue;

struct TauriHost {
    app: tauri::AppHandle,
    #[cfg(target_os = "macos")]
    main_thread: main_thread_queue::MainThreadQueue,
}

impl Paths for TauriHost {
    fn app_data_dir(&self) -> Result<PathBuf, ContextError> {
        self.app
            .path()
            .app_data_dir()
            .map_err(|_| ContextError::PathUnavailable)
    }

    fn models_dir(&self) -> Option<PathBuf> {
        // Preserve the VAD install location; do not silently migrate models.
        crate::identity::data_dir().map(|root| root.join("local-dictation").join("models"))
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
        #[cfg(target_os = "macos")]
        {
            self.main_thread.dispatch(task)
        }
        #[cfg(not(target_os = "macos"))]
        {
            // No always-queued native adapter exists for other hosts yet.
            // Tauri's run_on_main_thread may execute inline and is unsuitable.
            drop(task);
            Err(ContextError::SchedulingFailed)
        }
    }
}

pub fn new(app: tauri::AppHandle) -> AppContext {
    let host = Arc::new(TauriHost {
        app,
        #[cfg(target_os = "macos")]
        main_thread: Default::default(),
    });
    AppContext {
        paths: host.clone(),
        permissions: host.clone(),
        background: host.clone(),
        main_thread: host,
    }
}
