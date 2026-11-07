//! Startup file work is never joined by setup/main. Each owner retains a
//! shutdown writer permit; the independent coordinator deadlines bound waiting.
use crate::host::ServiceAccess;
use crate::{
    commands, diarization_audio, knowledge_store, legacy_query_cleanup, legacy_transform_cleanup,
    telemetry,
};

pub(crate) fn spawn<R: crate::host::Runtime>(
    app: &crate::host::Context<R>,
    name: &str,
    work: impl FnOnce(crate::host::Context<R>) + Send + 'static,
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

pub(crate) fn initialize(app: &crate::host::Context) -> Result<(), String> {
    // Path resolution only: no create, exists, read, migration or cleanup here.
    let root = app
        .path()
        .app_data_dir()
        .map_err(|_| "App-data path unavailable")?;
    let health_generation = app.services().capture_health.generation();
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
        if let Err(error) = app.services().capture_health.initialize_at(
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
            .services()
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
            .services()
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
        let knowledge_status = app.services().knowledge.initialize(knowledge_root);
        if knowledge_status.availability != knowledge_store::StoreAvailability::Unavailable {
            if let Err(error) = commands::knowledge::refresh_correction_rules(&app.services()) {
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
        let meeting_status = app.services().meeting_store.initialize(meeting_root);
        tracing::info!(
            target: "meeting",
            availability = ?meeting_status.availability,
            schema_version = meeting_status.schema_version,
            session_count = meeting_status.session_count,
            pending_segment_count = meeting_status.pending_segment_count,
            "meeting transcript store initialized"
        );
        if let Ok(repository) = app.services().meeting_store.repository() {
            app.services()
                .meetings
                .recover_pending(app.clone(), repository);
        }
    })?;
    Ok(())
}
