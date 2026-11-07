//! Thin host adapter: native producers submit once, windows only observe.
use crate::events::EventsExt;
use murmur_core::completion::producers::Producers;
use murmur_core::completion::{
    Command, CompletionService, Destination, GateAction, HistoryMutation, LegacyImport,
    MigrationGate, Snapshot,
};
use murmur_core::events::{EventKind, EventSink};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Manager;

/// Every core command can retry a retained lifecycle save while draining its
/// FIFO, including native completion producers and file acceptance. Core fences
/// publication with the shared source mutex; this observer retires host ownership
/// and invalidates the warning after that publication.
struct SourcePublicationObserver<'a, R: tauri::Runtime>(&'a tauri::AppHandle<R>);
impl<R: tauri::Runtime> Drop for SourcePublicationObserver<'_, R> {
    fn drop(&mut self) {
        crate::commands::settings_store::reconcile_completion_publication(self.0);
    }
}

pub(crate) struct Completion {
    service: CompletionService,
    pub(crate) producers: Producers,
    pub(crate) admitted: AtomicBool,
}
struct CompletionInitialization {
    failed: AtomicBool,
}
pub(crate) fn initialize(app: &tauri::AppHandle) -> Result<(), String> {
    app.manage(CompletionInitialization {
        failed: AtomicBool::new(false),
    });
    crate::startup::spawn(app, "completion-init", |app| {
        match initialize_worker(&app) {
            Ok(()) => start_gate(app),
            Err(_) => {
                app.state::<CompletionInitialization>()
                    .failed
                    .store(true, Ordering::SeqCst);
                storage_error(&app);
            }
        }
    })
}
fn ready<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<tauri::State<'_, Completion>, String> {
    app.try_state::<Completion>()
        .ok_or_else(|| "Completion storage is still initializing".into())
}
async fn wait_ready(app: &tauri::AppHandle) -> Result<(), String> {
    loop {
        if app.try_state::<Completion>().is_some() {
            return Ok(());
        }
        if crate::shutdown::closing()
            || app
                .try_state::<CompletionInitialization>()
                .is_none_or(|state| state.failed.load(Ordering::SeqCst))
        {
            return Err("Completion storage unavailable".into());
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}
fn initialize_worker(app: &tauri::AppHandle) -> Result<(), String> {
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
    initialize_with_service(app, service.clone());
    crate::commands::updater::initialize(app, service)?;
    Ok(())
}
pub(crate) fn initialize_with_service<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    service: CompletionService,
) {
    app.manage(Completion {
        producers: Producers::new(service.clone(), uuid::Uuid::new_v4().to_string()),
        service,
        admitted: AtomicBool::new(false),
    });
}

pub(crate) fn start_gate(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut gate = MigrationGate::default();
        let start = std::time::Instant::now();
        loop {
            if crate::shutdown::closing() {
                break;
            }
            let worker_app = app.clone();
            let snapshot = tauri::async_runtime::spawn_blocking(move || {
                let _settings_observer = SourcePublicationObserver(&worker_app);
                ready(&worker_app)?
                    .service
                    .execute(Command::Snapshot)
                    .map_err(|e| e.to_string())
            })
            .await
            .unwrap_or_else(|_| Err("Completion worker unavailable".into()));
            let action = gate.tick(
                snapshot.is_ok(),
                snapshot.as_ref().is_ok_and(|s| s.v1_committed),
                start.elapsed().as_millis() as u64,
            );
            match action {
                GateAction::Arm | GateAction::ArmHolding => {
                    let Ok(state) = ready(&app) else {
                        break;
                    };
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
                let _ = app.events().send_to(
                    murmur_core::events::Consumer::Main,
                    "completion-storage-error",
                    (),
                );
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
    let _settings_observer = SourcePublicationObserver(app);
    let result = ready(app)?
        .service
        .execute(command)
        .map_err(|e| e.to_string());
    if result.is_ok() {
        let _ = app.events().publish(EventKind::HistoryChanged, ());
        let _ = app.events().publish(EventKind::StatisticsChanged, ());
    }
    result
}
pub(crate) fn produce<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    f: impl FnOnce(&Producers) -> murmur_core::completion::Result<Snapshot>,
) {
    let _settings_observer = SourcePublicationObserver(app);
    let Ok(state) = ready(app) else {
        storage_error(app);
        return;
    };
    match f(&state.producers) {
        Ok(_) => {
            let _ = app.events().publish(EventKind::HistoryChanged, ());
            let _ = app.events().publish(EventKind::StatisticsChanged, ());
        }
        Err(_) => {
            storage_error(app);
        }
    }
}
fn storage_error<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    tracing::warn!(target:"completion",code="completion_write_failed","completion service write failed");
    let _ = app.events().send_to(
        murmur_core::events::Consumer::Main,
        "completion-storage-error",
        (),
    );
}
/// A failed/cancelled native operation drops its pending content on every exit.
pub(crate) struct FileOperation<'a> {
    _permit: crate::shutdown::WriterPermit,
    producers: &'a Producers,
    operation: String,
    ready: bool,
}
impl<'a> FileOperation<'a> {
    pub(crate) fn begin(producers: &'a Producers, operation: String) -> Result<Self, String> {
        let permit = crate::shutdown::WriterPermit::acquire()?;
        producers
            .begin_file(&operation)
            .map_err(|e| e.to_string())?;
        Ok(Self {
            _permit: permit,
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
pub(crate) fn cancel_file_transcription<R: tauri::Runtime>(
    window: tauri::WebviewWindow<R>,
    app: tauri::AppHandle<R>,
    operation_id: String,
) -> Result<(), String> {
    main_only(&window)?;
    ready(&app)?.producers.cancel_file(&operation_id);
    Ok(())
}
#[tauri::command]
pub(crate) async fn accept_file_transcription<R: tauri::Runtime>(
    window: tauri::WebviewWindow<R>,
    app: tauri::AppHandle<R>,
    operation_id: String,
) -> Result<(), String> {
    main_only(&window)?;
    let permit = crate::shutdown::WriterPermit::acquire()?;
    let worker_app = app.clone();
    if tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        let _settings_observer = SourcePublicationObserver(&worker_app);
        ready(&worker_app)?
            .producers
            .accept_file(&operation_id)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|_| {
        storage_error(&app);
        "Completion worker unavailable".to_owned()
    })?
    .map_err(|e| {
        storage_error(&app);
        e.to_string()
    })?
    .is_some()
    {
        let _ = app.events().publish(EventKind::HistoryChanged, ());
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
fn main_only<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>) -> Result<(), String> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err("Completion mutations belong to the main window".into())
    }
}
#[tauri::command]
pub(crate) async fn recover_completion_pending(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
) -> Result<Snapshot, String> {
    main_only(&window)?;
    let _permit = crate::shutdown::WriterPermit::acquire()?;
    crate::shutdown::source_call(move || execute(&app, Command::RecoverPending))
        .await
        .map_err(|_| "Completion recovery worker unavailable".to_string())?
}
#[tauri::command]
pub(crate) async fn completion_snapshot(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
) -> Result<Snapshot, String> {
    if !matches!(window.label(), "main" | "diagnostics") {
        return Err("Completion data is unavailable in this window".into());
    }
    wait_ready(&app).await?;
    tauri::async_runtime::spawn_blocking(move || {
        let _settings_observer = SourcePublicationObserver(&app);
        ready(&app)?
            .service
            .execute(Command::Snapshot)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|_| "Completion worker unavailable".to_owned())?
}
#[tauri::command(async)]
pub(crate) fn import_completion_v1(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    source: LegacyImport,
) -> Result<Snapshot, String> {
    main_only(&window)?;
    execute(&app, Command::Import(source))
}
#[tauri::command(async)]
pub(crate) fn mutate_history(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    mutation: HistoryMutation,
) -> Result<Snapshot, String> {
    main_only(&window)?;
    execute(&app, Command::History(mutation))
}
#[tauri::command(async)]
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
#[tauri::command(async)]
pub(crate) fn store_benchmark_report(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    report: serde_json::Value,
) -> Result<Snapshot, String> {
    main_only(&window)?;
    execute(&app, Command::Report(report))
}

#[tauri::command(async)]
pub(crate) fn acknowledge_completion_keys(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    destinations: Vec<Destination>,
) -> Result<(), String> {
    main_only(&window)?;
    let _settings_observer = SourcePublicationObserver(&app);
    ready(&app)?
        .service
        .execute(Command::KeysRemoved(destinations))
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod dispatch_tests {
    use super::*;
    use std::{
        io::Write,
        sync::{mpsc, Arc},
        time::Duration,
    };

    // Exercises the actual generated Tauri wrappers and Webview::on_message on
    // one dispatch thread. A synthetic FIFO blocks the real completion owner.
    // MockRuntime does not run WKWebView's Objective-C main-thread callback.
    #[test]
    fn file_cancel_ipc_completes_while_accept_owner_is_blocked() {
        let root = tempfile::tempdir().unwrap();
        let service = CompletionService::start(root.path().into(), false).unwrap();
        service.execute(Command::Snapshot).unwrap();
        let path = root.path().join("completion-holding.json");
        let holding = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        let c_path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
        // SAFETY: synthetic temporary path, valid nul-terminated string.
        assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
        let (blocked, observed) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        let released = Arc::new(AtomicBool::new(false));
        let writer_released = released.clone();
        let writer = std::thread::spawn(move || {
            let mut fifo = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
            // The in-flight read owns the FIFO; later reads use this regular file.
            let replacement = path.with_extension("synthetic-replacement");
            std::fs::write(&replacement, &holding).unwrap();
            std::fs::rename(replacement, &path).unwrap();
            blocked.send(()).unwrap();
            // Watchdog releases even if the old synchronous handler deadlocks
            // this test's dispatcher, making the regression fail without hanging.
            let _ = resume.recv_timeout(Duration::from_secs(5));
            writer_released.store(true, Ordering::SeqCst);
            fifo.write_all(&holding).unwrap();
        });
        let producers = Producers::new(service.clone(), "synthetic-ipc".into());
        producers.begin_file("synthetic-operation").unwrap();
        producers.finish_file(
            "synthetic-operation",
            Some(entry("synthetic", 1., "file")),
            true,
        );
        let app = tauri::test::mock_builder()
            .manage(Completion {
                service,
                producers,
                admitted: AtomicBool::new(true),
            })
            .invoke_handler(tauri::generate_handler![
                accept_file_transcription,
                cancel_file_transcription
            ])
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        let request = |cmd: &str| tauri::webview::InvokeRequest {
            cmd: cmd.into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: "tauri://localhost".parse().unwrap(),
            body: tauri::ipc::InvokeBody::Json(
                serde_json::json!({"operationId":"synthetic-operation"}),
            ),
            headers: Default::default(),
            invoke_key: tauri::test::INVOKE_KEY.into(),
        };
        let (accepted, acceptance) = mpsc::channel();
        window.as_ref().clone().on_message(
            request("accept_file_transcription"),
            Box::new(move |_, _, response, _, _| {
                accepted.send(response).unwrap();
            }),
        );
        // The owner opened the FIFO while processing this acceptance, so the
        // production adapter now holds files and is waiting for the owner.
        observed.recv_timeout(Duration::from_secs(5)).unwrap();
        let (cancelled, cancellation) = mpsc::channel();
        window.as_ref().clone().on_message(
            request("cancel_file_transcription"),
            Box::new(move |_, _, response, _, _| {
                cancelled.send(response).unwrap();
            }),
        );
        let response = cancellation.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(matches!(response, tauri::ipc::InvokeResponse::Ok(_)));
        assert!(
            !released.load(Ordering::SeqCst),
            "cancel queued behind acceptance"
        );
        release.send(()).unwrap();
        writer.join().unwrap();
        assert!(matches!(
            acceptance.recv_timeout(Duration::from_secs(5)).unwrap(),
            tauri::ipc::InvokeResponse::Ok(_)
        ));
        assert!(app
            .state::<Completion>()
            .service
            .execute(Command::Snapshot)
            .unwrap()
            .history
            .is_empty());
    }
}

fn webkit_state_snapshot_worker(
    app: tauri::AppHandle,
) -> Result<murmur_core::completion::lifecycle::Receipt, String> {
    let _warning_notification = crate::commands::settings_store::SettingsWarningNotification(&app);
    source_status(
        &ready(&app)?.service,
        &crate::commands::settings_store::data_dir(&app)?,
    )
}
fn source_status(
    service: &CompletionService,
    root: &std::path::Path,
) -> Result<murmur_core::completion::lifecycle::Receipt, String> {
    crate::commands::settings_store::reconcile_lifecycle_after(root, || {
        service
            .lifecycle(murmur_core::completion::lifecycle::Operation::Status)
            .map_err(|e| e.to_string())
    })
}
fn import_webkit_state_worker(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    source: std::collections::BTreeMap<String, Option<String>>,
) -> Result<murmur_core::completion::lifecycle::Receipt, String> {
    main_only(&window)?;
    let _warning_notification = crate::commands::settings_store::SettingsWarningNotification(&app);
    let root = crate::commands::settings_store::data_dir(&app)?;
    crate::commands::settings_store::reconcile_lifecycle_after(&root, || {
        ready(&app)?
            .service
            .lifecycle(murmur_core::completion::lifecycle::Operation::Import(
                source,
            ))
            .map_err(|e| e.to_string())
    })
}
fn save_webkit_source_worker(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    key: String,
    value: serde_json::Value,
) -> Result<(), String> {
    main_only(&window)?;
    if matches!(
        key.as_str(),
        "dictation-settings"
            | "murmur-theme-library"
            | "updater-last-check"
            | "pending-update-release-notes"
    ) {
        return Err("Use the source owner's command".into());
    }
    let _warning_notification = crate::commands::settings_store::SettingsWarningNotification(&app);
    let root = crate::commands::settings_store::data_dir(&app)?;
    crate::commands::settings_store::reconcile_lifecycle_after(&root, || {
        ready(&app)?
            .service
            .lifecycle(murmur_core::completion::lifecycle::Operation::Save { key, value })
            .map(|_| ())
            .map_err(|e| e.to_string())
    })
}

#[cfg(test)]
mod source_ordering_tests {
    use super::*;
    use crate::MutexExt;
    #[test]
    fn lifecycle_and_shutdown_reconciliation_do_not_hold_lock_while_waiting() {
        let root = tempfile::tempdir().unwrap();
        let service = CompletionService::start(root.path().into(), false).unwrap();
        service.execute(Command::Snapshot).unwrap();
        for shutdown in [false, true] {
            // Block the core's lifecycle critical section. The adapter must not
            // acquire this mutex around the wait, nor retain it for reconciliation.
            let guard = crate::commands::settings_store::WRITE_LOCK.lock_or_recover();
            let owner = service.clone();
            let path = root.path().to_path_buf();
            let (started, start) = std::sync::mpsc::channel();
            let (finished, finish) = std::sync::mpsc::channel();
            let worker = std::thread::spawn(move || {
                started.send(()).unwrap();
                if shutdown {
                    let receipt =
                        crate::commands::settings_store::reconcile_lifecycle_after(&path, || {
                            tauri::async_runtime::block_on(owner.shutdown_receipt())
                        });
                    assert_eq!(receipt, murmur_core::shutdown::WriteOutcome::Committed);
                } else {
                    source_status(&owner, &path).unwrap();
                }
                finished.send(()).unwrap();
            });
            start.recv().unwrap();
            drop(guard);
            finish
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            worker.join().unwrap();
        }
    }

    #[test]
    fn absent_webkit_status_serializes_with_source_writer() {
        let root = tempfile::tempdir().unwrap();
        let service = CompletionService::start(root.path().into(), false).unwrap();
        let guard = crate::commands::settings_store::WRITE_LOCK.lock_or_recover();
        murmur_core::completion::save_source(
            root.path(),
            "dictation-settings",
            r#"{"language":"es"}"#,
        )
        .unwrap();
        let worker = service.clone();
        let (started, waiting) = std::sync::mpsc::channel();
        let worker_root = root.path().to_path_buf();
        let status = std::thread::spawn(move || {
            started.send(()).unwrap();
            source_status(&worker, &worker_root)
        });
        waiting.recv().unwrap();
        murmur_core::completion::save_source(
            root.path(),
            "dictation-settings",
            r#"{"language":"fr"}"#,
        )
        .unwrap();
        drop(guard);
        assert_eq!(
            status.join().unwrap().unwrap().values["dictation-settings"]["language"],
            "fr"
        );
        let disk: serde_json::Value =
            serde_json::from_slice(&std::fs::read(root.path().join("settings.json")).unwrap())
                .unwrap();
        assert_eq!(disk["language"], "fr");
    }
}

/// Worker-only final barrier. The service stays retained in managed state even
/// if the coordinator stops waiting, so no joining destructor reaches main.
pub(crate) fn shutdown_flush(app: &tauri::AppHandle) -> murmur_core::shutdown::WriteOutcome {
    let Some(state) = app.try_state::<Completion>() else {
        return murmur_core::shutdown::WriteOutcome::UnpreservedFailure;
    };
    let _warning_notification = crate::commands::settings_store::SettingsWarningNotification(app);
    // Seal/drain without WRITE_LOCK: queued lifecycle retries acquire it inside
    // the core owner. Reconcile only after that owner has returned its receipt.
    let drain = || tauri::async_runtime::block_on(state.service.shutdown_receipt());
    match crate::commands::settings_store::data_dir(app) {
        Ok(root) => crate::commands::settings_store::reconcile_lifecycle_after(&root, drain),
        Err(_) => drain(),
    }
}

#[tauri::command]
pub(crate) async fn webkit_state_snapshot(
    app: tauri::AppHandle,
) -> Result<murmur_core::completion::lifecycle::Receipt, String> {
    wait_ready(&app).await?;
    let admission = crate::shutdown::WriterPermit::acquire()?;
    crate::shutdown::source_call(move || {
        let _admission = admission;
        webkit_state_snapshot_worker(app)
    })
    .await
    .map_err(|_| "Source writer unavailable".to_owned())?
}

#[tauri::command]
pub(crate) async fn import_webkit_state(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    source: std::collections::BTreeMap<String, Option<String>>,
) -> Result<murmur_core::completion::lifecycle::Receipt, String> {
    wait_ready(&app).await?;
    let admission = crate::shutdown::WriterPermit::acquire()?;
    crate::shutdown::source_call(move || {
        let _admission = admission;
        import_webkit_state_worker(window, app, source)
    })
    .await
    .map_err(|_| "Source writer unavailable".to_owned())?
}

#[tauri::command]
pub(crate) async fn save_webkit_source(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    key: String,
    value: serde_json::Value,
    intent_generation: u64,
    source_session: String,
) -> Result<murmur_core::source_order::SourceWrite<()>, String> {
    let admission = crate::shutdown::WriterPermit::acquire()?;
    crate::shutdown::versioned_source_call(
        window.label().into(),
        source_session,
        key.clone(),
        intent_generation,
        move || {
            let _admission = admission;
            save_webkit_source_worker(window, app, key, value)
        },
    )
    .await
}
