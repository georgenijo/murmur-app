//! Streaming speech detection model commands (workstream E1).
//!
//! The pinned Silero ONNX model is fetched only when the user asks: switching
//! "Streaming speech detection" on, or pressing Download / Retry. Starting a
//! recording never touches the network. These mirror the Smart Turn model
//! commands (`commands/smart_turn.rs`).

use crate::vad_stream::{self, ModelStatus};

#[tauri::command]
pub async fn streaming_vad_model_status() -> ModelStatus {
    tokio::task::spawn_blocking(vad_stream::model_status)
        .await
        .unwrap_or_else(|_| ModelStatus {
            state: vad_stream::ModelState::Failed,
            size_bytes: vad_stream::MODEL_SIZE_BYTES,
            error: Some("Status check failed".to_string()),
        })
}

#[tauri::command]
pub async fn download_streaming_vad_model() -> Result<(), String> {
    vad_stream::download_model().await
}

#[tauri::command]
pub async fn remove_streaming_vad_model() -> Result<(), String> {
    vad_stream::remove_model().await
}
