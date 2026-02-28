use crate::{MutexExt, State};
use crate::transcriber::{self, TranscriptionBackend};
use crate::log_info;
use tauri::Emitter;

#[tauri::command]
pub fn check_model_exists(state: tauri::State<'_, State>) -> bool {
    let backend = state.app_state.backend.lock_or_recover();
    if backend.model_exists() {
        return true;
    }
    // Also check the other backend type so the model downloader screen
    // doesn't appear when a model from the other engine is already installed.
    if backend.name() == "whisper" {
        transcriber::MoonshineBackend::new().model_exists()
    } else {
        transcriber::WhisperBackend::new().model_exists()
    }
}

#[tauri::command]
pub fn check_specific_model_exists(model_name: String) -> bool {
    if transcriber::is_moonshine_model(&model_name) {
        let backend = transcriber::MoonshineBackend::new();
        let models_dir = match backend.models_dir() {
            Ok(d) => d,
            Err(_) => return false,
        };
        models_dir.join(transcriber::moonshine::model_dir_name(&model_name)).exists()
    } else {
        let backend = transcriber::WhisperBackend::new();
        let models_dir = match backend.models_dir() {
            Ok(d) => d,
            Err(_) => return false,
        };
        models_dir.join(format!("ggml-{}.bin", model_name)).exists()
    }
}

#[tauri::command]
pub async fn download_model(app_handle: tauri::AppHandle, model_name: String, state: tauri::State<'_, State>) -> Result<(), String> {
    const ALLOWED_MODELS: &[&str] = &[
        "large-v3-turbo", "small.en", "base.en", "tiny.en", "medium.en",
        "moonshine-tiny", "moonshine-base",
    ];
    if !ALLOWED_MODELS.contains(&model_name.as_str()) {
        return Err(format!("Unknown model '{}'. Allowed: {}", model_name, ALLOWED_MODELS.join(", ")));
    }

    let models_dir = state.app_state.backend.lock_or_recover().models_dir()?;
    tokio::fs::create_dir_all(&models_dir)
        .await
        .map_err(|e| format!("Failed to create models directory: {}", e))?;

    if transcriber::is_moonshine_model(&model_name) {
        download_moonshine_model(&app_handle, &model_name, &models_dir).await
    } else {
        download_whisper_model(&app_handle, &model_name, &models_dir).await
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

    log_info!("Model downloaded: {} ({} bytes)", filename, received);
    Ok(())
}

/// Download a moonshine model archive (tar.bz2) and extract it.
async fn download_moonshine_model(
    app_handle: &tauri::AppHandle,
    model_name: &str,
    models_dir: &std::path::Path,
) -> Result<(), String> {
    let archive_name = transcriber::moonshine::archive_filename(model_name);
    let url = transcriber::moonshine::download_url(model_name);
    let temp_path = models_dir.join(format!("{}.tmp", archive_name));

    let received = stream_download(app_handle, &url, &temp_path).await?;

    // Extract tar.bz2 archive on a blocking thread
    let temp_clone = temp_path.clone();
    let models_dir_owned = models_dir.to_path_buf();
    let dir_name = transcriber::moonshine::model_dir_name(model_name);
    let extracted_dir = models_dir.join(&dir_name);
    let extracted_dir_clone = extracted_dir.clone();
    let extraction_result = tokio::task::spawn_blocking(move || {
        let file = std::fs::File::open(&temp_clone)
            .map_err(|e| format!("Failed to open archive: {}", e))?;
        let decompressor = bzip2::read::BzDecoder::new(file);
        let mut archive = tar::Archive::new(decompressor);
        archive
            .unpack(&models_dir_owned)
            .map_err(|e| {
                // Clean up partially extracted directory
                let _ = std::fs::remove_dir_all(&extracted_dir_clone);
                format!("Failed to extract archive: {}", e)
            })?;
        Ok::<(), String>(())
    })
    .await
    .map_err(|e| format!("Extraction task failed: {}", e))?;

    // Clean up temp archive file regardless of extraction result
    let _ = tokio::fs::remove_file(&temp_path).await;

    extraction_result?;

    log_info!("Moonshine model downloaded and extracted: {} ({} bytes)", dir_name, received);
    Ok(())
}

/// Stream a file download with progress events. Returns total bytes received.
async fn stream_download(
    app_handle: &tauri::AppHandle,
    url: &str,
    dest: &std::path::Path,
) -> Result<u64, String> {
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
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
