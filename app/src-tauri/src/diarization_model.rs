use crate::MutexExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};
use tauri::Emitter;

pub const MODEL_ID: &str = "meeting-diarization-coreml";
const REVISION: &str = "1ed7a662fdc7109e36d822db793ee6eebdaf8594";
pub const MODEL_BYTES: u64 = 21_599_417;
static INSTALL_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));
static INSTALLING: AtomicBool = AtomicBool::new(false);
// Native model use and filesystem publication/removal share this reservation.
pub(crate) static MODEL_USE: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

#[derive(Deserialize)]
struct ModelFile {
    path: String,
    bytes: u64,
    sha256: String,
}

fn manifest() -> Vec<ModelFile> {
    serde_json::from_str(include_str!("diarization-model-manifest.json"))
        .expect("compiled diarization model manifest")
}

pub(crate) struct CacheLease {
    _file: fs::File,
}
impl CacheLease {
    pub(crate) fn acquire(exclusive: bool) -> Result<Self, String> {
        let root = directory().ok_or("The speaker model directory is unavailable.")?;
        let parent = root
            .parent()
            .ok_or("The speaker model directory is unavailable.")?;
        fs::create_dir_all(parent).map_err(|_| "The speaker model directory is unavailable.")?;
        let mut options = fs::OpenOptions::new();
        options.create(true).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let file = options
            .open(parent.join(".murmur-diarization.lock"))
            .map_err(|_| "The speaker model lock is unavailable.")?;
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let operation = if exclusive {
                libc::LOCK_EX
            } else {
                libc::LOCK_SH
            };
            if unsafe { libc::flock(file.as_raw_fd(), operation | libc::LOCK_NB) } != 0 {
                return Err(
                    "Speaker models are in use by another Murmur operation. Try again shortly."
                        .into(),
                );
            }
        }
        #[cfg(not(unix))]
        let _ = exclusive;
        Ok(Self { _file: file })
    }
}

fn sweep_staging(parent: &Path) {
    if let Ok(entries) = fs::read_dir(parent) {
        for entry in entries.flatten() {
            if entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with(".murmur-diarization-"))
            {
                let _ = fs::remove_dir_all(entry.path());
            }
        }
    }
}

pub fn supported() -> bool {
    cfg!(all(target_os = "macos", target_arch = "aarch64"))
}

pub(crate) fn directory() -> Option<PathBuf> {
    dirs::data_dir().map(|root| root.join("FluidAudio/Models/speaker-diarization-coreml"))
}

fn regular_file(root: &Path, relative: &str) -> bool {
    let mut path = root.to_path_buf();
    if !fs::symlink_metadata(&path).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink()) {
        return false;
    }
    let parts = Path::new(relative).components().collect::<Vec<_>>();
    for (index, part) in parts.iter().enumerate() {
        if !matches!(part, std::path::Component::Normal(_)) {
            return false;
        }
        path.push(part.as_os_str());
        let Ok(meta) = fs::symlink_metadata(&path) else {
            return false;
        };
        if meta.file_type().is_symlink() || (index + 1 < parts.len() && !meta.is_dir()) {
            return false;
        }
    }
    path.is_file()
}

pub(crate) fn validate_at(root: &Path) -> bool {
    manifest().iter().all(|entry| {
        if !regular_file(root, &entry.path) {
            return false;
        }
        let Ok(mut file) = fs::File::open(root.join(&entry.path)) else {
            return false;
        };
        if !file.metadata().is_ok_and(|meta| meta.len() == entry.bytes) {
            return false;
        }
        let mut digest = Sha256::new();
        let mut buffer = [0; 64 * 1024];
        loop {
            match file.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => digest.update(&buffer[..count]),
                Err(_) => return false,
            }
        }
        format!("{:x}", digest.finalize()) == entry.sha256
    })
}

pub fn installed() -> bool {
    supported() && directory().is_some_and(|root| validate_at(&root))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    supported: bool,
    installed: bool,
    installing: bool,
    bytes: u64,
}

#[tauri::command]
pub fn get_diarization_model_status() -> ModelStatus {
    ModelStatus {
        supported: supported(),
        installed: installed(),
        installing: INSTALLING.load(Ordering::SeqCst),
        bytes: MODEL_BYTES,
    }
}

