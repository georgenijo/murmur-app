//! Explicit, one-shot private capture for live dictation diagnostics.
//!
//! Ordinary telemetry remains content-free. This store receives transcript
//! text only after a local arm is claimed by one accepted live recording.

use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use crate::dictation_telemetry::{DictationErrorCode, DictationTerminalOutcome};
use crate::MutexExt;

const SCHEMA_VERSION: u32 = 1;
const MAX_CAPTURES: usize = 3;
const MAX_PRIVATE_TEXT_BYTES: usize = 8 * 1024;
const MAX_MODEL_ID_BYTES: usize = 128;
const ARM_DURATION_MS: i64 = 10 * 60 * 1_000;
const RETENTION_MS: i64 = 7 * 24 * 60 * 60 * 1_000;
static CAPTURE_TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BoundedPrivateTextV1 {
    pub(crate) text: String,
    pub(crate) truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DictationCaptureContentV1 {
    pub(crate) raw_text: BoundedPrivateTextV1,
    pub(crate) final_text: BoundedPrivateTextV1,
    pub(crate) model_id: String,
    pub(crate) total_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum DictationCaptureResultV1 {
    Success {
        #[serde(flatten)]
        content: DictationCaptureContentV1,
    },
    NoContent {
        outcome: String,
        error_code: String,
    },
}

impl DictationCaptureResultV1 {
    pub(crate) fn content(&self) -> Option<&DictationCaptureContentV1> {
        match self {
            Self::Success { content } => Some(content),
            Self::NoContent { .. } => None,
        }
    }

    fn outcome(&self) -> &str {
        match self {
            Self::Success { .. } => DictationTerminalOutcome::Success.as_str(),
            Self::NoContent { outcome, .. } => outcome,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DictationDiagnosticCaptureV1 {
    pub(crate) schema_version: u32,
    pub(crate) capture_id: String,
    pub(crate) recording_id: u64,
    pub(crate) captured_at_ms: i64,
    pub(crate) expires_at_ms: i64,
    pub(crate) result: DictationCaptureResultV1,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DictationDiagnosticCaptureSummaryV1 {
    pub(crate) capture_id: String,
    pub(crate) recording_id: u64,
    pub(crate) captured_at_ms: i64,
    pub(crate) expires_at_ms: i64,
    pub(crate) outcome: String,
    pub(crate) has_content: bool,
}

impl DictationDiagnosticCaptureV1 {
    fn summary(&self) -> DictationDiagnosticCaptureSummaryV1 {
        DictationDiagnosticCaptureSummaryV1 {
            capture_id: self.capture_id.clone(),
            recording_id: self.recording_id,
            captured_at_ms: self.captured_at_ms,
            expires_at_ms: self.expires_at_ms,
            outcome: self.result.outcome().to_string(),
            has_content: self.result.content().is_some(),
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "camelCase")]
pub(crate) enum DictationCaptureArmStatusV1 {
    Unarmed,
    Armed { expires_at_ms: i64 },
    Capturing { recording_id: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ArmState {
    Unarmed,
    Armed {
        expires_at_ms: i64,
    },
    Capturing {
        recording_id: u64,
        capture_id: String,
        captured_at_ms: i64,
    },
}

impl Default for ArmState {
    fn default() -> Self {
        Self::Unarmed
    }
}

#[derive(Default)]
struct Inner {
    root: Option<PathBuf>,
    arm: ArmState,
}

#[derive(Default)]
pub(crate) struct DictationDiagnostics {
    inner: Mutex<Inner>,
}

pub(crate) enum DictationCaptureCompletion<'a> {
    Success {
        raw_text: &'a str,
        final_text: &'a str,
        model_id: &'a str,
        total_ms: u64,
    },
    Terminal {
        outcome: DictationTerminalOutcome,
        error_code: DictationErrorCode,
    },
}

impl DictationDiagnostics {
    pub(crate) fn initialize(&self, root: PathBuf) -> Result<(), String> {
        ensure_private_dir(&root)?;
        ensure_private_dir(&root.join("captures"))?;
        let mut inner = self.inner.lock_or_recover();
        inner.root = Some(root);
        prune_captures(&inner)?;
        Ok(())
    }

    pub(crate) fn arm_next(&self) -> Result<DictationCaptureArmStatusV1, String> {
        let mut inner = self.inner.lock_or_recover();
        if inner.root.is_none() {
            return Err("dictation diagnostic capture store unavailable".to_string());
        }
        expire_unclaimed_arm(&mut inner.arm);
        if matches!(inner.arm, ArmState::Capturing { .. }) {
            return Err("a dictation diagnostic capture is already in progress".to_string());
        }
        let expires_at_ms = now_ms() + ARM_DURATION_MS;
        inner.arm = ArmState::Armed { expires_at_ms };
        Ok(DictationCaptureArmStatusV1::Armed { expires_at_ms })
    }

    pub(crate) fn arm_status(&self) -> DictationCaptureArmStatusV1 {
        let mut inner = self.inner.lock_or_recover();
        expire_unclaimed_arm(&mut inner.arm);
        match inner.arm {
            ArmState::Unarmed => DictationCaptureArmStatusV1::Unarmed,
            ArmState::Armed { expires_at_ms } => {
                DictationCaptureArmStatusV1::Armed { expires_at_ms }
            }
            ArmState::Capturing { recording_id, .. } => {
                DictationCaptureArmStatusV1::Capturing { recording_id }
            }
        }
    }

    pub(crate) fn claim(&self, recording_id: u64) -> bool {
        if recording_id == 0 {
            return false;
        }
        let mut inner = self.inner.lock_or_recover();
        expire_unclaimed_arm(&mut inner.arm);
        if !matches!(inner.arm, ArmState::Armed { .. }) {
            return false;
        }
        inner.arm = ArmState::Capturing {
            recording_id,
            capture_id: uuid::Uuid::new_v4().to_string(),
            captured_at_ms: now_ms(),
        };
        true
    }

    pub(crate) fn finish(
        &self,
        recording_id: u64,
        completion: DictationCaptureCompletion<'_>,
    ) -> Result<bool, String> {
        let mut inner = self.inner.lock_or_recover();
        let arm = std::mem::take(&mut inner.arm);
        let ArmState::Capturing {
            recording_id: owner,
            capture_id,
            captured_at_ms,
        } = arm
        else {
            inner.arm = arm;
            return Ok(false);
        };
        if owner != recording_id {
            inner.arm = ArmState::Capturing {
                recording_id: owner,
                capture_id,
                captured_at_ms,
            };
            return Ok(false);
        }

        let result = match completion {
            DictationCaptureCompletion::Success {
                raw_text,
                final_text,
                model_id,
                total_ms,
            } => DictationCaptureResultV1::Success {
                content: DictationCaptureContentV1 {
                    raw_text: bounded_private_text(raw_text),
                    final_text: bounded_private_text(final_text),
                    model_id: bounded_model_id(model_id),
                    total_ms,
                },
            },
            DictationCaptureCompletion::Terminal {
                outcome,
                error_code,
            } => DictationCaptureResultV1::NoContent {
                outcome: outcome.as_str().to_string(),
                error_code: error_code.as_str().to_string(),
            },
        };
        let capture = DictationDiagnosticCaptureV1 {
            schema_version: SCHEMA_VERSION,
            capture_id,
            recording_id,
            captured_at_ms,
            expires_at_ms: captured_at_ms + RETENTION_MS,
            result,
        };
        write_capture(&inner, &capture)?;
        prune_captures(&inner)?;
        Ok(true)
    }

    pub(crate) fn list_captures(&self) -> Result<Vec<DictationDiagnosticCaptureSummaryV1>, String> {
        let inner = self.inner.lock_or_recover();
        prune_captures(&inner)?;
        let mut captures = read_captures(&inner)?;
        captures.sort_by_key(|capture| std::cmp::Reverse(capture.captured_at_ms));
        Ok(captures
            .into_iter()
            .map(|capture| capture.summary())
            .collect())
    }

    pub(crate) fn get_capture(
        &self,
        capture_id: &str,
    ) -> Result<Option<DictationDiagnosticCaptureV1>, String> {
        validate_capture_id(capture_id)?;
        let inner = self.inner.lock_or_recover();
        prune_captures(&inner)?;
        read_capture_path(&capture_path(&inner, capture_id)?)
    }

    pub(crate) fn delete_capture(&self, capture_id: &str) -> Result<(), String> {
        validate_capture_id(capture_id)?;
        let inner = self.inner.lock_or_recover();
        let path = capture_path(&inner, capture_id)?;
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                Err("dictation diagnostic capture target refused".to_string())
            }
            Ok(_) => fs::remove_file(path)
                .map_err(|_| "dictation diagnostic capture could not be deleted".to_string()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err("dictation diagnostic capture could not be deleted".to_string()),
        }
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn expire_unclaimed_arm(state: &mut ArmState) {
    if matches!(state, ArmState::Armed { expires_at_ms } if *expires_at_ms <= now_ms()) {
        *state = ArmState::Unarmed;
    }
}

fn bounded_private_text(value: &str) -> BoundedPrivateTextV1 {
    if value.len() <= MAX_PRIVATE_TEXT_BYTES {
        return BoundedPrivateTextV1 {
            text: value.to_string(),
            truncated: false,
        };
    }
    let mut end = MAX_PRIVATE_TEXT_BYTES;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    BoundedPrivateTextV1 {
        text: value[..end].to_string(),
        truncated: true,
    }
}

fn bounded_model_id(value: &str) -> String {
    if value.len() <= MAX_MODEL_ID_BYTES {
        return value.to_string();
    }
    let mut end = MAX_MODEL_ID_BYTES;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

fn ensure_private_dir(path: &Path) -> Result<(), String> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("dictation diagnostic store target refused".to_string());
        }
    } else {
        fs::create_dir_all(path)
            .map_err(|_| "dictation diagnostic capture store unavailable".to_string())?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| "dictation diagnostic store permissions unavailable".to_string())?;
    }
    Ok(())
}

fn capture_root(inner: &Inner) -> Result<PathBuf, String> {
    inner
        .root
        .as_ref()
        .map(|root| root.join("captures"))
        .ok_or_else(|| "dictation diagnostic capture store unavailable".to_string())
}

fn capture_path(inner: &Inner, capture_id: &str) -> Result<PathBuf, String> {
    Ok(capture_root(inner)?.join(format!("{capture_id}.json")))
}

fn validate_capture_id(capture_id: &str) -> Result<(), String> {
    if uuid::Uuid::parse_str(capture_id)
        .is_ok_and(|parsed| parsed.hyphenated().to_string() == capture_id)
    {
        Ok(())
    } else {
        Err("invalid dictation diagnostic capture id".to_string())
    }
}

fn validate_capture(capture: &DictationDiagnosticCaptureV1) -> Result<(), String> {
    if capture.schema_version != SCHEMA_VERSION
        || capture.recording_id == 0
        || capture.expires_at_ms <= capture.captured_at_ms
    {
        return Err("dictation diagnostic capture invalid".to_string());
    }
    validate_capture_id(&capture.capture_id)?;
    if let Some(content) = capture.result.content() {
        if content.raw_text.text.len() > MAX_PRIVATE_TEXT_BYTES
            || content.final_text.text.len() > MAX_PRIVATE_TEXT_BYTES
            || content.model_id.len() > MAX_MODEL_ID_BYTES
        {
            return Err("dictation diagnostic capture invalid".to_string());
        }
    }
    Ok(())
}

fn write_capture(inner: &Inner, capture: &DictationDiagnosticCaptureV1) -> Result<(), String> {
    validate_capture(capture)?;
    let root = capture_root(inner)?;
    let path = capture_path(inner, &capture.capture_id)?;
    let payload = serde_json::to_vec(capture)
        .map_err(|_| "dictation diagnostic capture could not be encoded".to_string())?;
    let (temp_path, mut file) = create_capture_temp(&root, &capture.capture_id)?;
    let result = (|| {
        file.write_all(&payload)
            .map_err(|_| "dictation diagnostic capture could not be written".to_string())?;
        file.flush()
            .map_err(|_| "dictation diagnostic capture could not be written".to_string())?;
        file.sync_all()
            .map_err(|_| "dictation diagnostic capture could not be written".to_string())?;
        drop(file);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err("dictation diagnostic capture target refused".to_string());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("dictation diagnostic capture unavailable".to_string()),
        }
        fs::rename(&temp_path, &path)
            .map_err(|_| "dictation diagnostic capture could not be written".to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
                .map_err(|_| "dictation diagnostic store permissions unavailable".to_string())?;
            std::fs::File::open(&root)
                .and_then(|directory| directory.sync_all())
                .map_err(|_| "dictation diagnostic capture could not be written".to_string())?;
        }
        Ok(())
    })();
    if result.is_err() {
        remove_regular_store_file(&temp_path);
    }
    result
}

fn create_capture_temp(root: &Path, capture_id: &str) -> Result<(PathBuf, fs::File), String> {
    for _ in 0..16 {
        let sequence = CAPTURE_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = root.join(format!(
            ".capture-{capture_id}-{}-{sequence}.tmp",
            std::process::id()
        ));
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .mode(0o600)
                .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
        }
        match options.open(&path) {
            Ok(file) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if file
                        .set_permissions(fs::Permissions::from_mode(0o600))
                        .is_err()
                    {
                        drop(file);
                        remove_regular_store_file(&path);
                        return Err(
                            "dictation diagnostic store permissions unavailable".to_string()
                        );
                    }
                }
                return Ok((path, file));
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err("dictation diagnostic capture unavailable".to_string()),
        }
    }
    Err("dictation diagnostic capture unavailable".to_string())
}

fn open_private_read(path: &Path) -> Result<fs::File, String> {
    if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err("dictation diagnostic capture target refused".to_string());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    options
        .open(path)
        .map_err(|_| "dictation diagnostic capture unavailable".to_string())
}

fn read_capture_path(path: &Path) -> Result<Option<DictationDiagnosticCaptureV1>, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("dictation diagnostic capture unavailable".to_string()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("dictation diagnostic capture target refused".to_string());
    }
    let file = open_private_read(path)?;
    let mut bytes = Vec::new();
    file.take((2 * MAX_PRIVATE_TEXT_BYTES + 4096) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "dictation diagnostic capture unavailable".to_string())?;
    let capture: DictationDiagnosticCaptureV1 = serde_json::from_slice(&bytes)
        .map_err(|_| "dictation diagnostic capture invalid".to_string())?;
    validate_capture(&capture)?;
    if path.file_name().and_then(|value| value.to_str())
        != Some(format!("{}.json", capture.capture_id).as_str())
    {
        return Err("dictation diagnostic capture identity mismatch".to_string());
    }
    Ok(Some(capture))
}

fn read_captures(inner: &Inner) -> Result<Vec<DictationDiagnosticCaptureV1>, String> {
    let root = capture_root(inner)?;
    let mut captures = Vec::new();
    for entry in fs::read_dir(root)
        .map_err(|_| "dictation diagnostic capture store unavailable".to_string())?
    {
        let Ok(entry) = entry else {
            continue;
        };
        let path = entry.path();
        let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if file_name.starts_with(".capture-") && file_name.ends_with(".tmp") {
            remove_regular_store_file(&path);
            continue;
        }
        let Some(capture_id) = file_name.strip_suffix(".json") else {
            continue;
        };
        if validate_capture_id(capture_id).is_err() {
            continue;
        }
        match read_capture_path(&path) {
            Ok(Some(capture)) => captures.push(capture),
            Ok(None) => {}
            Err(_) => remove_regular_store_file(&path),
        }
    }
    Ok(captures)
}

fn remove_regular_store_file(path: &Path) {
    if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_file()) {
        let _ = fs::remove_file(path);
    }
}

fn prune_captures(inner: &Inner) -> Result<(), String> {
    let mut captures = read_captures(inner)?;
    captures.sort_by_key(|capture| std::cmp::Reverse(capture.captured_at_ms));
    let now = now_ms();
    for (index, capture) in captures.into_iter().enumerate() {
        if index >= MAX_CAPTURES || capture.expires_at_ms <= now {
            let path = capture_path(inner, &capture.capture_id)?;
            if !fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_symlink())
            {
                let _ = fs::remove_file(path);
            }
        }
    }
    Ok(())
}
