use crate::audio_lifecycle::{self, AudioCancelReason, AudioLifecycleEvent};
use crate::microphone_preview::{MicrophonePreviewStatus, PreviewPhase};
use crate::state::DictationStatus;
use crate::{keyboard, MutexExt, State};
use std::time::Duration;
use tauri::{Emitter, Manager};

const CONFIRMED_STOP_TIMEOUT: Duration = Duration::from_secs(15);

fn require_main_window(window: &tauri::WebviewWindow) -> Result<(), String> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err("Microphone testing is available only from the main Settings window".to_string())
    }
}

fn emit_status(app_handle: &tauri::AppHandle) {
    let status = app_handle
        .state::<State>()
        .app_state
        .microphone_preview
        .status();
    let _ = app_handle.emit_to("main", "microphone-preview-status", status);
}

fn normalized_device_id(device_id: String) -> Result<Option<String>, String> {
    if device_id.len() > 4_096 {
        return Err("The selected microphone identifier is invalid".to_string());
    }
    if device_id == "system_default" {
        Ok(None)
    } else if device_id.is_empty() {
        Err("Choose a microphone before starting the test".to_string())
    } else {
        Ok(Some(device_id))
    }
}

pub(crate) fn handle_audio_lifecycle(
    app_handle: tauri::AppHandle,
    preview_id: u64,
    event: AudioLifecycleEvent,
) {
    let state = app_handle.state::<State>();
    let preview = &state.app_state.microphone_preview;
    let changed = match event {
        AudioLifecycleEvent::Ready => preview.set_phase_if(preview_id, PreviewPhase::Active),
        AudioLifecycleEvent::StillConnecting => preview.set_still_connecting_if(preview_id),
        AudioLifecycleEvent::Recovering { .. } => {
            preview.set_phase_if(preview_id, PreviewPhase::Stopping)
        }
        AudioLifecycleEvent::InitializationFailed { error, kind } => {
            preview.set_error_if(preview_id, kind.as_str(), error)
        }
        AudioLifecycleEvent::RecoveryStalled => preview.set_error_if(
            preview_id,
            "stop_timeout",
            "Microphone cleanup is taking longer than expected. Wait for it to finish before trying again.",
        ),
        AudioLifecycleEvent::Interrupted { .. } => preview.set_error_if(
            preview_id,
            "stream_interrupted",
            "The microphone stopped unexpectedly. Check the selected input and try again.",
        ),
        AudioLifecycleEvent::Idle => preview.clear_if(preview_id),
    };
    if changed {
        emit_status(&app_handle);
    }
}

#[tauri::command]
pub fn get_microphone_preview_status(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, State>,
) -> Result<MicrophonePreviewStatus, String> {
    require_main_window(&window)?;
    Ok(state.app_state.microphone_preview.status())
}

#[tauri::command]
pub async fn start_microphone_preview(
    window: tauri::WebviewWindow,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, State>,
    device_id: String,
) -> Result<MicrophonePreviewStatus, String> {
    require_main_window(&window)?;
    let device_id = normalized_device_id(device_id)?;
    let transition = state.app_state.recording_transition.lock().await;
    {
        let dictation = state.app_state.dictation.lock_or_recover();
        if dictation.status != DictationStatus::Idle {
            return Err("Finish the current dictation before testing the microphone".to_string());
        }
        if state.app_state.transform_status().blocks_recording()
            || state.transform_runtime.is_transform_busy()
        {
            return Err("Finish the current transform before testing the microphone".to_string());
        }
        if state.query.status().blocks_pipeline() {
            return Err("Finish the current voice query before testing the microphone".to_string());
        }
        if state.benchmark.is_running() {
            return Err("Finish the current benchmark before testing the microphone".to_string());
        }
        #[cfg(feature = "internal-benchmark")]
        if state.corpus.is_active() {
            return Err(
                "Finish the personal corpus recording before testing the microphone".to_string(),
            );
        }
        if keyboard::is_app_disabled() {
            return Err("Enable Murmur before testing the microphone".to_string());
        }
        if state.app_state.microphone_preview.is_active() {
            return Err("A microphone test is already active".to_string());
        }
        if audio_lifecycle::is_audio_active() {
            return Err(
                "The microphone is still in use or recovering. Wait for Murmur to become ready."
                    .to_string(),
            );
        }
    }
    let preview_id = state.app_state.microphone_preview.claim()?;
    tracing::info!(
        target: "audio",
        event_code = "audio.preview_started",
        preview_id,
        device_selection = if device_id.is_some() { "explicit" } else { "system_default" },
        "microphone preview requested"
    );
    emit_status(&app_handle);

    // The preview claim blocks competing starts. Do not hold the transition
    // mutex while waiting for the supervisor's acceptance channel.
    drop(transition);
    let start_handle = app_handle.clone();
    let start_result = match tokio::task::spawn_blocking(move || {
        audio_lifecycle::start_preview_recording(start_handle, device_id, preview_id)
    })
    .await
    {
        Ok(result) => result,
        Err(error) => {
            let message = format!("Microphone test task failed: {error}");
            state.app_state.microphone_preview.fail_and_clear(
                preview_id,
                "start_failed",
                message.clone(),
            );
            emit_status(&app_handle);
            return Err(message);
        }
    };
    if let Err(error) = start_result {
        state.app_state.microphone_preview.fail_and_clear(
            preview_id,
            "start_failed",
            error.to_string(),
        );
        emit_status(&app_handle);
        return Err(error.to_string());
    }
    Ok(state.app_state.microphone_preview.status())
}

