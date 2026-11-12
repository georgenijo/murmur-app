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

/// Full validation: hashes every manifest file. Use this before the model is
/// consumed (meeting start, the diarization worker) and when publishing.
pub fn installed() -> bool {
    supported() && directory().is_some_and(|root| validate_at(&root))
}

/// Per-file identity used to decide whether a cached verdict still describes
/// the files on disk: size and modification time, plus the inode change time
/// where the platform has one (a writer cannot reset it).
#[derive(Clone, PartialEq, Eq)]
struct FileStamp {
    len: u64,
    modified: Option<std::time::SystemTime>,
    changed: Option<(i64, i64)>,
}

#[derive(Clone)]
struct CachedVerdict {
    root: PathBuf,
    stamps: Vec<FileStamp>,
    valid: bool,
}

type VerdictCache = Mutex<Option<CachedVerdict>>;

static VERDICT: VerdictCache = Mutex::new(None);

fn file_stamp(meta: &fs::Metadata) -> FileStamp {
    #[cfg(unix)]
    let changed = {
        use std::os::unix::fs::MetadataExt;
        Some((meta.ctime(), meta.ctime_nsec()))
    };
    #[cfg(not(unix))]
    let changed = None;
    FileStamp {
        len: meta.len(),
        modified: meta.modified().ok(),
        changed,
    }
}

/// Stat every manifest file without opening or hashing it. `None` means a
/// file is missing or not a regular file, so the install is invalid.
fn stamp_all(root: &Path) -> Option<Vec<FileStamp>> {
    manifest()
        .iter()
        .map(|entry| {
            if !regular_file(root, &entry.path) {
                return None;
            }
            fs::symlink_metadata(root.join(&entry.path))
                .ok()
                .map(|meta| file_stamp(&meta))
        })
        .collect()
}

/// Return the cached verdict while every file's stamp is unchanged; otherwise
/// run `validate` (the hashing pass) and cache its result.
fn cached_verdict(cache: &VerdictCache, root: &Path, validate: impl FnOnce() -> bool) -> bool {
    let Some(stamps) = stamp_all(root) else {
        return false;
    };
    if let Some(hit) = cache.lock_or_recover().as_ref() {
        if hit.root == root && hit.stamps == stamps {
            return hit.valid;
        }
    }
    let valid = validate();
    // A file changed while it was hashed: do not cache a verdict for stamps
    // that no longer describe what was read.
    if stamp_all(root).as_ref() == Some(&stamps) {
        *cache.lock_or_recover() = Some(CachedVerdict {
            root: root.to_path_buf(),
            stamps,
            valid,
        });
    }
    valid
}

fn invalidate_verdict(cache: &VerdictCache) {
    *cache.lock_or_recover() = None;
}

/// Status-path check. Hashes ~21.6 MB only the first time, or after a download,
/// removal, or any change to a model file; repeat calls only stat 21 files.
pub fn installed_cached() -> bool {
    supported()
        && directory().is_some_and(|root| {
            cached_verdict(&VERDICT, &root, || validate_at(&root))
        })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    supported: bool,
    installed: bool,
    installing: bool,
    bytes: u64,
}

/// Settings polls this once a second while an install runs, so it must never
/// hash on the main thread: it runs on the blocking pool and reuses the cached
/// verdict.
#[tauri::command]
pub async fn get_diarization_model_status() -> Result<ModelStatus, String> {
    tauri::async_runtime::spawn_blocking(|| ModelStatus {
        supported: supported(),
        installed: installed_cached(),
        installing: INSTALLING.load(Ordering::SeqCst),
        bytes: MODEL_BYTES,
    })
    .await
    .map_err(|_| "The speaker model status could not be read.".to_string())
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
    invalidate_verdict(&VERDICT);
    let published = fs::rename(&staging.0, &root);
    invalidate_verdict(&VERDICT);
    published.map_err(|_| "The speaker models could not be installed.")?;
    let _ = app.emit("download-progress", serde_json::json!({"modelName":MODEL_ID,"attemptId":attempt_id,"received":MODEL_BYTES,"total":MODEL_BYTES,"phase":"validating","repeatedRepair":false}));
    Ok(())
}

