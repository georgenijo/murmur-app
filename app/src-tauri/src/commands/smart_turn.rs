//! Smart Auto-Stop commands (#834): the pinned Smart Turn model's
//! install lifecycle, and the per-pause end-of-turn check.
//!
//! The download is user-initiated from Settings, like every other model. It
//! streams to an exact `.partial` path, caps the size while streaming, hashes
//! as bytes arrive, fsyncs, and atomically publishes under a hash-versioned
//! directory. Any mismatch fails closed and deletes the partial.
//!
//! `check_smart_turn` is called by the frontend's silence detector once per
//! pause. It peeks the tail of the live recording (a copy; capture keeps
//! running and the final stop still owns the buffer) and scores it on a
//! blocking worker. No audio or score is persisted; the log line carries only
//! the probability and timings.

use crate::smart_turn::{
    self, CheckError, MODEL_FILENAME, MODEL_SHA256, MODEL_SIZE_BYTES, MODEL_URL,
};
use crate::state::DictationStatus;
use crate::{MutexExt, State};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::LazyLock;

/// Single-flight guard: only one Smart Turn download at a time.
static DOWNLOAD_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));
static DOWNLOADING: AtomicBool = AtomicBool::new(false);

/// Below this much audio there is no turn worth scoring (the model was trained
/// on clips of at least a second of speech).
const MIN_CHECK_SAMPLES: usize = smart_turn::SAMPLE_RATE / 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SmartTurnModelState {
    NotDownloaded,
    Downloading,
    Ready,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SmartTurnModelStatus {
    pub state: SmartTurnModelState,
    pub size_bytes: u64,
}

#[tauri::command]
pub fn smart_turn_model_status() -> SmartTurnModelStatus {
    let state = if smart_turn::model_installed() {
        SmartTurnModelState::Ready
    } else if DOWNLOADING.load(Ordering::Acquire) {
        SmartTurnModelState::Downloading
    } else {
        SmartTurnModelState::NotDownloaded
    };
    SmartTurnModelStatus {
        state,
        size_bytes: MODEL_SIZE_BYTES,
    }
}

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
pub async fn download_smart_turn_model() -> Result<(), String> {
    let _single_flight = DOWNLOAD_LOCK.lock().await;
    if smart_turn::model_installed() {
        return Ok(());
    }
    let _flag = DownloadingFlag::set();

    let root = smart_turn::models_root()
        .ok_or_else(|| "Could not determine the Smart Turn model directory".to_string())?;
    tokio::fs::create_dir_all(&root)
        .await
        .map_err(|e| format!("Failed to create the Smart Turn model directory: {e}"))?;
    let partial = root.join(format!("{MODEL_FILENAME}.partial"));
    let _ = tokio::fs::remove_file(&partial).await;

    let (size, sha) = stream_verified_download(&partial).await.inspect_err(|_| {
        let _ = std::fs::remove_file(&partial);
    })?;
    if size != MODEL_SIZE_BYTES || sha != MODEL_SHA256 {
        let _ = tokio::fs::remove_file(&partial).await;
        return Err("Smart Turn model verification failed".to_string());
    }

    let final_dir = root.join(MODEL_SHA256);
    tokio::fs::create_dir_all(&final_dir)
        .await
        .map_err(|e| format!("Failed to create the model version directory: {e}"))?;
    tokio::fs::rename(&partial, final_dir.join(MODEL_FILENAME))
        .await
        .map_err(|e| {
            let _ = std::fs::remove_file(&partial);
            format!("Failed to publish the Smart Turn model: {e}")
        })?;
    tracing::info!(target: "system", size_bytes = size, "smart_turn_model_installed");
    Ok(())
}

/// Stream the pinned URL to `dest`, capping the size and hashing as bytes
/// arrive. At ~8.7 MB the download needs no progress events.
async fn stream_verified_download(dest: &std::path::Path) -> Result<(u64, String), String> {
    use futures_util::StreamExt;
    use sha2::{Digest, Sha256};
    use tokio::io::AsyncWriteExt;

    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(30))
        .timeout(std::time::Duration::from_secs(5 * 60))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {e}"))?;
    let response = client
        .get(MODEL_URL)
        .send()
        .await
        .map_err(|e| format!("Download request failed: {e}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "Download failed with status: {}",
            response.status()
        ));
    }

    let mut received: u64 = 0;
    let mut hasher = Sha256::new();
    let mut file = tokio::fs::File::create(dest)
        .await
        .map_err(|e| format!("Failed to create partial file: {e}"))?;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Download error: {e}"))?;
        received += chunk.len() as u64;
        if received > MODEL_SIZE_BYTES {
            return Err("Download exceeded the expected model size".to_string());
        }
        hasher.update(&chunk);
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("Failed to write partial file: {e}"))?;
    }
    file.flush()
        .await
        .map_err(|e| format!("Failed to flush partial file: {e}"))?;
    file.sync_all()
        .await
        .map_err(|e| format!("Failed to fsync partial file: {e}"))?;
    Ok((received, format!("{:x}", hasher.finalize())))
}

