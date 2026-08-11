use crate::meeting_capture::{
    render_meeting_text, MeetingCaptureConfig, MeetingRuntimeStatus, SystemAudioPermissionState,
};
use crate::meeting_store::{MeetingDetail, MeetingPage, MeetingSession, MeetingStoreStatus};
use crate::state::DictationStatus;
use crate::{MutexExt, State};
use serde::Deserialize;
use std::sync::atomic::Ordering;
use tauri::Emitter;
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartMeetingRequest {
    #[serde(default)]
    pub device_name: Option<String>,
    #[serde(default)]
    pub retain_audio: bool,
    #[serde(default)]
    pub retention_days: Option<u32>,
    #[serde(default = "default_max_sessions")]
    pub max_sessions: u32,
}

fn default_max_sessions() -> u32 {
    100
}

fn meeting_conflict(state: &State) -> Option<&'static str> {
    if state.app_state.meeting_active.load(Ordering::SeqCst) {
        return Some("A meeting is already active.");
    }
    if state
        .app_state
        .meeting_inference_active
        .load(Ordering::SeqCst)
    {
        return Some("Murmur is recovering an interrupted meeting transcript. Try again shortly.");
    }
    if crate::audio_lifecycle::is_audio_active() {
        return Some("Stop the active microphone recording before starting a meeting.");
    }
    if state.app_state.file_transcribing.load(Ordering::SeqCst) {
        return Some("Wait for file transcription to finish before starting a meeting.");
    }
    if state.benchmark.is_running() {
        return Some("Wait for the benchmark to finish before starting a meeting.");
    }
    if state.app_state.transform_status().blocks_recording()
        || state.transform_runtime.is_transform_busy()
    {
        return Some("Finish or cancel the active text transform before starting a meeting.");
    }
    if state.app_state.dictation.lock_or_recover().status != DictationStatus::Idle {
        return Some("Wait for dictation to finish before starting a meeting.");
    }
    None
}

#[tauri::command]
pub async fn start_meeting(
    app: tauri::AppHandle,
    request: StartMeetingRequest,
    state: tauri::State<'_, State>,
) -> Result<MeetingSession, String> {
    let _transition = state.app_state.recording_transition.lock().await;
    if let Some(error) = meeting_conflict(&state) {
        return Err(error.to_string());
    }
    if crate::keyboard::is_app_disabled() {
        return Err("Enable Murmur before starting a meeting.".to_string());
    }

    let (model_name, language, vad_sensitivity, smart_punctuation) = {
        let dictation = state.app_state.dictation.lock_or_recover();
        (
            dictation.model_name.clone(),
            dictation.language.clone(),
            dictation.vad_sensitivity,
            dictation.smart_punctuation,
        )
    };
    if !crate::model_runtime::model_installed(&model_name) {
        return Err(
            "Install the selected transcription model before starting a meeting.".to_string(),
        );
    }
    if crate::vad::is_enabled(vad_sensitivity) && !crate::vad::vad_model_exists() {
        return Err(
            "Install the Voice Activity Detection model before starting a meeting.".to_string(),
        );
    }

    let repository = state.meeting_store.repository()?;
    let _ = repository.prune(
        request.retention_days.map(|days| days.clamp(1, 3650)),
        request.max_sessions.clamp(1, 10_000),
    );
    let generation = state.app_state.next_meeting_generation();
    let session_id = Uuid::new_v4().to_string();
    let session = repository.create_session(
        &session_id,
        &model_name,
        &language,
        smart_punctuation,
        request.retain_audio,
    )?;
    state.app_state.meeting_active.store(true, Ordering::SeqCst);
    state
        .app_state
        .meeting_inference_active
        .store(true, Ordering::SeqCst);
    state.transform_runtime.shutdown();
    let config = MeetingCaptureConfig {
        generation,
        session_id: session_id.clone(),
        vad_sensitivity,
        device_id: request
            .device_name
            .filter(|device| device != "system_default"),
    };
    if let Err(error) = state.meetings.start(app, repository.clone(), config) {
        state
            .app_state
            .meeting_active
            .store(false, Ordering::SeqCst);
        state
            .app_state
            .meeting_inference_active
            .store(false, Ordering::SeqCst);
        let _ = repository.finish_session(
            &session_id,
            crate::meeting_store::MeetingSessionStatus::Failed,
            Some("supervisor_unavailable"),
        );
        return Err(error);
    }
    Ok(session)
}

