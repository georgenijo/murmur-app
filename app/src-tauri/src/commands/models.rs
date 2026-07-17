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
            "total": 0,
            "phase": "installing"
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
            "total": 1,
            "phase": "installing"
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
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_dir(label: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "murmur-{label}-{}-{nonce}",
            std::process::id()
        ))
    }

    fn write_parakeet_archive(
        archive_path: &std::path::Path,
        dir_name: &str,
        complete: bool,
    ) {
        let source_root = archive_path.with_extension("source");
        let bundle = source_root.join(dir_name);
        fs::create_dir_all(&bundle).unwrap();
        fs::write(bundle.join("encoder.fp16.onnx"), b"encoder").unwrap();
        fs::write(bundle.join("decoder.fp16.onnx"), b"decoder").unwrap();
        if complete {
            fs::write(bundle.join("joiner.fp16.onnx"), b"joiner").unwrap();
        }
        fs::write(bundle.join("tokens.txt"), b"tokens").unwrap();

        let file = fs::File::create(archive_path).unwrap();
        let encoder = bzip2::write::BzEncoder::new(file, bzip2::Compression::best());
        let mut archive = tar::Builder::new(encoder);
        archive.append_dir_all(dir_name, &bundle).unwrap();
        let encoder = archive.into_inner().unwrap();
        encoder.finish().unwrap();
        fs::remove_dir_all(source_root).unwrap();
    }

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

    #[test]
    fn parakeet_extraction_replaces_partial_bundle_only_after_validation() {
        let root = test_dir("parakeet-atomic-install");
        let models_dir = root.join("models");
        fs::create_dir_all(&models_dir).unwrap();
        let model_name = "parakeet-tdt-0.6b-v2-fp16";
        let (_, dir_name) = transcriber::parakeet::download_spec(model_name).unwrap();
        let partial = models_dir.join(&dir_name);
        fs::create_dir_all(&partial).unwrap();
        fs::write(partial.join("encoder.fp16.onnx"), b"partial").unwrap();
        let archive_path = root.join("model.tar.bz2");
        write_parakeet_archive(&archive_path, &dir_name, true);

        extract_parakeet_archive(&archive_path, &models_dir, model_name, &dir_name).unwrap();

        assert!(transcriber::parakeet::specific_model_exists_in(
            model_name,
            &models_dir
        ));
        assert_eq!(fs::read(partial.join("encoder.fp16.onnx")).unwrap(), b"encoder");
        assert!(!models_dir.join(format!(".{dir_name}.extracting")).exists());
        assert!(archive_path.exists(), "caller owns archive cleanup after success");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn incomplete_parakeet_archive_never_publishes_a_model() {
        let root = test_dir("parakeet-incomplete-install");
        let models_dir = root.join("models");
        fs::create_dir_all(&models_dir).unwrap();
        let model_name = "parakeet-tdt-0.6b-v2-fp16";
        let (_, dir_name) = transcriber::parakeet::download_spec(model_name).unwrap();
        let archive_path = root.join("model.tar.bz2");
        write_parakeet_archive(&archive_path, &dir_name, false);

        let error = extract_parakeet_archive(&archive_path, &models_dir, model_name, &dir_name)
            .unwrap_err();

        assert!(error.contains("incomplete"));
        assert!(!models_dir.join(&dir_name).exists());
        assert!(!models_dir.join(format!(".{dir_name}.extracting")).exists());
        fs::remove_dir_all(root).unwrap();
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
    if transcriber::parakeet::specific_model_exists_in(model_name, models_dir) {
        return Ok(());
    }

    let archive_path = models_dir.join(format!("{}.tar.bz2", dir_name));
    let legacy_temp_path = models_dir.join(format!("{}.tar.bz2.tmp", dir_name));

    // v0.16.0 could leave a complete `.tmp` archive when Murmur exited during
    // extraction. Try that expensive local data before downloading it again.
    if legacy_temp_path.is_file() {
        emit_installing(app_handle);
        let archive = legacy_temp_path.clone();
        let root = models_dir.to_path_buf();
        let model = model_name.to_string();
        let bundle = dir_name.clone();
        match tokio::task::spawn_blocking(move || {
            extract_parakeet_archive(&archive, &root, &model, &bundle)
        })
        .await
        .map_err(|e| format!("Extraction task failed: {}", e))?
        {
            Ok(()) => {
                let _ = tokio::fs::remove_file(&legacy_temp_path).await;
                tracing::info!(target: "system", "Recovered Parakeet installation from retained archive: {}", dir_name);
                return Ok(());
            }
            Err(error) => {
                tracing::warn!(target: "system", "Retained Parakeet archive was unusable; downloading again: {}", error);
                let _ = tokio::fs::remove_file(&legacy_temp_path).await;
            }
        }
    }

    // A finalized archive is retained when extraction fails (for example due
    // to low disk space), so Retry performs only the local install work.
    if !archive_path.is_file() {
        let download_path = models_dir.join(format!("{}.tar.bz2.download", dir_name));
        let received = stream_download(app_handle, &url, &download_path).await?;
        tokio::fs::rename(&download_path, &archive_path)
            .await
            .map_err(|e| {
                let _ = std::fs::remove_file(&download_path);
                format!("Failed to finalize Parakeet archive: {}", e)
            })?;
        tracing::info!(target: "system", "Parakeet archive downloaded: {} ({} bytes)", dir_name, received);
    }

    emit_installing(app_handle);
    let archive = archive_path.clone();
    let root = models_dir.to_path_buf();
    let model = model_name.to_string();
    let bundle = dir_name.clone();
    tokio::task::spawn_blocking(move || {
        extract_parakeet_archive(&archive, &root, &model, &bundle)
    })
    .await
    .map_err(|e| format!("Extraction task failed: {}", e))??;

    let _ = tokio::fs::remove_file(&archive_path).await;

    tracing::info!(target: "system", "Parakeet model installed: {}", dir_name);
    Ok(())
}

fn emit_installing(app_handle: &tauri::AppHandle) {
    let _ = app_handle.emit("download-progress", serde_json::json!({
        "received": 0,
        "total": 0,
        "phase": "installing"
    }));
}

fn extract_parakeet_archive(
    archive_path: &std::path::Path,
    models_dir: &std::path::Path,
    model_name: &str,
    dir_name: &str,
) -> Result<(), String> {
    let final_dir = models_dir.join(dir_name);
    let staging_root = models_dir.join(format!(".{}.extracting", dir_name));
    let staged_dir = staging_root.join(dir_name);

    let _ = std::fs::remove_dir_all(&staging_root);
    if final_dir.exists()
        && !transcriber::parakeet::specific_model_exists_in(model_name, models_dir)
    {
        std::fs::remove_dir_all(&final_dir)
            .map_err(|e| format!("Failed to remove incomplete model bundle: {}", e))?;
    }
    std::fs::create_dir_all(&staging_root)
        .map_err(|e| format!("Failed to create extraction staging directory: {}", e))?;

    let extraction = (|| {
        let file = std::fs::File::open(archive_path)
            .map_err(|e| format!("Failed to open archive: {}", e))?;
        let decompressor = bzip2::read::BzDecoder::new(file);
        let mut archive = tar::Archive::new(decompressor);
        archive
            .unpack(&staging_root)
            .map_err(|e| format!("Failed to extract archive: {}", e))?;
        if !transcriber::parakeet::specific_model_exists_in(model_name, &staging_root) {
            return Err("Extracted Parakeet bundle is incomplete".to_string());
        }
        std::fs::rename(&staged_dir, &final_dir)
            .map_err(|e| format!("Failed to publish Parakeet model bundle: {}", e))?;
        Ok(())
    })();

    let _ = std::fs::remove_dir_all(&staging_root);
    extraction
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
                "total": total,
                "phase": "downloading"
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

    if total > 0 && received != total {
        let _ = tokio::fs::remove_file(dest).await;
        return Err(format!(
            "Download ended early: received {} of {} bytes",
            received, total
        ));
    }

    Ok(received)
}
