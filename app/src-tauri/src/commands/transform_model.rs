//! Install + lifecycle commands for the pinned local-LLM transform model (#312).
//!
//! The download streams to an exact `.partial` path, enforces the compiled-in
//! maximum and final size while streaming, computes SHA-256 while streaming,
//! fsyncs, and atomically publishes under a hash-versioned directory beneath the
//! app models dir. Any mismatch fails closed and deletes the partial. The helper
//! has no downloader or URL handling — all trust lives here in the signed app.

use crate::llm_sidecar::{
    installed_model_path, transform_models_root, TRANSFORM_MODEL_FILENAME, TRANSFORM_MODEL_SHA256,
    TRANSFORM_MODEL_SIZE_BYTES, TRANSFORM_MODEL_URL,
};
use crate::State;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::LazyLock;
use tauri::Emitter;

/// Single-flight guard: only one transform-model download at a time.
static DOWNLOAD_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));
/// Coarse status flag so `transform_model_status` can report `downloading`.
static DOWNLOADING: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TransformModelState {
    NotDownloaded,
    Downloading,
    Ready,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransformModelStatus {
    pub state: TransformModelState,
    /// Absolute path to the published model when ready; `None` otherwise.
    pub path: Option<String>,
    pub size_bytes: u64,
    pub sha256: &'static str,
}

/// A published model of exactly the pinned size is considered ready. The full
/// SHA-256 is re-verified before every spawn, so status stays cheap here.
fn model_is_ready() -> Option<std::path::PathBuf> {
    let path = installed_model_path()?;
    let metadata = std::fs::metadata(&path).ok()?;
    if metadata.is_file() && metadata.len() == TRANSFORM_MODEL_SIZE_BYTES {
        Some(path)
    } else {
        None
    }
}

#[tauri::command]
pub fn transform_model_status() -> TransformModelStatus {
    let ready = model_is_ready();
    let state = if ready.is_some() {
        TransformModelState::Ready
    } else if DOWNLOADING.load(Ordering::Acquire) {
        TransformModelState::Downloading
    } else {
        TransformModelState::NotDownloaded
    };
    TransformModelStatus {
        state,
        path: ready.map(|p| p.to_string_lossy().into_owned()),
        size_bytes: TRANSFORM_MODEL_SIZE_BYTES,
        sha256: TRANSFORM_MODEL_SHA256,
    }
}

/// Clears the `DOWNLOADING` flag on drop, even on early return / error.
struct DownloadingFlag;
impl DownloadingFlag {
    fn set() -> Self {
        DOWNLOADING.store(true, Ordering::Release);
        Self
    }
}
impl Drop for DownloadingFlag {
    fn drop(&mut self) {
        DOWNLOADING.store(false, Ordering::Release);
    }
}

#[tauri::command]
pub async fn download_transform_model(
    app_handle: tauri::AppHandle,
    _state: tauri::State<'_, State>,
) -> Result<(), String> {
    let _single_flight = DOWNLOAD_LOCK.lock().await;
    if model_is_ready().is_some() {
        return Ok(());
    }
    let _flag = DownloadingFlag::set();

    let root = transform_models_root()
        .ok_or_else(|| "Could not determine transform model directory".to_string())?;
    tokio::fs::create_dir_all(&root)
        .await
        .map_err(|e| format!("Failed to create transform model directory: {}", e))?;

    let partial = root.join(format!("{}.partial", TRANSFORM_MODEL_FILENAME));
    // Clean any residue from a previous interrupted attempt.
    let _ = tokio::fs::remove_file(&partial).await;

    let (size, sha) = stream_verified_download(&app_handle, &partial).await.map_err(|e| {
        let _ = std::fs::remove_file(&partial);
        e
    })?;

    if size != TRANSFORM_MODEL_SIZE_BYTES || sha != TRANSFORM_MODEL_SHA256 {
        let _ = tokio::fs::remove_file(&partial).await;
        // Never log the observed hash/size content; the mismatch itself is the
        // signal and the bytes are untrusted.
        return Err("Transform model verification failed".to_string());
    }

    let final_dir = root.join(TRANSFORM_MODEL_SHA256);
    tokio::fs::create_dir_all(&final_dir)
        .await
        .map_err(|e| format!("Failed to create model version directory: {}", e))?;
    let final_path = final_dir.join(TRANSFORM_MODEL_FILENAME);

    tokio::fs::rename(&partial, &final_path).await.map_err(|e| {
        let _ = std::fs::remove_file(&partial);
        format!("Failed to publish transform model: {}", e)
    })?;

    tracing::info!(
        target: "system",
        size_bytes = size,
        "transform_model_installed"
    );
    // Dedicated channel so this never collides with the whisper/parakeet
    // downloader UI on the shared "download-progress" event.
    let _ = app_handle.emit(
        "transform-model-download-progress",
        serde_json::json!({ "received": size, "total": size, "phase": "installed" }),
    );
    Ok(())
}

/// Stream the pinned URL to `dest`, enforcing the maximum size while streaming
/// and hashing as bytes arrive. Returns `(size_bytes, sha256_hex)`.
async fn stream_verified_download(
    app_handle: &tauri::AppHandle,
    dest: &std::path::Path,
) -> Result<(u64, String), String> {
    use futures_util::StreamExt;
    use sha2::{Digest, Sha256};
    use tokio::io::AsyncWriteExt;

    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .timeout(std::time::Duration::from_secs(30 * 60))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let response = client
        .get(TRANSFORM_MODEL_URL)
        .send()
        .await
        .map_err(|e| format!("Download request failed: {}", e))?;
    if !response.status().is_success() {
        return Err(format!("Download failed with status: {}", response.status()));
    }

    // Report against the pinned size so progress is meaningful even without a
    // server content-length.
    let total = TRANSFORM_MODEL_SIZE_BYTES;
    let mut received: u64 = 0;
    let mut hasher = Sha256::new();

    let mut file = tokio::fs::File::create(dest)
        .await
        .map_err(|e| format!("Failed to create partial file: {}", e))?;

    // Throttle progress emits so a 1.1 GB stream doesn't flood the UI: emit on
    // each whole-percent advance or at most every 250ms, on a dedicated channel
    // that never collides with the whisper/parakeet downloader.
    let mut last_emit = std::time::Instant::now();
    let mut last_pct: u64 = u64::MAX;

    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Download error: {}", e))?;
        received += chunk.len() as u64;
        if received > TRANSFORM_MODEL_SIZE_BYTES {
            return Err("Download exceeded the expected model size".to_string());
        }
        hasher.update(&chunk);
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("Failed to write partial file: {}", e))?;

        let pct = received.saturating_mul(100) / total.max(1);
        let now = std::time::Instant::now();
        if pct != last_pct || now.duration_since(last_emit) >= std::time::Duration::from_millis(250)
        {
            last_pct = pct;
            last_emit = now;
            let _ = app_handle.emit(
                "transform-model-download-progress",
                serde_json::json!({ "received": received, "total": total, "phase": "downloading" }),
            );
        }
    }

    file.flush()
        .await
        .map_err(|e| format!("Failed to flush partial file: {}", e))?;
    // fsync so the published bytes survive a crash between rename and flush.
    file.sync_all()
        .await
        .map_err(|e| format!("Failed to fsync partial file: {}", e))?;

    let sha = format!("{:x}", hasher.finalize());
    Ok((received, sha))
}

