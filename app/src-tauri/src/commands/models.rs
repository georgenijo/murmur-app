use crate::{MutexExt, State};
use crate::transcriber::{self, TranscriptionBackend};
use crate::vad;
use tauri::Emitter;

#[tauri::command]
pub fn check_model_exists(state: tauri::State<'_, State>) -> bool {
    let backend = state.app_state.backend.lock_or_recover();
    backend.model_exists()
}

#[tauri::command]
pub fn check_specific_model_exists(model_name: String) -> bool {
    // Reject path traversal or absolute paths in untrusted input
    if model_name.contains("..") || model_name.contains('/') || model_name.contains('\\') {
        return false;
    }
    if transcriber::is_coreml_model(&model_name) {
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        return transcriber::coreml::specific_model_exists(&model_name);
        #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
        return false;
    }
    // --- Parakeet backend (removable): delete this branch to remove. ---
    if transcriber::parakeet::is_parakeet_model(&model_name) {
        return transcriber::parakeet::specific_model_exists(&model_name);
    }
    transcriber::whisper::specific_model_exists(&model_name)
}

#[tauri::command]
pub async fn download_model(app_handle: tauri::AppHandle, model_name: String) -> Result<(), String> {
    const ALLOWED_MODELS: &[&str] = &[
        "large-v3-turbo", "small.en", "base.en", "tiny.en", "medium.en",
    ];
    let is_coreml = transcriber::is_coreml_model(&model_name);
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    if is_coreml {
        return Err(
            "Core ML transcription is available only on macOS 14 or newer with Apple Silicon"
                .to_string(),
        );
    }
    // --- Parakeet backend (removable): delete this branch + download_parakeet_model to remove. ---
    let is_parakeet = transcriber::parakeet::is_parakeet_model(&model_name);
    if is_coreml {
        // The explicit Core ML value is also prefixed with "parakeet"; classify
        // it before the broad sherpa sentinel.
    } else if is_parakeet {
        if transcriber::parakeet::download_spec(&model_name).is_none() {
            return Err(format!("Unknown Parakeet model '{}'", model_name));
        }
    } else if !ALLOWED_MODELS.contains(&model_name.as_str()) {
        return Err(format!("Unknown model '{}'. Allowed: {}", model_name, ALLOWED_MODELS.join(", ")));
    }

    // Whisper and sherpa share Murmur's models directory. FluidAudio owns a
    // separate Application Support cache, but VAD must still land here.
    let models_dir = transcriber::WhisperBackend::new().models_dir()?;
    tokio::fs::create_dir_all(&models_dir)
        .await
        .map_err(|e| format!("Failed to create models directory: {}", e))?;

    if is_coreml {
        let _ = app_handle.emit("download-progress", serde_json::json!({
            "received": 0,
            "total": 0
        }));
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            let model_name = model_name.clone();
            tokio::task::spawn_blocking(move || transcriber::coreml::prepare_model(&model_name))
                .await
                .map_err(|error| format!("Core ML setup task failed: {error}"))??;
        }
        let _ = app_handle.emit("download-progress", serde_json::json!({
            "received": 1,
            "total": 1
        }));
    } else if is_parakeet {
        download_parakeet_model(&app_handle, &model_name, &models_dir).await?;
    } else {
        download_whisper_model(&app_handle, &model_name, &models_dir).await?;
    }

    // Co-download VAD model alongside the transcription model (~1.8MB)
    if !vad::vad_model_exists() {
        let vad_dest = models_dir.join(vad::VAD_MODEL_FILENAME);
        let vad_tmp = models_dir.join(format!("{}.tmp", vad::VAD_MODEL_FILENAME));
        match stream_download(&app_handle, vad::VAD_MODEL_URL, &vad_tmp).await {
            Ok(bytes) => {
                if let Err(e) = tokio::fs::rename(&vad_tmp, &vad_dest).await {
                    let _ = tokio::fs::remove_file(&vad_tmp).await;
                    tracing::warn!(target: "system", "Failed to finalize VAD model download: {}", e);
                } else {
                    tracing::info!(target: "system", "VAD model co-downloaded: {} ({} bytes)", vad::VAD_MODEL_FILENAME, bytes);
                }
            }
            Err(e) => {
                tracing::warn!(target: "system", "VAD model co-download failed (non-fatal): {}", e);
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specific_model_check_rejects_paths() {
        assert!(!check_specific_model_exists("../base.en".to_string()));
        assert!(!check_specific_model_exists("models/base.en".to_string()));
        assert!(!check_specific_model_exists("models\\base.en".to_string()));
    }

    #[test]
    fn coreml_model_is_not_dispatched_as_sherpa_download() {
        assert!(transcriber::is_coreml_model(transcriber::COREML_MODEL_NAME));
        assert!(transcriber::parakeet::is_parakeet_model(
            transcriber::COREML_MODEL_NAME
        ));
    }
}

/// Download a single whisper ggml .bin file from Hugging Face.
async fn download_whisper_model(
    app_handle: &tauri::AppHandle,
    model_name: &str,
    models_dir: &std::path::Path,
) -> Result<(), String> {
    let filename = format!("ggml-{}.bin", model_name);
    let url = format!(
        "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}",
        filename
    );
    let dest_path = models_dir.join(&filename);
    let temp_path = models_dir.join(format!("{}.tmp", filename));

    let received = stream_download(app_handle, &url, &temp_path).await?;

    tokio::fs::rename(&temp_path, &dest_path)
        .await
        .map_err(|e| {
            let _ = std::fs::remove_file(&temp_path);
            format!("Failed to finalize download: {}", e)
        })?;

    tracing::info!(target: "system", "Model downloaded: {} ({} bytes)", filename, received);
    Ok(())
}

/// --- Parakeet backend (removable): delete this fn to remove. ---
/// Download a Parakeet model bundle (.tar.bz2) and extract it into the models dir.
async fn download_parakeet_model(
    app_handle: &tauri::AppHandle,
    model_name: &str,
    models_dir: &std::path::Path,
) -> Result<(), String> {
    let (url, dir_name) = transcriber::parakeet::download_spec(model_name)
        .ok_or_else(|| format!("Unknown Parakeet model '{}'", model_name))?;
    let temp_path = models_dir.join(format!("{}.tar.bz2.tmp", dir_name));

    let received = stream_download(app_handle, &url, &temp_path).await?;

    // Decompress + untar on a blocking thread; archive unpacks to `<dir_name>/`.
    let temp_clone = temp_path.clone();
    let models_dir_owned = models_dir.to_path_buf();
    let extracted_dir = models_dir.join(&dir_name);
    let extraction_result = tokio::task::spawn_blocking(move || {
        // Remove any stale/partial bundle before extracting.
        let _ = std::fs::remove_dir_all(&extracted_dir);
        let file = std::fs::File::open(&temp_clone)
            .map_err(|e| format!("Failed to open archive: {}", e))?;
        let decompressor = bzip2::read::BzDecoder::new(file);
        let mut archive = tar::Archive::new(decompressor);
        archive.unpack(&models_dir_owned).map_err(|e| {
            let _ = std::fs::remove_dir_all(&extracted_dir);
            format!("Failed to extract archive: {}", e)
        })?;
        Ok::<(), String>(())
    })
    .await
    .map_err(|e| format!("Extraction task failed: {}", e))?;

    // Remove the temp archive regardless of extraction outcome.
    let _ = tokio::fs::remove_file(&temp_path).await;
    extraction_result?;

    tracing::info!(target: "system", "Parakeet model downloaded and extracted: {} ({} bytes)", dir_name, received);
    Ok(())
}

/// Ensure the VAD model is present, downloading it if necessary.
/// This is the fallback for users who have a transcription model but not the
/// VAD model (e.g. upgrade from a pre-VAD version or manual model install).
pub(crate) async fn ensure_vad_model(app_handle: &tauri::AppHandle) -> Result<(), String> {
    if vad::vad_model_exists() {
        return Ok(());
    }

    let model_path = vad::vad_model_path()
        .ok_or_else(|| "Could not determine VAD model path".to_string())?;
    let models_dir = model_path.parent()
        .ok_or_else(|| "Could not determine models directory".to_string())?;

    tokio::fs::create_dir_all(models_dir)
        .await
        .map_err(|e| format!("Failed to create models directory: {}", e))?;

    tracing::info!(target: "system", "VAD model not found, downloading...");

    let temp_path = models_dir.join(format!("{}.tmp", vad::VAD_MODEL_FILENAME));
    let received = stream_download(app_handle, vad::VAD_MODEL_URL, &temp_path).await?;

    tokio::fs::rename(&temp_path, &model_path)
        .await
        .map_err(|e| {
            let _ = std::fs::remove_file(&temp_path);
            format!("Failed to finalize VAD model download: {}", e)
        })?;

    tracing::info!(target: "system", "VAD model downloaded: {} ({} bytes)", vad::VAD_MODEL_FILENAME, received);
    Ok(())
}

/// Stream a file download with progress events. Returns total bytes received.
pub(crate) async fn stream_download(
    app_handle: &tauri::AppHandle,
    url: &str,
    dest: &std::path::Path,
) -> Result<u64, String> {
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .timeout(std::time::Duration::from_secs(15 * 60))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Download request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Download failed with status: {}", response.status()));
    }

    let total = response.content_length().unwrap_or(0);
    let mut received: u64 = 0;

    use tokio::io::AsyncWriteExt;
    let mut file = tokio::fs::File::create(dest)
        .await
        .map_err(|e| format!("Failed to create temp file: {}", e))?;

    let mut stream = response.bytes_stream();
    use futures_util::StreamExt;
    let stream_result = async {
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("Download error: {}", e))?;
            file.write_all(&chunk)
                .await
                .map_err(|e| format!("Failed to write to file: {}", e))?;
            received += chunk.len() as u64;
            let _ = app_handle.emit("download-progress", serde_json::json!({
                "received": received,
                "total": total
            }));
        }
        file.flush()
            .await
            .map_err(|e| format!("Failed to flush file: {}", e))?;
        Ok::<(), String>(())
    }.await;

    if let Err(e) = stream_result {
        let _ = tokio::fs::remove_file(dest).await;
        return Err(e);
    }

    Ok(received)
}
