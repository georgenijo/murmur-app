//! Startup file work is never joined by setup/main. Each owner retains a
//! shutdown writer permit; the independent coordinator deadlines bound waiting.
use crate::{
    commands, diarization_audio, knowledge_store, legacy_query_cleanup, legacy_transform_cleanup,
    telemetry, State,
};
use tauri::Manager;

pub(crate) fn spawn<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    name: &str,
    work: impl FnOnce(tauri::AppHandle<R>) + Send + 'static,
) -> Result<(), String> {
    let app = app.clone();
    let permit = crate::shutdown::WriterPermit::accepted_producer();
    std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            let _permit = permit;
            work(app);
        })
        .map(|_| ())
        .map_err(|_| "Startup worker unavailable".to_owned())
}

pub(crate) fn initialize(app: &tauri::AppHandle) -> Result<(), String> {
    // Path resolution only: no create, exists, read, migration or cleanup here.
    let root = app
        .path()
        .app_data_dir()
        .map_err(|_| "App-data path unavailable")?;
    let health_generation = app.state::<State>().capture_health.generation();
    let app_data_root = root.clone();
    spawn(app, "legacy-cleanup", move |_app| {
        if legacy_query_cleanup::cleanup(&app_data_root).is_err() {
            tracing::warn!(target: "system", "legacy conversation cleanup failed");
        }
        if legacy_transform_cleanup::cleanup(&app_data_root).is_err() {
            tracing::warn!(target: "system", "legacy transform capture cleanup failed");
        }
    })?;
    let app_data_root = root.clone();
    spawn(app, "capture-health-init", move |app| {
        let performance_root = app_data_root.join("diagnostics");
        if let Err(error) = app.state::<State>().capture_health.initialize_at(
            performance_root.clone(),
            &telemetry::event_jsonl_paths(),
            health_generation,
        ) {
            tracing::warn!(
                target: "system",
                diagnostics_available = false,
                "capture-health diagnostics store unavailable: {}",
                error
            );
        }
    })?;
    let app_data_root = root.clone();
    spawn(app, "diagnostics-init", move |app| {
        let performance_root = app_data_root.join("diagnostics");
        if let Err(error) = app
            .state::<State>()
            .performance
            .initialize(performance_root.clone(), Some(app.clone()))
        {
            tracing::warn!(
                target: "system",
                diagnostics_available = false,
                "performance diagnostics store unavailable: {}",
                error
            );
        }
        if app
            .state::<State>()
            .dictation_diagnostics
            .initialize(performance_root.join("dictation-captures"))
            .is_err()
        {
            tracing::warn!(
                target: "system",
                diagnostics_available = false,
                "dictation diagnostic capture store unavailable"
            );
        }
    })?;
    let app_data_root = root.clone();
    spawn(app, "knowledge-init", move |app| {
        let knowledge_root = app_data_root.join("knowledge");
        let knowledge_status = app.state::<State>().knowledge.initialize(knowledge_root);
        if knowledge_status.availability != knowledge_store::StoreAvailability::Unavailable {
            if let Err(error) = commands::knowledge::refresh_correction_rules(&app.state::<State>())
            {
                tracing::warn!(target: "system", error, "initial knowledge correction matcher refresh failed");
            }
        }
        tracing::info!(
            target: "system",
            availability = ?knowledge_status.availability,
            schema_version = knowledge_status.schema_version,
            record_count = knowledge_status.record_count,
            "personal knowledge store initialized"
        );
    })?;
    let app_data_root = root;
    spawn(app, "meeting-init", move |app| {
        let meeting_root = app_data_root.join("meetings");
        diarization_audio::sweep_abandoned(&meeting_root);
        let meeting_status = app.state::<State>().meeting_store.initialize(meeting_root);
        tracing::info!(
            target: "meeting",
            availability = ?meeting_status.availability,
            schema_version = meeting_status.schema_version,
            session_count = meeting_status.session_count,
            pending_segment_count = meeting_status.pending_segment_count,
            "meeting transcript store initialized"
        );
        if let Ok(repository) = app.state::<State>().meeting_store.repository() {
            app.state::<State>()
                .meetings
                .recover_pending(app.clone(), repository);
        }
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn quit_dispatch_remains_available_while_startup_reader_is_stalled() {
        use murmur_core::shutdown::{Action, Coordinator, Request};
        use std::{
            sync::{mpsc, Mutex},
            time::Duration,
        };
        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let (entered, reading) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        let (done, finished) = mpsc::channel();
        super::spawn(app.handle(), "synthetic-stalled-startup", move |_| {
            #[cfg(target_os = "macos")]
            assert!(objc2::MainThreadMarker::new().is_none());
            entered.send(()).unwrap();
            resume.recv_timeout(Duration::from_secs(5)).unwrap();
            done.send(()).unwrap();
        })
        .unwrap();
        reading.recv_timeout(Duration::from_secs(2)).unwrap();
        // Setup already returned; the same dispatcher can accept Quit and
        // the independent deadline adapter can issue its terminal callback.
        let core = Mutex::new(Coordinator::default());
        let (send, receive) = mpsc::channel();
        crate::shutdown::adapter::accept(&core, &send, Request::Quit, 0);
        crate::shutdown::adapter::reduce(&core, Ok(receive.recv().unwrap()), 0);
        crate::shutdown::adapter::reduce(&core, Err(mpsc::RecvTimeoutError::Timeout), 10_000);
        assert_eq!(core.lock().unwrap().issue(), Some(Action::Quit));
        assert!(finished.try_recv().is_err());
        release.send(()).unwrap();
        finished.recv_timeout(Duration::from_secs(2)).unwrap();
    }
}
