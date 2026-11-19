//! The IPC wrappers of the four dictation commands, **compiled only with the
//! `dictation-shadow` feature** (D-S2c-8 f, amendment 1).
//!
//! Each wrapper is a new `#[tauri::command]` registered under the same IPC name (the
//! `rename` attribute; its Rust name differs, so the macros Tauri generates for the
//! original command and for the wrapper do not collide) that **calls** the original command and hands the shadow the reply that command returned to IPC
//! ([`commanded`]): a decision with a reply concludes with the kind of that value, not
//! the kind it decided. The original commands are untouched; `lib.rs` registers these
//! instead of them in a shadow build.
//!
//! The reply sink is a tokio task-local scoped here, once per invocation. Every access
//! comes after an active-flight check; no spawned task reads it (a task-local is not
//! inherited by `tokio::spawn` or `spawn_blocking`).

use super::{audio_lifecycle, SmartAutoRequest, State};
use crate::dictation_executor::shadow::commanded;

#[tauri::command(rename = "start_native_recording")]
pub(crate) async fn start_native_recording_witnessed(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, State>,
    device_name: Option<String>,
    smart_auto: Option<SmartAutoRequest>,
    origin: Option<String>,
    gesture_token: Option<String>,
) -> Result<serde_json::Value, String> {
    commanded(super::start_native_recording(
        app_handle,
        state,
        device_name,
        smart_auto,
        origin,
        gesture_token,
    ))
    .await
}

#[tauri::command(rename = "stop_native_recording")]
pub(crate) async fn stop_native_recording_witnessed(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, State>,
    stop_trigger: Option<audio_lifecycle::DictationStopTrigger>,
) -> Result<serde_json::Value, String> {
    commanded(super::stop_native_recording(
        app_handle,
        state,
        stop_trigger,
    ))
    .await
}

#[tauri::command(rename = "cancel_native_recording")]
pub(crate) async fn cancel_native_recording_witnessed(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, State>,
    expected_recording_id: Option<u64>,
) -> Result<(), String> {
    commanded(super::cancel_native_recording(
        app_handle,
        state,
        expected_recording_id,
    ))
    .await
}

#[tauri::command(rename = "process_audio")]
pub(crate) async fn process_audio_witnessed(
    app_handle: tauri::AppHandle,
    audio_data: String,
    state: tauri::State<'_, State>,
) -> Result<serde_json::Value, String> {
    commanded(super::process_audio(app_handle, audio_data, state)).await
}
