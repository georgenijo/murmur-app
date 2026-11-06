//! Thin host adapter: native producers submit once, windows only observe.
use murmur_core::completion::producers::Producers;
use murmur_core::completion::{
    Command, CompletionService, Destination, GateAction, HistoryMutation, LegacyImport,
    MigrationGate, Snapshot,
};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{Emitter, Manager};

pub(crate) struct Completion {
    service: CompletionService,
    pub(crate) producers: Producers,
    pub(crate) admitted: AtomicBool,
}
pub(crate) fn initialize(app: &tauri::AppHandle) -> Result<(), String> {
    let root = crate::commands::settings_store::data_dir(app)?;
    // Existence only; never enumerate/read the WebKit store. Isolated bundle
    // identities use their own directory, with no access to production data.
    let webkit = app
        .path()
        .home_dir()
        .map_err(|_| "Home directory unavailable")?
        .join("Library/WebKit")
        .join(&app.config().identifier);
    let service = CompletionService::start(root, webkit.exists()).map_err(|e| e.to_string())?;
    app.manage(Completion {
        producers: Producers::new(service.clone(), uuid::Uuid::new_v4().to_string()),
        service,
        admitted: AtomicBool::new(false),
    });
    Ok(())
}
pub(crate) fn start_gate(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut gate = MigrationGate::default();
        let start = std::time::Instant::now();
        loop {
            let state = app.state::<Completion>();
            let snapshot = state.service.execute(Command::Snapshot);
            let action = gate.tick(
                snapshot.is_ok(),
                snapshot.as_ref().is_ok_and(|s| s.v1_committed),
                start.elapsed().as_millis() as u64,
            );
            match action {
                GateAction::Arm | GateAction::ArmHolding => {
                    state.admitted.store(true, Ordering::SeqCst);
                    if action == GateAction::ArmHolding {
                        tracing::warn!(target:"completion",code="migration_timeout_held","completion migration timed out");
                    }
                    crate::dictation_control::refresh(&app).await;
                    break;
                }
                GateAction::ReloadOnce => {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.eval("location.reload()");
                    }
                }
                GateAction::Wait => {}
            }
            if snapshot.is_err() {
                let _ = app.emit_to("main", "completion-storage-error", ());
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    });
}
pub(crate) fn admitted(app: &tauri::AppHandle) -> bool {
    app.try_state::<Completion>()
        .is_some_and(|s| s.admitted.load(Ordering::SeqCst))
}
fn execute(app: &tauri::AppHandle, command: Command) -> Result<Snapshot, String> {
    let result = app
        .state::<Completion>()
        .service
        .execute(command)
        .map_err(|e| e.to_string());
    if result.is_ok() {
        let _ = app.emit("history-changed", ());
        let _ = app.emit("statistics-changed", ());
    }
    result
}
pub(crate) fn produce(
    app: &tauri::AppHandle,
    f: impl FnOnce(&Producers) -> murmur_core::completion::Result<Snapshot>,
) {
    let state = app.state::<Completion>();
    match f(&state.producers) {
        Ok(_) => {
            let _ = app.emit("history-changed", ());
            let _ = app.emit("statistics-changed", ());
        }
        Err(_) => {
            tracing::warn!(target:"completion",code="completion_write_failed","completion service write failed");
            let _ = app.emit_to("main", "completion-storage-error", ());
        }
    }
}
/// A failed/cancelled native operation drops its pending content on every exit.
pub(crate) struct FileOperation<'a> {
    producers: &'a Producers,
    operation: String,
    ready: bool,
}
impl<'a> FileOperation<'a> {
    pub(crate) fn begin(producers: &'a Producers, operation: String) -> Result<Self, String> {
        producers
            .begin_file(&operation)
            .map_err(|e| e.to_string())?;
        Ok(Self {
            producers,
            operation,
            ready: false,
        })
    }
    pub(crate) fn finish(mut self, entry: Option<serde_json::Value>, retain: bool) {
        self.producers.finish_file(&self.operation, entry, retain);
        self.ready = true;
    }
}
impl Drop for FileOperation<'_> {
    fn drop(&mut self) {
        if !self.ready {
            self.producers.discard_file(&self.operation);
        }
    }
}
#[tauri::command]
pub(crate) fn cancel_file_transcription(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    operation_id: String,
) -> Result<(), String> {
    main_only(&window)?;
    app.state::<Completion>()
        .producers
        .cancel_file(&operation_id);
    Ok(())
}
#[tauri::command]
pub(crate) fn accept_file_transcription(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    operation_id: String,
) -> Result<(), String> {
    main_only(&window)?;
    if app
        .state::<Completion>()
        .producers
        .accept_file(&operation_id)
        .map_err(|e| e.to_string())?
        .is_some()
    {
        let _ = app.emit("history-changed", ());
    }
    Ok(())
}
pub(crate) fn retain(app: &tauri::AppHandle) -> bool {
    app.state::<crate::dictation_control::Control>()
        .settings
        .lock()
        .unwrap_or_else(|p| p.into_inner())["retainHistory"]
        .as_bool()
        .unwrap_or(true)
}
pub(crate) fn now() -> u64 {
    chrono::Utc::now().timestamp_millis().max(0) as u64
}
pub(crate) fn day() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}
pub(crate) fn entry(text: &str, duration: f64, source: &str) -> serde_json::Value {
    serde_json::json!({"schemaVersion":2,"id":uuid::Uuid::new_v4().to_string(),"text":text,"duration":duration,"source":source,"timestamp":now(),"pinned":false})
}
fn main_only(window: &tauri::WebviewWindow) -> Result<(), String> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err("Completion mutations belong to the main window".into())
    }
}
#[tauri::command]
pub(crate) fn completion_snapshot(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
) -> Result<Snapshot, String> {
    if !matches!(window.label(), "main" | "diagnostics") {
        return Err("Completion data is unavailable in this window".into());
    }
    app.state::<Completion>()
        .service
        .execute(Command::Snapshot)
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub(crate) fn import_completion_v1(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    source: LegacyImport,
) -> Result<Snapshot, String> {
    main_only(&window)?;
    execute(&app, Command::Import(source))
}
#[tauri::command]
pub(crate) fn mutate_history(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    mutation: HistoryMutation,
) -> Result<Snapshot, String> {
    main_only(&window)?;
    execute(&app, Command::History(mutation))
}
#[tauri::command]
pub(crate) fn clear_completion_destination(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    destination: Destination,
) -> Result<Snapshot, String> {
    main_only(&window)?;
    execute(
        &app,
        Command::Clear {
            destination,
            at: now(),
        },
    )
}
#[tauri::command]
pub(crate) fn store_benchmark_report(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    report: serde_json::Value,
) -> Result<Snapshot, String> {
    main_only(&window)?;
    execute(&app, Command::Report(report))
}

#[tauri::command]
pub(crate) fn acknowledge_completion_keys(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    destinations: Vec<Destination>,
) -> Result<(), String> {
    main_only(&window)?;
    app.state::<Completion>()
        .service
        .execute(Command::KeysRemoved(destinations))
        .map(|_| ())
        .map_err(|e| e.to_string())
}