#[tauri::command]
pub async fn remove_diarization_model() -> Result<(), String> {
    let _install = INSTALL_LOCK.lock().await;
    crate::meeting_diarization::cancel_all()?;
    let _use = MODEL_USE.lock_or_recover();
    let _cache = CacheLease::acquire(true)?;
    // Invalidate on both sides of the deletion, including a partial failure.
    invalidate_verdict(&VERDICT);
    let result = remove_installed_models();
    invalidate_verdict(&VERDICT);
    result
}

fn remove_installed_models() -> Result<(), String> {
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

    /// A tree with every manifest path present as a regular file. Contents are
    /// placeholders: the cache tests inject the hashing step.
    fn fake_tree() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("temp dir");
        for entry in manifest() {
            let path = dir.path().join(&entry.path);
            fs::create_dir_all(path.parent().expect("parent")).expect("create parents");
            fs::write(path, b"model").expect("write file");
        }
        dir
    }

    fn counted(cache: &VerdictCache, root: &Path, verdict: bool, runs: &mut u32) -> bool {
        cached_verdict(cache, root, || {
            *runs += 1;
            verdict
        })
    }

    #[test]
    fn verdict_is_hashed_once_then_served_from_stat_only_checks() {
        let dir = fake_tree();
        let cache: VerdictCache = Mutex::new(None);
        let mut runs = 0;
        assert!(counted(&cache, dir.path(), true, &mut runs));
        for _ in 0..5 {
            assert!(counted(&cache, dir.path(), true, &mut runs));
        }
        assert_eq!(runs, 1, "repeat status polls must not hash");
    }

    #[test]
    fn rejected_verdict_is_cached_too() {
        let dir = fake_tree();
        let cache: VerdictCache = Mutex::new(None);
        let mut runs = 0;
        assert!(!counted(&cache, dir.path(), false, &mut runs));
        assert!(!counted(&cache, dir.path(), false, &mut runs));
        assert_eq!(runs, 1);
    }

    #[test]
    fn size_change_forces_a_rehash() {
        let dir = fake_tree();
        let cache: VerdictCache = Mutex::new(None);
        let mut runs = 0;
        assert!(counted(&cache, dir.path(), true, &mut runs));
        let first = dir.path().join(&manifest()[0].path);
        fs::write(&first, b"model, but longer").expect("grow file");
        assert!(!counted(&cache, dir.path(), false, &mut runs));
        assert_eq!(runs, 2);
    }

    #[test]
    fn mtime_change_with_same_size_forces_a_rehash() {
        let dir = fake_tree();
        let cache: VerdictCache = Mutex::new(None);
        let mut runs = 0;
        assert!(counted(&cache, dir.path(), true, &mut runs));
        let first = dir.path().join(&manifest()[0].path);
        let file = fs::OpenOptions::new()
            .write(true)
            .open(&first)
            .expect("open file");
        file.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(60))
            .expect("set mtime");
        assert!(!counted(&cache, dir.path(), false, &mut runs));
        assert_eq!(runs, 2);
    }

    #[test]
    fn missing_file_is_invalid_without_hashing() {
        let dir = fake_tree();
        let cache: VerdictCache = Mutex::new(None);
        fs::remove_file(dir.path().join(&manifest()[0].path)).expect("remove file");
        let mut runs = 0;
        assert!(!counted(&cache, dir.path(), true, &mut runs));
        assert_eq!(runs, 0);
    }

    #[test]
    fn download_and_remove_invalidate_the_cached_verdict() {
        let dir = fake_tree();
        let cache: VerdictCache = Mutex::new(None);
        let mut runs = 0;
        assert!(counted(&cache, dir.path(), true, &mut runs));
        // `download` and `remove_diarization_model` both call this on the
        // global cache around publishing and deleting the model directory.
        invalidate_verdict(&cache);
        assert!(cache.lock_or_recover().is_none());
        assert!(counted(&cache, dir.path(), true, &mut runs));
        assert_eq!(runs, 2);
    }

    #[test]
    fn cache_is_scoped_to_its_root() {
        let first = fake_tree();
        let second = fake_tree();
        let cache: VerdictCache = Mutex::new(None);
        let mut runs = 0;
        assert!(counted(&cache, first.path(), true, &mut runs));
        assert!(counted(&cache, second.path(), true, &mut runs));
        assert_eq!(runs, 2);
    }
}
