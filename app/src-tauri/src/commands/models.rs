use crate::model_runtime::{self, InstallKind, InstallState};
use crate::transcriber::{self, TranscriptionBackend};
use crate::vad;
use crate::State;
use std::sync::LazyLock;
use tauri::Emitter;

static VAD_INSTALL_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

#[tauri::command]
pub fn check_model_exists(state: tauri::State<'_, State>) -> bool {
    state.app_state.model_runtime.any_model_installed()
}

#[tauri::command]
pub fn get_model_runtime_catalog(
    state: tauri::State<'_, State>,
) -> Vec<model_runtime::ModelRuntimeSnapshot> {
    state.app_state.model_runtime.catalog()
}

#[tauri::command]
pub fn get_model_runtime_status(
    state: tauri::State<'_, State>,
    model_name: String,
) -> Result<model_runtime::ModelRuntimeSnapshot, String> {
    state.app_state.model_runtime.snapshot(&model_name)
}

#[tauri::command]
pub fn check_specific_model_exists(state: tauri::State<'_, State>, model_name: String) -> bool {
    if !is_safe_model_identifier(&model_name) {
        return false;
    }
    state
        .app_state
        .model_runtime
        .snapshot(&model_name)
        .is_ok_and(|snapshot| snapshot.install_state == InstallState::Installed)
}

fn is_safe_model_identifier(model_name: &str) -> bool {
    // Model identifiers are catalog keys, never paths supplied by callers.
    !model_name.contains("..") && !model_name.contains('/') && !model_name.contains('\\')
}