#[tauri::command]
pub async fn stop_meeting(
    app: tauri::AppHandle,
    state: tauri::State<'_, State>,
) -> Result<(), String> {
    let _transition = state.app_state.recording_transition.lock().await;
    state.meetings.stop(&app)
}

#[tauri::command]
pub fn get_meeting_status(state: tauri::State<'_, State>) -> MeetingRuntimeStatus {
    state.meetings.status()
}

#[tauri::command]
pub fn get_system_audio_permission_status(
    state: tauri::State<'_, State>,
) -> SystemAudioPermissionState {
    state.meetings.permission_status()
}

#[tauri::command]
pub async fn request_system_audio_permission(
    app: tauri::AppHandle,
    state: tauri::State<'_, State>,
) -> Result<SystemAudioPermissionState, String> {
    {
        let _transition = state.app_state.recording_transition.lock().await;
        if let Some(error) = meeting_conflict(&state) {
            return Err(error.to_string());
        }
        state.app_state.meeting_active.store(true, Ordering::SeqCst);
    }
    let coordinator = state.meetings.clone();
    let joined = tauri::async_runtime::spawn_blocking(move || coordinator.request_permission())
        .await
        .map_err(|_| "The System Audio permission request stopped unexpectedly.".to_string());
    state
        .app_state
        .meeting_active
        .store(false, Ordering::SeqCst);
    let result = joined?;
    if let Ok(status) = result.as_ref() {
        let _ = app.emit("system-audio-permission-changed", status);
    }
    result
}

#[tauri::command]
pub fn get_meeting_store_status(state: tauri::State<'_, State>) -> MeetingStoreStatus {
    state.meeting_store.status()
}

#[tauri::command]
pub fn list_meetings(
    query: Option<String>,
    offset: Option<u64>,
    limit: Option<u32>,
    state: tauri::State<'_, State>,
) -> Result<MeetingPage, String> {
    state.meeting_store.repository()?.list_sessions(
        query.as_deref(),
        offset.unwrap_or(0),
        limit.unwrap_or(25),
    )
}

#[tauri::command]
pub fn get_meeting(id: String, state: tauri::State<'_, State>) -> Result<MeetingDetail, String> {
    state.meeting_store.repository()?.detail(id.trim())
}

#[tauri::command]
pub fn get_meeting_export_text(
    id: String,
    state: tauri::State<'_, State>,
) -> Result<String, String> {
    let detail = state.meeting_store.repository()?.detail(id.trim())?;
    Ok(render_meeting_text(&detail.segments))
}

#[tauri::command]
pub fn delete_meeting(id: String, state: tauri::State<'_, State>) -> Result<(), String> {
    let id = id.trim();
    if state.meetings.status().session_id.as_deref() == Some(id) && state.meetings.is_active() {
        return Err("Stop this meeting before deleting it.".to_string());
    }
    state.meeting_store.repository()?.delete_session(id)
}

#[tauri::command]
pub fn delete_all_meetings(state: tauri::State<'_, State>) -> Result<(), String> {
    if state.meetings.is_active() {
        return Err("Stop the active meeting before deleting meeting history.".to_string());
    }
    state.meeting_store.repository()?.delete_all()
}

#[tauri::command]
pub fn prune_meetings(
    retention_days: Option<u32>,
    max_sessions: u32,
    state: tauri::State<'_, State>,
) -> Result<u64, String> {
    state.meeting_store.repository()?.prune(
        retention_days.map(|days| days.clamp(1, 3650)),
        max_sessions.clamp(1, 10_000),
    )
}