#[tauri::command]
pub async fn remove_smart_turn_model() -> Result<(), String> {
    let _single_flight = DOWNLOAD_LOCK.lock().await;
    tokio::task::spawn_blocking(smart_turn::unload)
        .await
        .map_err(|_| "Failed to unload the Smart Turn model".to_string())?;
    let root = smart_turn::models_root()
        .ok_or_else(|| "Could not determine the Smart Turn model directory".to_string())?;
    let final_dir = root.join(MODEL_SHA256);
    if final_dir.exists() {
        tokio::fs::remove_dir_all(&final_dir)
            .await
            .map_err(|e| format!("Failed to remove the Smart Turn model: {e}"))?;
    }
    let _ = tokio::fs::remove_file(root.join(format!("{MODEL_FILENAME}.partial"))).await;
    tracing::info!(target: "system", "smart_turn_model_removed");
    Ok(())
}

/// One answered check. `probability` is the model's end-of-turn score.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SmartTurnCheck {
    pub recording_id: u64,
    pub probability: f32,
    pub inference_ms: u64,
}

/// Why a check returned no score. The frontend treats every outcome the same
/// way — keep waiting for the silence timer — but logs which one it was.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "outcome")]
pub enum SmartTurnCheckResult {
    Scored(SmartTurnCheck),
    NotRecording,
    TooShort,
    Unavailable { reason: &'static str },
}

fn current_recording(state: &crate::state::AppState) -> Option<u64> {
    let recording_id = state.recording_id.load(Ordering::SeqCst);
    (recording_id > 0
        && state.dictation.lock_or_recover().status == DictationStatus::Recording
        && !state.is_cancelled(recording_id))
    .then_some(recording_id)
}

/// Score whether the speaker has finished, for the recording in flight now.
#[tauri::command]
pub async fn check_smart_turn(
    state: tauri::State<'_, State>,
) -> Result<SmartTurnCheckResult, String> {
    let Some(recording_id) = current_recording(&state.app_state) else {
        return Ok(SmartTurnCheckResult::NotRecording);
    };
    if !smart_turn::model_installed() {
        return Ok(SmartTurnCheckResult::Unavailable {
            reason: CheckError::NotInstalled.as_str(),
        });
    }
    // The peek waits on the audio supervisor, so it runs off the async runtime
    // too; the supervisor copies the tail under the buffer lock and returns.
    let scored = tokio::task::spawn_blocking(move || {
        let samples = crate::audio_lifecycle::peek_dictation_samples(
            recording_id,
            smart_turn::WINDOW_SAMPLES,
        )?;
        Some((samples.len() >= MIN_CHECK_SAMPLES).then(|| smart_turn::score(&samples)))
    })
    .await
    .map_err(|_| "Smart Turn worker failed".to_string())?;
    let Some(scored) = scored else {
        return Ok(SmartTurnCheckResult::NotRecording);
    };
    let Some(scored) = scored else {
        return Ok(SmartTurnCheckResult::TooShort);
    };
    match scored {
        Ok(score) => {
            tracing::info!(
                target: "recording",
                recording_id,
                probability = score.probability,
                inference_ms = score.inference_ms,
                loaded_model = score.loaded,
                "smart turn check"
            );
            Ok(SmartTurnCheckResult::Scored(SmartTurnCheck {
                recording_id,
                probability: score.probability,
                inference_ms: score.inference_ms,
            }))
        }
        Err(error) => {
            tracing::info!(
                target: "recording",
                recording_id,
                outcome = error.as_str(),
                "smart turn check unavailable"
            );
            Ok(SmartTurnCheckResult::Unavailable {
                reason: error.as_str(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_results_serialize_with_a_tagged_outcome() {
        let scored = serde_json::to_value(SmartTurnCheckResult::Scored(SmartTurnCheck {
            recording_id: 7,
            probability: 0.5,
            inference_ms: 12,
        }))
        .unwrap();
        assert_eq!(scored["outcome"], "scored");
        assert_eq!(scored["recordingId"], 7);
        assert_eq!(scored["inferenceMs"], 12);
        let unavailable = serde_json::to_value(SmartTurnCheckResult::Unavailable {
            reason: CheckError::Busy.as_str(),
        })
        .unwrap();
        assert_eq!(unavailable["outcome"], "unavailable");
        assert_eq!(unavailable["reason"], "busy");
        assert_eq!(
            serde_json::to_value(SmartTurnCheckResult::NotRecording).unwrap()["outcome"],
            "notRecording"
        );
    }

    #[test]
    fn status_carries_only_bounded_metadata() {
        let json = serde_json::to_string(&smart_turn_model_status()).unwrap();
        assert!(json.contains("sizeBytes"));
        assert!(!json.contains("huggingface"));
    }
}