#[tauri::command]
pub async fn remove_transform_model(state: tauri::State<'_, State>) -> Result<(), String> {
    // Stop any resident helper first so the model file is not open.
    state.transform_runtime.shutdown();

    let root = transform_models_root()
        .ok_or_else(|| "Could not determine transform model directory".to_string())?;
    let final_dir = root.join(TRANSFORM_MODEL_SHA256);
    if final_dir.exists() {
        tokio::fs::remove_dir_all(&final_dir)
            .await
            .map_err(|e| format!("Failed to remove transform model: {}", e))?;
    }
    // Sweep any stray partial too.
    let _ = tokio::fs::remove_file(root.join(format!("{}.partial", TRANSFORM_MODEL_FILENAME))).await;
    tracing::info!(target: "system", "transform_model_removed");
    Ok(())
}

#[tauri::command]
pub fn reset_transform_runtime(state: tauri::State<'_, State>) {
    state.transform_runtime.reset();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_shape_carries_only_bounded_metadata() {
        let status = transform_model_status();
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("sha256"));
        assert!(json.contains("sizeBytes"));
        // No URL, revision, or free-form text leaks into the status payload.
        assert!(!json.contains("huggingface"));
    }

    #[test]
    fn catalog_pins_match_the_supervisor() {
        assert_eq!(TRANSFORM_MODEL_SIZE_BYTES, 1_117_320_736);
        assert_eq!(TRANSFORM_MODEL_SHA256.len(), 64);
        assert!(TRANSFORM_MODEL_URL.ends_with(TRANSFORM_MODEL_FILENAME));
    }
}