#[tauri::command]
pub async fn stop_microphone_preview(
    window: tauri::WebviewWindow,
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, State>,
    preview_id: u64,
) -> Result<MicrophonePreviewStatus, String> {
    require_main_window(&window)?;
    let transition = state.app_state.recording_transition.lock().await;
    if !state.app_state.microphone_preview.is_current(preview_id) {
        return Ok(state.app_state.microphone_preview.status());
    }
    state
        .app_state
        .microphone_preview
        .set_phase_if(preview_id, PreviewPhase::Stopping);
    emit_status(&app_handle);

    // Like dictation stop, release the short transition lock before any
    // supervisor wait. The active preview claim keeps racing starts blocked.
    drop(transition);
    let stop_result =
        tokio::task::spawn_blocking(move || audio_lifecycle::stop_preview_recording(preview_id))
            .await;
    if let Err(error) = stop_result {
        state.app_state.microphone_preview.set_error_if(
            preview_id,
            "stop_failed",
            format!("Microphone test stop task failed: {error}"),
        );
        emit_status(&app_handle);
        return Err("The microphone test could not be stopped safely".to_string());
    }

    // Stop during Connecting only begins recovery and acknowledges promptly;
    // Stop during Stopping/Recovering may report an expected in-progress error.
    // In every case the Preview Idle lifecycle event is the teardown authority.
    if !state
        .app_state
        .microphone_preview
        .wait_until_inactive(preview_id, CONFIRMED_STOP_TIMEOUT)
        .await
    {
        state.app_state.microphone_preview.set_error_if(
            preview_id,
            "stop_timeout",
            "Microphone cleanup is taking longer than expected. Wait for it to finish before trying again.",
        );
        emit_status(&app_handle);
        return Err(
            "Microphone cleanup is still in progress; a new input was not opened".to_string(),
        );
    }
    tracing::info!(
        target: "audio",
        event_code = "audio.preview_stopped",
        preview_id,
        "microphone preview stopped with confirmed worker teardown"
    );
    Ok(state.app_state.microphone_preview.status())
}

fn resolve_cancel_id(current: Option<u64>, requested: Option<u64>) -> Option<u64> {
    match (current, requested) {
        (Some(current), Some(requested)) if current == requested => Some(current),
        (Some(current), None) => Some(current),
        _ => None,
    }
}

async fn cancel_exact_preview(
    app_handle: tauri::AppHandle,
    requested_id: Option<u64>,
) -> Result<bool, String> {
    let state = app_handle.state::<State>();
    let transition = state.app_state.recording_transition.lock().await;
    let Some(preview_id) = resolve_cancel_id(
        state.app_state.microphone_preview.current_id(),
        requested_id,
    ) else {
        return Ok(false);
    };
    state
        .app_state
        .microphone_preview
        .set_phase_if(preview_id, PreviewPhase::Stopping);
    emit_status(&app_handle);
    drop(transition);
    tokio::task::spawn_blocking(move || {
        audio_lifecycle::cancel_preview_capture(preview_id, AudioCancelReason::User)
    })
    .await
    .map_err(|error| format!("Microphone test cancel task failed: {error}"))?
}

#[tauri::command]
pub async fn cancel_microphone_preview(
    window: tauri::WebviewWindow,
    app_handle: tauri::AppHandle,
    preview_id: Option<u64>,
) -> Result<bool, String> {
    require_main_window(&window)?;
    cancel_exact_preview(app_handle, preview_id).await
}

pub(crate) fn cancel_for_window_close(app_handle: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let _ = cancel_exact_preview(app_handle, None).await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optional_cleanup_never_becomes_an_ownerless_audio_cancel() {
        assert_eq!(resolve_cancel_id(None, None), None);
        assert_eq!(resolve_cancel_id(None, Some(1)), None);
        assert_eq!(resolve_cancel_id(Some(7), Some(6)), None);
        assert_eq!(resolve_cancel_id(Some(7), Some(7)), Some(7));
        assert_eq!(resolve_cancel_id(Some(7), None), Some(7));
    }

    #[test]
    fn missing_explicit_ids_are_not_rewritten_to_system_default() {
        assert_eq!(normalized_device_id("system_default".into()).unwrap(), None);
        assert_eq!(
            normalized_device_id("missing-stable-id".into()).unwrap(),
            Some("missing-stable-id".into())
        );
    }
}