struct Staging(PathBuf);
impl Drop for Staging {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct Installing;
impl Drop for Installing {
    fn drop(&mut self) {
        INSTALLING.store(false, Ordering::SeqCst);
    }
}

pub async fn download(app: &tauri::AppHandle) -> Result<(), String> {
    let _install = INSTALL_LOCK.lock().await;
    if !supported() {
        return Err("Remote speaker models require an Apple Silicon Mac.".into());
    }
    if installed() {
        return Ok(());
    }
    let _cache = CacheLease::acquire(true)?;
    INSTALLING.store(true, Ordering::SeqCst);
    let _installing = Installing;
    let root = directory().ok_or("The local model directory is unavailable.")?;
    let parent = root
        .parent()
        .ok_or("The local model directory is unavailable.")?;
    fs::create_dir_all(parent).map_err(|_| "The local model directory is unavailable.")?;
    sweep_staging(parent);
    let staging = Staging(parent.join(format!(".murmur-diarization-{}", uuid::Uuid::new_v4())));
    fs::create_dir(&staging.0).map_err(|_| "The speaker model download could not start.")?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|_| "The speaker model download could not start.")?;
    let attempt_id = uuid::Uuid::new_v4().as_u128() as u64;
    let mut received = 0u64;
    for entry in manifest() {
        let url = format!("https://huggingface.co/FluidInference/speaker-diarization-coreml/resolve/{REVISION}/{}", entry.path);
        let mut response = client
            .get(url)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|_| "The speaker model download failed. Retry when connected.")?;
        let mut bytes = Vec::with_capacity(entry.bytes as usize);
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| "The speaker model download was interrupted.")?
        {
            if bytes.len() as u64 + chunk.len() as u64 > entry.bytes {
                return Err("The speaker model download did not match its manifest.".into());
            }
            bytes.extend_from_slice(&chunk);
            received += chunk.len() as u64;
            let _ = app.emit("download-progress", serde_json::json!({"modelName": MODEL_ID, "attemptId":attempt_id, "received":received, "total":MODEL_BYTES, "phase":"downloading", "repeatedRepair":false}));
        }
        if bytes.len() as u64 != entry.bytes
            || format!("{:x}", Sha256::digest(&bytes)) != entry.sha256
        {
            return Err("The speaker model download did not match its manifest.".into());
        }
        let path = staging.0.join(&entry.path);
        fs::create_dir_all(path.parent().ok_or("Invalid model manifest.")?)
            .map_err(|_| "The speaker model could not be saved.")?;
        fs::write(path, bytes).map_err(|_| "The speaker model could not be saved.")?;
    }
    let _use = MODEL_USE.lock_or_recover();
    if !validate_at(&staging.0) {
        return Err("The speaker models failed validation.".into());
    }
    if root.exists() {
        fs::remove_dir_all(&root)
            .map_err(|_| "The incomplete speaker models could not be replaced.")?;
    }
    fs::rename(&staging.0, &root).map_err(|_| "The speaker models could not be installed.")?;
    let _ = app.emit("download-progress", serde_json::json!({"modelName":MODEL_ID,"attemptId":attempt_id,"received":MODEL_BYTES,"total":MODEL_BYTES,"phase":"validating","repeatedRepair":false}));
    Ok(())
}

#[tauri::command]
pub async fn remove_diarization_model() -> Result<(), String> {
    let _install = INSTALL_LOCK.lock().await;
    crate::meeting_diarization::cancel_all()?;
    let _use = MODEL_USE.lock_or_recover();
    let _cache = CacheLease::acquire(true)?;
    if let Some(root) = directory() {
        if let Some(parent) = root.parent() {
            sweep_staging(parent);
        }
        if !root.exists() {
            return Ok(());
        }
        fs::remove_dir_all(root).map_err(|_| "The speaker models could not be removed.")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_manifest_is_bounded_and_contains_all_offline_bundles() {
        let entries = manifest();
        assert_eq!(entries.iter().map(|f| f.bytes).sum::<u64>(), MODEL_BYTES);
        assert_eq!(entries.len(), 21);
        assert!(entries
            .iter()
            .all(|f| f.sha256.len() == 64 && !f.path.contains("..")));
        assert!(!validate_at(Path::new("/nonexistent-murmur-diarization")));
    }
}