#[tauri::command]
pub async fn download_model(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, State>,
    model_name: String,
) -> Result<(), String> {
    let definition = model_runtime::model_definition(&model_name)?;
    if !model_runtime::model_supported(definition) {
        return Err("This model is not supported on the current platform".to_string());
    }

    // The entire existence-check/download/install transaction is single-flight
    // per model. Different models may still download concurrently.
    let install_lock = state.app_state.model_runtime.install_lock(&model_name)?;
    let _install_guard = install_lock.lock().await;
    if !state
        .app_state
        .model_runtime
        .begin_install(Some(&app_handle), &model_name)?
    {
        return Ok(());
    }

    let install_result: Result<(), String> = async {
        // Whisper and sherpa share Murmur's models directory. FluidAudio owns a
        // separate Application Support cache, but VAD must still land here.
        // Keep setup inside this result boundary so every failure after the
        // Installing transition is published as Invalid rather than leaving a
        // permanently in-progress snapshot.
        let models_dir = transcriber::WhisperBackend::new().models_dir()?;
        tokio::fs::create_dir_all(&models_dir)
            .await
            .map_err(|e| format!("Failed to create models directory: {}", e))?;

        if definition.install_kind == InstallKind::Coreml {
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
        } else if definition.install_kind == InstallKind::Parakeet {
            download_parakeet_model(&app_handle, &model_name, &models_dir).await?;
        } else {
            download_whisper_model(&app_handle, &model_name, &models_dir).await?;
        }

        state.app_state.model_runtime.set_install_state(
            Some(&app_handle),
            &model_name,
            InstallState::Validating,
        )?;
        if !model_runtime::model_installed(&model_name) {
            return Err("Model installation completed but validation failed".to_string());
        }

        // Co-download VAD model alongside the transcription model (~1.8MB).
        // Its own lock prevents different model installs from sharing a temp file.
        if let Err(error) = ensure_vad_model(&app_handle).await {
            tracing::warn!(target: "system", "VAD model co-download failed (non-fatal): {}", error);
        }

        Ok(())
    }
    .await;

    match install_result {
        Ok(()) => state.app_state.model_runtime.set_install_state(
            Some(&app_handle),
            &model_name,
            InstallState::Installed,
        ),
        Err(error) => {
            let _ = state.app_state.model_runtime.set_install_state(
                Some(&app_handle),
                &model_name,
                InstallState::Invalid,
            );
            Err(error)
        }
    }
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
        assert!(!is_safe_model_identifier("../base.en"));
        assert!(!is_safe_model_identifier("models/base.en"));
        assert!(!is_safe_model_identifier("models\\base.en"));
        assert!(is_safe_model_identifier("base.en"));
    }

    #[test]
    fn coreml_model_is_not_dispatched_as_sherpa_download() {
        assert!(transcriber::is_coreml_model(transcriber::COREML_MODEL_NAME));
        assert!(!transcriber::parakeet::is_parakeet_model(
            transcriber::COREML_MODEL_NAME
        ));
        assert_eq!(
            model_runtime::model_definition(transcriber::COREML_MODEL_NAME)
                .unwrap()
                .install_kind,
            InstallKind::Coreml
        );
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

        assert!(
            matches!(error, ParakeetInstallError::InvalidArchive(_)),
            "unexpected classification: {error:?}"
        );
        assert!(should_discard_archive(&error));
        assert!(!models_dir.join(&dir_name).exists());
        assert!(!models_dir.join(format!(".{dir_name}.extracting")).exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn corrupt_parakeet_archive_is_marked_for_redownload() {
        let root = test_dir("parakeet-corrupt-archive");
        let models_dir = root.join("models");
        fs::create_dir_all(&models_dir).unwrap();
        let model_name = "parakeet-tdt-0.6b-v2-fp16";
        let (_, dir_name) = transcriber::parakeet::download_spec(model_name).unwrap();
        let archive_path = root.join("model.tar.bz2");
        fs::write(&archive_path, b"not a bzip2 archive").unwrap();

        let error = extract_parakeet_archive(&archive_path, &models_dir, model_name, &dir_name)
            .unwrap_err();

        assert!(
            matches!(error, ParakeetInstallError::InvalidArchive(_)),
            "unexpected classification: {error:?}"
        );
        assert!(should_discard_archive(&error));
        assert!(!models_dir.join(&dir_name).exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn transient_publish_failure_preserves_a_retryable_archive() {
        let root = test_dir("parakeet-transient-install");
        let models_dir = root.join("models");
        fs::create_dir_all(&models_dir).unwrap();
        let model_name = "parakeet-tdt-0.6b-v2-fp16";
        let (_, dir_name) = transcriber::parakeet::download_spec(model_name).unwrap();
        let archive_path = root.join("model.tar.bz2");
        write_parakeet_archive(&archive_path, &dir_name, true);
        fs::write(models_dir.join(&dir_name), b"blocks directory publication").unwrap();

        let error = extract_parakeet_archive(&archive_path, &models_dir, model_name, &dir_name)
            .unwrap_err();

        assert!(matches!(error, ParakeetInstallError::Installation(_)));
        assert!(!should_discard_archive(&error));
        assert!(archive_path.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn model_install_locks_are_keyed_and_reused() {
        let manager = model_runtime::ModelRuntimeManager::default();
        let first = manager.install_lock("base.en").unwrap();
        let same = manager.install_lock("base.en").unwrap();
        let different = manager.install_lock("tiny.en").unwrap();
        assert!(std::sync::Arc::ptr_eq(&first, &same));
        assert!(!std::sync::Arc::ptr_eq(&first, &different));
        assert!(manager.install_lock("unknown-model").is_err());
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
        match extract_parakeet_archive_on_worker(archive, root, model, bundle).await {
            Ok(()) => {
                let _ = tokio::fs::remove_file(&legacy_temp_path).await;
                let _ = tokio::fs::remove_file(&archive_path).await;
                tracing::info!(target: "system", "Recovered Parakeet installation from retained archive: {}", dir_name);
                return Ok(());
            }
            Err(error) if should_discard_archive(&error) => {
                tracing::warn!(target: "system", "Retained Parakeet archive was unusable; downloading again: {}", error);
                let _ = tokio::fs::remove_file(&legacy_temp_path).await;
            }
            Err(error) => return Err(error.to_string()),
        }
    }

    let mut downloaded_this_attempt = false;
    loop {
        // A finalized archive is retained after transient installation failures
        // so Retry performs only local work. Invalid contents are discarded and
        // downloaded once more in the same attempt.
        if !archive_path.is_file() {
            let download_path = models_dir.join(format!("{}.tar.bz2.download", dir_name));
            let received = stream_download(app_handle, &url, &download_path).await?;
            tokio::fs::rename(&download_path, &archive_path)
                .await
                .map_err(|e| {
                    let _ = std::fs::remove_file(&download_path);
                    format!("Failed to finalize Parakeet archive: {}", e)
                })?;
            downloaded_this_attempt = true;
            tracing::info!(target: "system", "Parakeet archive downloaded: {} ({} bytes)", dir_name, received);
        }

        emit_installing(app_handle);
        let result = extract_parakeet_archive_on_worker(
            archive_path.clone(),
            models_dir.to_path_buf(),
            model_name.to_string(),
            dir_name.clone(),
        )
        .await;

        match result {
            Ok(()) => {
                let _ = tokio::fs::remove_file(&archive_path).await;
                tracing::info!(target: "system", "Parakeet model installed: {}", dir_name);
                return Ok(());
            }
            Err(error) if should_discard_archive(&error) => {
                let _ = tokio::fs::remove_file(&archive_path).await;
                if downloaded_this_attempt {
                    return Err(error.to_string());
                }
                tracing::warn!(target: "system", "Retained Parakeet archive was invalid; downloading again: {}", error);
            }
            Err(error) => return Err(error.to_string()),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum ParakeetInstallError {
    InvalidArchive(String),
    Installation(String),
}

impl std::fmt::Display for ParakeetInstallError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidArchive(message) | Self::Installation(message) => {
                formatter.write_str(message)
            }
        }
    }
}

fn should_discard_archive(error: &ParakeetInstallError) -> bool {
    matches!(error, ParakeetInstallError::InvalidArchive(_))
}

async fn extract_parakeet_archive_on_worker(
    archive_path: std::path::PathBuf,
    models_dir: std::path::PathBuf,
    model_name: String,
    dir_name: String,
) -> Result<(), ParakeetInstallError> {
    tokio::task::spawn_blocking(move || {
        extract_parakeet_archive(&archive_path, &models_dir, &model_name, &dir_name)
    })
    .await
    .map_err(|error| {
        ParakeetInstallError::Installation(format!("Extraction task failed: {error}"))
    })?
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
) -> Result<(), ParakeetInstallError> {
    let final_dir = models_dir.join(dir_name);
    let staging_root = models_dir.join(format!(".{}.extracting", dir_name));
    let staged_dir = staging_root.join(dir_name);

    let _ = std::fs::remove_dir_all(&staging_root);
    if final_dir.exists()
        && !transcriber::parakeet::specific_model_exists_in(model_name, models_dir)
    {
        std::fs::remove_dir_all(&final_dir)
            .map_err(|e| {
                ParakeetInstallError::Installation(format!(
                    "Failed to remove incomplete model bundle: {}",
                    e
                ))
            })?;
    }
    std::fs::create_dir_all(&staging_root)
        .map_err(|e| {
            ParakeetInstallError::Installation(format!(
                "Failed to create extraction staging directory: {}",
                e
            ))
        })?;

    let extraction = (|| {
        let file = std::fs::File::open(archive_path)
            .map_err(|e| {
                ParakeetInstallError::Installation(format!("Failed to open archive: {}", e))
            })?;
        let decompressor = bzip2::read::BzDecoder::new(file);
        let mut archive = tar::Archive::new(decompressor);
        archive
            .unpack(&staging_root)
            .map_err(classify_archive_unpack_error)?;
        if !transcriber::parakeet::specific_model_exists_in(model_name, &staging_root) {
            return Err(ParakeetInstallError::InvalidArchive(
                "Extracted Parakeet bundle is incomplete".to_string(),
            ));
        }
        std::fs::rename(&staged_dir, &final_dir)
            .map_err(|e| {
                ParakeetInstallError::Installation(format!(
                    "Failed to publish Parakeet model bundle: {}",
                    e
                ))
            })?;
        Ok(())
    })();

    let _ = std::fs::remove_dir_all(&staging_root);
    extraction
}

fn classify_archive_unpack_error(error: std::io::Error) -> ParakeetInstallError {
    let message = error.to_string();
    let normalized = message.to_ascii_lowercase();
    if matches!(
        error.kind(),
        std::io::ErrorKind::InvalidData | std::io::ErrorKind::UnexpectedEof
    )
        || normalized.contains("data integrity")
        || normalized.contains("corrupt")
        || normalized.contains("failed to iterate over archive")
    {
        ParakeetInstallError::InvalidArchive(format!("Invalid Parakeet archive: {message}"))
    } else {
        ParakeetInstallError::Installation(format!("Failed to extract archive: {message}"))
    }
}

/// Ensure the VAD model is present, downloading it if necessary.
/// This is the fallback for users who have a transcription model but not the
/// VAD model (e.g. upgrade from a pre-VAD version or manual model install).
pub(crate) async fn ensure_vad_model(app_handle: &tauri::AppHandle) -> Result<(), String> {
    let _install_guard = VAD_INSTALL_LOCK.lock().await;
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
