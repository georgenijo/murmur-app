//! Privacy-bounded diagnostics for selected-text transforms.
//!
//! `TransformAttemptV1` is always content-free. Exact selection, instruction,
//! and output text is only written after an explicit one-shot arm, to a
//! separate local store with restrictive permissions and bounded retention.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use crate::MutexExt;

const SCHEMA_VERSION: u32 = 1;
const MAX_ATTEMPTS: usize = 200;
const MAX_CAPTURES: usize = 3;
const CAPTURE_ARM_MS: i64 = 10 * 60 * 1_000;
const CAPTURE_RETENTION_MS: i64 = 7 * 24 * 60 * 60 * 1_000;
static CAPTURE_TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TransformPhaseV1 {
    pub phase: String,
    pub outcome: String,
    pub duration_ms: Option<u64>,
    pub error_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TransformAttemptV1 {
    pub schema_version: u32,
    pub transform_pass_id: u64,
    pub started_at_ms: i64,
    pub finished_at_ms: Option<i64>,
    pub outcome: String,
    pub selection_source: Option<String>,
    pub selection_result: Option<String>,
    pub selection_size_bucket: Option<String>,
    pub range_available: Option<bool>,
    pub bounds_available: Option<bool>,
    pub instruction_confidence_bucket: Option<String>,
    pub model_warm_state: Option<String>,
    pub output_token_count: Option<u32>,
    pub finish_reason: Option<String>,
    pub process_exit_code: Option<i32>,
    pub process_exit_signal: Option<i32>,
    pub phases: Vec<TransformPhaseV1>,
}

impl TransformAttemptV1 {
    fn new(transform_pass_id: u64, now_ms: i64) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            transform_pass_id,
            started_at_ms: now_ms,
            finished_at_ms: None,
            outcome: "inProgress".to_string(),
            selection_source: None,
            selection_result: None,
            selection_size_bucket: None,
            range_available: None,
            bounds_available: None,
            instruction_confidence_bucket: None,
            model_warm_state: None,
            output_token_count: None,
            finish_reason: None,
            process_exit_code: None,
            process_exit_signal: None,
            phases: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TransformAttemptListV1 {
    pub schema_version: u32,
    pub attempts: Vec<TransformAttemptV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CaptureArmStatusV1 {
    pub armed: bool,
    pub expires_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticCaptureSummaryV1 {
    pub capture_id: String,
    pub transform_pass_id: u64,
    pub captured_at_ms: i64,
    pub expires_at_ms: i64,
    pub outcome: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticCaptureV1 {
    pub schema_version: u32,
    pub capture_id: String,
    pub transform_pass_id: u64,
    pub captured_at_ms: i64,
    pub expires_at_ms: i64,
    pub outcome: String,
    pub selection: Option<String>,
    pub instruction: Option<String>,
    pub output: Option<String>,
    pub phases: Vec<TransformPhaseV1>,
}

impl DiagnosticCaptureV1 {
    fn summary(&self) -> DiagnosticCaptureSummaryV1 {
        DiagnosticCaptureSummaryV1 {
            capture_id: self.capture_id.clone(),
            transform_pass_id: self.transform_pass_id,
            captured_at_ms: self.captured_at_ms,
            expires_at_ms: self.expires_at_ms,
            outcome: self.outcome.clone(),
        }
    }
}

#[derive(Default)]
struct Inner {
    root: Option<PathBuf>,
    attempts: Vec<TransformAttemptV1>,
    active: HashMap<u64, TransformAttemptV1>,
    /// Terminal pass IDs from this process only. Transform pass IDs are
    /// process-local and restart at 1, so persisted attempts must never seed
    /// this set during initialization.
    finished: BTreeSet<u64>,
    armed_until_ms: Option<i64>,
    captures: HashMap<u64, DiagnosticCaptureV1>,
}

#[derive(Default)]
pub struct TransformDiagnostics {
    inner: Mutex<Inner>,
}

impl TransformDiagnostics {
    pub fn initialize(&self, root: PathBuf) -> Result<(), String> {
        ensure_private_dir(&root)?;
        let capture_root = root.join("transform-captures");
        ensure_private_dir(&capture_root)?;
        let mut inner = self.inner.lock_or_recover();
        inner.root = Some(root);
        inner.attempts = read_attempts(&inner)?;
        prune_captures(&mut inner)?;
        Ok(())
    }

    pub fn begin(&self, transform_pass_id: u64) {
        if transform_pass_id == 0 {
            return;
        }
        let now = now_ms();
        let mut inner = self.inner.lock_or_recover();
        // One physical pass owns one diagnostics attempt. A delayed duplicate
        // start command must neither replace live phase data nor resurrect a
        // terminal pass as a new inProgress entry.
        if inner.active.contains_key(&transform_pass_id)
            || inner.finished.contains(&transform_pass_id)
        {
            return;
        }
        inner.active.insert(
            transform_pass_id,
            TransformAttemptV1::new(transform_pass_id, now),
        );
        if inner
            .armed_until_ms
            .take()
            .is_some_and(|expiry| expiry >= now)
        {
            inner.captures.insert(
                transform_pass_id,
                DiagnosticCaptureV1 {
                    schema_version: SCHEMA_VERSION,
                    capture_id: format!("{transform_pass_id}-{now}"),
                    transform_pass_id,
                    captured_at_ms: now,
                    expires_at_ms: now + CAPTURE_RETENTION_MS,
                    outcome: "inProgress".to_string(),
                    selection: None,
                    instruction: None,
                    output: None,
                    phases: Vec::new(),
                },
            );
        }
    }

    /// Atomically terminalize a still-active attempt with exactly one terminal
    /// phase. Returns false when another path already terminalized this pass.
    ///
    /// This is used by queued start cancellation/supersession, where active
    /// transform ownership may already belong to a newer pass. Diagnostics
    /// ownership is keyed independently by `transform_pass_id`, so clearing or
    /// retaining the app's active owner must never strand this attempt.
    pub fn terminalize_active(&self, transform_pass_id: u64, phase: &str, outcome: &str) -> bool {
        let mut inner = self.inner.lock_or_recover();
        let Some(mut attempt) = inner.active.remove(&transform_pass_id) else {
            return false;
        };
        let now = now_ms();
        let terminal_phase = TransformPhaseV1 {
            phase: phase.to_string(),
            outcome: "completed".to_string(),
            duration_ms: None,
            error_code: None,
        };
        attempt.phases.push(terminal_phase.clone());
        attempt.finished_at_ms = Some(now);
        attempt.outcome = outcome.to_string();
        inner.attempts.push(attempt);
        remember_finished(&mut inner, transform_pass_id);
        inner.attempts.sort_by_key(|attempt| attempt.started_at_ms);
        if inner.attempts.len() > MAX_ATTEMPTS {
            let excess = inner.attempts.len() - MAX_ATTEMPTS;
            inner.attempts.drain(0..excess);
        }
        let _ = write_attempts(&inner);

        let capture = inner.captures.get_mut(&transform_pass_id).map(|capture| {
            capture.phases.push(terminal_phase);
            capture.outcome = outcome.to_string();
            capture.clone()
        });
        if let Some(capture) = capture {
            let _ = write_capture(&inner, &capture);
            let _ = prune_captures(&mut inner);
        }
        true
    }

    pub fn phase(
        &self,
        transform_pass_id: u64,
        phase: &str,
        outcome: &str,
        duration_ms: Option<u64>,
        error_code: Option<&str>,
    ) {
        let mut inner = self.inner.lock_or_recover();
        let entry = TransformPhaseV1 {
            phase: phase.to_string(),
            outcome: outcome.to_string(),
            duration_ms,
            error_code: error_code.map(str::to_string),
        };
        let mut persisted_attempt_changed = false;
        if let Some(attempt) = inner.active.get_mut(&transform_pass_id) {
            attempt.phases.push(entry.clone());
        } else if let Some(attempt) = inner
            .attempts
            .iter_mut()
            .rev()
            .find(|attempt| attempt.transform_pass_id == transform_pass_id)
        {
            attempt.phases.push(entry.clone());
            persisted_attempt_changed = true;
        }
        if let Some(capture) = inner.captures.get_mut(&transform_pass_id) {
            capture.phases.push(entry);
        }
        if persisted_attempt_changed {
            let _ = write_attempts(&inner);
        }
        if let Some(capture) = inner.captures.get(&transform_pass_id) {
            let _ = write_capture(&inner, capture);
        }
    }

    pub fn selection(
        &self,
        transform_pass_id: u64,
        result: Result<(&str, &str, bool, bool, &str), &str>,
        raw: Option<&str>,
    ) {
        let mut inner = self.inner.lock_or_recover();
        if let Some(attempt) = inner.active.get_mut(&transform_pass_id) {
            match result {
                Ok((source, bucket, range, bounds, outcome)) => {
                    attempt.selection_source = Some(source.to_string());
                    attempt.selection_result = Some(outcome.to_string());
                    attempt.selection_size_bucket = Some(bucket.to_string());
                    attempt.range_available = Some(range);
                    attempt.bounds_available = Some(bounds);
                }
                Err(error) => attempt.selection_result = Some(error.to_string()),
            }
        }
        if let (Some(capture), Some(raw)) = (inner.captures.get_mut(&transform_pass_id), raw) {
            capture.selection = Some(raw.to_string());
        }
    }

    pub fn instruction(&self, transform_pass_id: u64, raw: &str) {
        let mut inner = self.inner.lock_or_recover();
        let mut persisted_attempt_changed = false;
        if let Some(attempt) = inner.active.get_mut(&transform_pass_id) {
            // The current ASR seam does not expose calibrated confidence.
            attempt.instruction_confidence_bucket = Some("unavailable".to_string());
        } else if let Some(attempt) = inner
            .attempts
            .iter_mut()
            .rev()
            .find(|attempt| attempt.transform_pass_id == transform_pass_id)
        {
            attempt.instruction_confidence_bucket = Some("unavailable".to_string());
            persisted_attempt_changed = true;
        }
        if let Some(capture) = inner.captures.get_mut(&transform_pass_id) {
            capture.instruction = Some(raw.to_string());
        }
        if persisted_attempt_changed {
            let _ = write_attempts(&inner);
        }
        if let Some(capture) = inner.captures.get(&transform_pass_id) {
            let _ = write_capture(&inner, capture);
        }
    }

    pub fn sidecar_result(
        &self,
        transform_pass_id: u64,
        warm_state: Option<&str>,
        output_tokens: Option<u32>,
        finish_reason: Option<&str>,
        raw_output: Option<&str>,
    ) {
        let mut inner = self.inner.lock_or_recover();
        let mut persisted_attempt_changed = false;
        if let Some(attempt) = inner.active.get_mut(&transform_pass_id) {
            attempt.model_warm_state = warm_state.map(str::to_string);
            attempt.output_token_count = output_tokens;
            attempt.finish_reason = finish_reason.map(str::to_string);
        } else if let Some(attempt) = inner
            .attempts
            .iter_mut()
            .rev()
            .find(|attempt| attempt.transform_pass_id == transform_pass_id)
        {
            attempt.model_warm_state = warm_state.map(str::to_string);
            attempt.output_token_count = output_tokens;
            attempt.finish_reason = finish_reason.map(str::to_string);
            persisted_attempt_changed = true;
        }
        if let (Some(capture), Some(output)) =
            (inner.captures.get_mut(&transform_pass_id), raw_output)
        {
            capture.output = Some(output.to_string());
        }
        if persisted_attempt_changed {
            let _ = write_attempts(&inner);
        }
        if let Some(capture) = inner.captures.get(&transform_pass_id) {
            let _ = write_capture(&inner, capture);
        }
    }

    pub fn process_exit(
        &self,
        transform_pass_id: u64,
        exit_code: Option<i32>,
        signal: Option<i32>,
    ) {
        let mut inner = self.inner.lock_or_recover();
        let mut persisted_attempt_changed = false;
        if let Some(attempt) = inner.active.get_mut(&transform_pass_id) {
            attempt.process_exit_code = exit_code;
            attempt.process_exit_signal = signal;
        } else if let Some(attempt) = inner
            .attempts
            .iter_mut()
            .rev()
            .find(|attempt| attempt.transform_pass_id == transform_pass_id)
        {
            attempt.process_exit_code = exit_code;
            attempt.process_exit_signal = signal;
            persisted_attempt_changed = true;
        }
        if persisted_attempt_changed {
            let _ = write_attempts(&inner);
        }
    }

    pub fn finish(&self, transform_pass_id: u64, outcome: &str) {
        let mut inner = self.inner.lock_or_recover();
        let now = now_ms();
        let mut recorded_outcome = outcome.to_string();
        if let Some(mut attempt) = inner.active.remove(&transform_pass_id) {
            attempt.finished_at_ms = Some(now);
            attempt.outcome = outcome.to_string();
            inner.attempts.push(attempt);
            remember_finished(&mut inner, transform_pass_id);
            inner.attempts.sort_by_key(|attempt| attempt.started_at_ms);
            if inner.attempts.len() > MAX_ATTEMPTS {
                let excess = inner.attempts.len() - MAX_ATTEMPTS;
                inner.attempts.drain(0..excess);
            }
            let _ = write_attempts(&inner);
        } else if let Some(attempt) = inner
            .attempts
            .iter_mut()
            .rev()
            .find(|attempt| attempt.transform_pass_id == transform_pass_id)
        {
            // Cancellation/supersession can win while an async capture future
            // is still unwinding. A late `Aborted -> failed` cleanup must not
            // downgrade the terminal user outcome already persisted.
            if matches!(attempt.outcome.as_str(), "cancelled" | "superseded")
                && attempt.outcome != outcome
            {
                recorded_outcome = attempt.outcome.clone();
            } else {
                attempt.finished_at_ms = Some(now);
                attempt.outcome = outcome.to_string();
                let _ = write_attempts(&inner);
            }
        }
        if let Some(capture) = inner.captures.get_mut(&transform_pass_id) {
            capture.outcome = recorded_outcome;
            let capture = capture.clone();
            let _ = write_capture(&inner, &capture);
            let _ = prune_captures(&mut inner);
        }
    }

    pub fn arm_next(&self) -> Result<CaptureArmStatusV1, String> {
        let mut inner = self.inner.lock_or_recover();
        if inner.root.is_none() {
            return Err("diagnostic capture store unavailable".to_string());
        }
        let expiry = now_ms() + CAPTURE_ARM_MS;
        inner.armed_until_ms = Some(expiry);
        Ok(CaptureArmStatusV1 {
            armed: true,
            expires_at_ms: Some(expiry),
        })
    }

    pub fn arm_status(&self) -> CaptureArmStatusV1 {
        let mut inner = self.inner.lock_or_recover();
        let now = now_ms();
        if inner.armed_until_ms.is_some_and(|expiry| expiry < now) {
            inner.armed_until_ms = None;
        }
        CaptureArmStatusV1 {
            armed: inner.armed_until_ms.is_some(),
            expires_at_ms: inner.armed_until_ms,
        }
    }

    pub fn list_attempts(&self, limit: usize) -> TransformAttemptListV1 {
        let inner = self.inner.lock_or_recover();
        let attempts = inner
            .attempts
            .iter()
            .rev()
            .take(limit.min(MAX_ATTEMPTS))
            .cloned()
            .collect();
        TransformAttemptListV1 {
            schema_version: SCHEMA_VERSION,
            attempts,
        }
    }

    pub fn list_captures(&self) -> Result<Vec<DiagnosticCaptureSummaryV1>, String> {
        let mut inner = self.inner.lock_or_recover();
        prune_captures(&mut inner)?;
        let mut captures = read_captures(&inner)?;
        captures.sort_by_key(|capture| std::cmp::Reverse(capture.captured_at_ms));
        Ok(captures
            .into_iter()
            .map(|capture| capture.summary())
            .collect())
    }

    pub fn get_capture(&self, capture_id: &str) -> Result<Option<DiagnosticCaptureV1>, String> {
        validate_capture_id(capture_id)?;
        let mut inner = self.inner.lock_or_recover();
        prune_captures(&mut inner)?;
        let path = capture_path(&inner, capture_id)?;
        read_capture_path(&path)
    }

    pub fn delete_capture(&self, capture_id: &str) -> Result<(), String> {
        validate_capture_id(capture_id)?;
        let mut inner = self.inner.lock_or_recover();
        inner
            .captures
            .retain(|_, capture| capture.capture_id != capture_id);
        let path = capture_path(&inner, capture_id)?;
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                Err("diagnostic capture target refused".to_string())
            }
            Ok(_) => fs::remove_file(path)
                .map_err(|_| "diagnostic capture could not be deleted".to_string()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err("diagnostic capture could not be deleted".to_string()),
        }
    }
}

fn remember_finished(inner: &mut Inner, transform_pass_id: u64) {
    inner.finished.insert(transform_pass_id);
    while inner.finished.len() > MAX_ATTEMPTS {
        inner.finished.pop_first();
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn ensure_private_dir(path: &Path) -> Result<(), String> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("diagnostic store target refused".to_string());
        }
    } else {
        fs::create_dir_all(path).map_err(|_| "diagnostic store unavailable".to_string())?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| "diagnostic store permissions unavailable".to_string())?;
    }
    Ok(())
}

fn open_private(path: &Path) -> Result<std::fs::File, String> {
    if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err("diagnostic file target refused".to_string());
    }
    let mut options = OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options
        .open(path)
        .map_err(|_| "diagnostic file unavailable".to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|_| "diagnostic file permissions unavailable".to_string())?;
    }
    Ok(file)
}

fn open_private_read(path: &Path, unavailable: &'static str) -> Result<std::fs::File, String> {
    if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err("diagnostic file target refused".to_string());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    options.open(path).map_err(|_| unavailable.to_string())
}

fn attempts_path(inner: &Inner) -> Result<PathBuf, String> {
    inner
        .root
        .as_ref()
        .map(|root| root.join("transform-attempts.json"))
        .ok_or_else(|| "diagnostic store unavailable".to_string())
}

fn capture_root(inner: &Inner) -> Result<PathBuf, String> {
    inner
        .root
        .as_ref()
        .map(|root| root.join("transform-captures"))
        .ok_or_else(|| "diagnostic capture store unavailable".to_string())
}

fn capture_path(inner: &Inner, capture_id: &str) -> Result<PathBuf, String> {
    Ok(capture_root(inner)?.join(format!("{capture_id}.json")))
}

fn write_attempts(inner: &Inner) -> Result<(), String> {
    let path = attempts_path(inner)?;
    let payload = serde_json::to_vec(&TransformAttemptListV1 {
        schema_version: SCHEMA_VERSION,
        attempts: inner.attempts.clone(),
    })
    .map_err(|_| "diagnostic records could not be encoded".to_string())?;
    let mut file = open_private(&path)?;
    file.write_all(&payload)
        .map_err(|_| "diagnostic records could not be written".to_string())
}

fn read_attempts(inner: &Inner) -> Result<Vec<TransformAttemptV1>, String> {
    let path = attempts_path(inner)?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    if fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err("diagnostic file target refused".to_string());
    }
    let mut file = open_private_read(&path, "diagnostic records unavailable")?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|_| "diagnostic records unavailable".to_string())?;
    let mut list: TransformAttemptListV1 =
        serde_json::from_slice(&bytes).map_err(|_| "diagnostic records invalid".to_string())?;
    if list.schema_version != SCHEMA_VERSION {
        return Err("diagnostic records version unsupported".to_string());
    }
    if list.attempts.len() > MAX_ATTEMPTS {
        list.attempts.drain(0..list.attempts.len() - MAX_ATTEMPTS);
    }
    Ok(list.attempts)
}

fn write_capture(inner: &Inner, capture: &DiagnosticCaptureV1) -> Result<(), String> {
    validate_capture_id(&capture.capture_id)?;
    let root = capture_root(inner)?;
    let path = capture_path(inner, &capture.capture_id)?;
    let payload = serde_json::to_vec(capture)
        .map_err(|_| "diagnostic capture could not be encoded".to_string())?;
    let (temp_path, mut file) = create_capture_temp(&root, &capture.capture_id)?;
    let result = (|| {
        file.write_all(&payload)
            .map_err(|_| "diagnostic capture could not be written".to_string())?;
        file.flush()
            .map_err(|_| "diagnostic capture could not be written".to_string())?;
        file.sync_all()
            .map_err(|_| "diagnostic capture could not be written".to_string())?;
        drop(file);

        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err("diagnostic capture target refused".to_string());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("diagnostic capture unavailable".to_string()),
        }

        fs::rename(&temp_path, &path)
            .map_err(|_| "diagnostic capture could not be written".to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
                .map_err(|_| "diagnostic capture permissions unavailable".to_string())?;
            std::fs::File::open(&root)
                .and_then(|directory| directory.sync_all())
                .map_err(|_| "diagnostic capture could not be written".to_string())?;
        }
        Ok(())
    })();
    if result.is_err() {
        remove_regular_store_file(&temp_path);
    }
    result
}

fn create_capture_temp(root: &Path, capture_id: &str) -> Result<(PathBuf, std::fs::File), String> {
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
                        return Err("diagnostic capture permissions unavailable".to_string());
                    }
                }
                return Ok((path, file));
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err("diagnostic capture unavailable".to_string()),
        }
    }
    Err("diagnostic capture unavailable".to_string())
}

fn validate_capture_id(capture_id: &str) -> Result<(), String> {
    if capture_id.is_empty()
        || capture_id.len() > 64
        || !capture_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'-')
    {
        return Err("invalid diagnostic capture id".to_string());
    }
    Ok(())
}

fn read_capture_path(path: &Path) -> Result<Option<DiagnosticCaptureV1>, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("diagnostic capture unavailable".to_string()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("diagnostic capture target refused".to_string());
    }
    let mut file = open_private_read(path, "diagnostic capture unavailable")?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|_| "diagnostic capture unavailable".to_string())?;
    let capture: DiagnosticCaptureV1 =
        serde_json::from_slice(&bytes).map_err(|_| "diagnostic capture invalid".to_string())?;
    if capture.schema_version != SCHEMA_VERSION {
        return Err("diagnostic capture version unsupported".to_string());
    }
    validate_capture_id(&capture.capture_id)?;
    if path.file_name().and_then(|value| value.to_str())
        != Some(format!("{}.json", capture.capture_id).as_str())
    {
        return Err("diagnostic capture identity mismatch".to_string());
    }
    Ok(Some(capture))
}

fn read_captures(inner: &Inner) -> Result<Vec<DiagnosticCaptureV1>, String> {
    let root = capture_root(inner)?;
    let mut captures = Vec::new();
    for entry in
        fs::read_dir(root).map_err(|_| "diagnostic capture store unavailable".to_string())?
    {
        let Ok(entry) = entry else {
            continue;
        };
        let path = entry.path();
        let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if is_capture_temp_name(file_name) {
            remove_regular_store_file(&path);
            continue;
        }
        if capture_id_from_file_name(file_name).is_none() {
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

fn capture_id_from_file_name(file_name: &str) -> Option<&str> {
    let capture_id = file_name.strip_suffix(".json")?;
    validate_capture_id(capture_id).ok()?;
    Some(capture_id)
}

fn is_capture_temp_name(file_name: &str) -> bool {
    file_name
        .strip_prefix(".capture-")
        .and_then(|value| value.strip_suffix(".tmp"))
        .is_some_and(|value| {
            !value.is_empty()
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || byte == b'-')
        })
}

fn remove_regular_store_file(path: &Path) {
    if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_file()) {
        let _ = fs::remove_file(path);
    }
}

fn prune_captures(inner: &mut Inner) -> Result<(), String> {
    let mut captures = read_captures(inner)?;
    captures.sort_by_key(|capture| std::cmp::Reverse(capture.captured_at_ms));
    let now = now_ms();
    for (index, capture) in captures.into_iter().enumerate() {
        if index >= MAX_CAPTURES || capture.expires_at_ms <= now {
            inner
                .captures
                .retain(|_, active| active.capture_id != capture.capture_id);
            let path = capture_path(inner, &capture.capture_id)?;
            if !fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_symlink())
            {
                let _ = fs::remove_file(path);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_is_off_by_default_one_shot_and_reviewable() {
        let temp = tempfile::tempdir().unwrap();
        let store = TransformDiagnostics::default();
        store.initialize(temp.path().join("diagnostics")).unwrap();
        assert!(!store.arm_status().armed);

        let armed = store.arm_next().unwrap();
        assert!(armed.armed);
        store.begin(41);
        assert!(!store.arm_status().armed);
        store.selection(
            41,
            Ok(("accessibility", "1-16", true, true, "success")),
            Some("private input"),
        );
        store.instruction(41, "make it shorter");
        store.sidecar_result(
            41,
            Some("cold"),
            Some(4),
            Some("stop"),
            Some("private output"),
        );
        store.finish(41, "ready");

        store.begin(42);
        store.finish(42, "failed");
        let summaries = store.list_captures().unwrap();
        assert_eq!(summaries.len(), 1);
        let capture = store
            .get_capture(&summaries[0].capture_id)
            .unwrap()
            .unwrap();
        assert_eq!(capture.selection.as_deref(), Some("private input"));
        assert_eq!(capture.instruction.as_deref(), Some("make it shorter"));
        assert_eq!(capture.output.as_deref(), Some("private output"));
        store.delete_capture(&capture.capture_id).unwrap();
        store.phase(41, "apply", "completed", Some(1), None);
        store.finish(41, "applied");
        assert!(store.list_captures().unwrap().is_empty());
    }

    #[test]
    fn arm_expires_and_retention_is_capped_at_three() {
        let temp = tempfile::tempdir().unwrap();
        let store = TransformDiagnostics::default();
        store.initialize(temp.path().join("diagnostics")).unwrap();

        store.inner.lock_or_recover().armed_until_ms = Some(now_ms() - 1);
        store.begin(1);
        store.finish(1, "cancelled");
        assert!(store.list_captures().unwrap().is_empty());

        for pass in 2..=5 {
            store.arm_next().unwrap();
            store.begin(pass);
            store.finish(pass, if pass == 5 { "failed" } else { "ready" });
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let captures = store.list_captures().unwrap();
        assert_eq!(captures.len(), MAX_CAPTURES);
        assert!(captures
            .iter()
            .all(|capture| capture.transform_pass_id >= 3));

        let attempts = store.list_attempts(10);
        assert_eq!(attempts.attempts.len(), 5);
        assert!(attempts
            .attempts
            .iter()
            .any(|attempt| { attempt.transform_pass_id == 1 && attempt.outcome == "cancelled" }));
    }

    #[test]
    fn cancel_during_capture_cannot_be_downgraded_by_late_abort() {
        let temp = tempfile::tempdir().unwrap();
        let store = TransformDiagnostics::default();
        store.initialize(temp.path().join("diagnostics")).unwrap();

        store.arm_next().unwrap();
        store.begin(88);
        store.phase(88, "cancellation", "completed", None, None);
        store.finish(88, "cancelled");

        // Mirrors start_transform_capture returning Aborted after the
        // cancellation path already terminalized and cleared the pass.
        store.finish(88, "failed");

        let attempt = store
            .list_attempts(10)
            .attempts
            .into_iter()
            .find(|attempt| attempt.transform_pass_id == 88)
            .unwrap();
        assert_eq!(attempt.outcome, "cancelled");
        let capture = store.list_captures().unwrap().pop().unwrap();
        assert_eq!(capture.outcome, "cancelled");
    }

    #[test]
    fn terminalize_active_is_idempotent_and_begin_cannot_resurrect_the_pass() {
        let store = TransformDiagnostics::default();
        store.begin(91);
        store.begin(91);

        assert!(store.terminalize_active(91, "cancellation", "cancelled"));
        assert!(!store.terminalize_active(91, "supersession", "superseded"));
        store.begin(91);

        let inner = store.inner.lock_or_recover();
        assert!(inner.active.is_empty());
        assert_eq!(inner.attempts.len(), 1);
        assert_eq!(inner.attempts[0].outcome, "cancelled");
        assert_eq!(
            inner.attempts[0]
                .phases
                .iter()
                .filter(|phase| phase.phase == "cancellation")
                .count(),
            1
        );
        assert!(!inner.attempts[0]
            .phases
            .iter()
            .any(|phase| phase.phase == "supersession"));
    }

    #[test]
    fn terminalized_attempts_are_capped_and_leave_no_active_entries() {
        let store = TransformDiagnostics::default();
        for transform_pass_id in 1..=(MAX_ATTEMPTS as u64 + 25) {
            store.begin(transform_pass_id);
            assert!(store.terminalize_active(transform_pass_id, "supersession", "superseded"));
        }

        let inner = store.inner.lock_or_recover();
        assert!(inner.active.is_empty());
        assert_eq!(inner.attempts.len(), MAX_ATTEMPTS);
        assert_eq!(inner.finished.len(), MAX_ATTEMPTS);
        assert_eq!(
            inner
                .attempts
                .first()
                .map(|attempt| attempt.transform_pass_id),
            Some(26)
        );
        assert_eq!(inner.finished.first().copied(), Some(26));
        assert!(inner
            .attempts
            .iter()
            .all(|attempt| attempt.outcome == "superseded"));
    }

    #[test]
    fn persisted_process_local_pass_id_can_begin_again_after_restart() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("diagnostics");

        let first_process = TransformDiagnostics::default();
        first_process.initialize(root.clone()).unwrap();
        first_process.begin(1);
        first_process.finish(1, "cancelled");
        drop(first_process);

        let next_process = TransformDiagnostics::default();
        next_process.initialize(root).unwrap();
        next_process.begin(1);
        assert!(next_process.inner.lock_or_recover().active.contains_key(&1));
        assert!(next_process.terminalize_active(1, "cancellation", "cancelled"));

        let attempts = next_process.list_attempts(10).attempts;
        assert_eq!(
            attempts
                .iter()
                .filter(|attempt| attempt.transform_pass_id == 1)
                .count(),
            2
        );
        assert!(attempts
            .iter()
            .all(|attempt| attempt.finished_at_ms.is_some()));
    }

    #[test]
    fn late_updates_for_reused_persisted_pass_id_target_latest_attempt_only() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("diagnostics");

        let first_process = TransformDiagnostics::default();
        first_process.initialize(root.clone()).unwrap();
        first_process.begin(1);
        first_process.phase(1, "firstProcess", "completed", None, None);
        first_process.finish(1, "ready");
        drop(first_process);

        let next_process = TransformDiagnostics::default();
        next_process.initialize(root).unwrap();
        next_process.begin(1);
        next_process.phase(1, "nextProcess", "completed", None, None);
        next_process.finish(1, "ready");

        // Mirrors late cancellation/unwind metadata after this process has
        // already persisted its attempt. Reverse lookup must attach to the
        // newest duplicate numeric ID, never the older process's record.
        next_process.phase(1, "lateFollowUp", "completed", None, None);
        next_process.finish(1, "cancelled");

        let inner = next_process.inner.lock_or_recover();
        let attempts = inner
            .attempts
            .iter()
            .filter(|attempt| attempt.transform_pass_id == 1)
            .collect::<Vec<_>>();
        assert_eq!(attempts.len(), 2);
        assert_eq!(attempts[0].outcome, "ready");
        assert!(attempts[0]
            .phases
            .iter()
            .any(|phase| phase.phase == "firstProcess"));
        assert!(!attempts[0]
            .phases
            .iter()
            .any(|phase| phase.phase == "lateFollowUp"));
        assert_eq!(attempts[1].outcome, "cancelled");
        assert!(attempts[1]
            .phases
            .iter()
            .any(|phase| phase.phase == "nextProcess"));
        assert!(attempts[1]
            .phases
            .iter()
            .any(|phase| phase.phase == "lateFollowUp"));
    }

    #[cfg(unix)]
    #[test]
    fn corrupt_capture_and_stale_temp_do_not_block_initialization_or_retention() {
        use std::os::unix::fs::{symlink, OpenOptionsExt, PermissionsExt};

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("diagnostics");
        let store = TransformDiagnostics::default();
        store.initialize(root.clone()).unwrap();

        store.arm_next().unwrap();
        store.begin(100);
        store.selection(
            100,
            Ok(("accessibility", "1-16", true, true, "success")),
            Some("current private input"),
        );
        store.finish(100, "ready");
        let current = store.list_captures().unwrap().pop().unwrap();

        let expired = DiagnosticCaptureV1 {
            schema_version: SCHEMA_VERSION,
            capture_id: "98-1".to_string(),
            transform_pass_id: 98,
            captured_at_ms: now_ms() - CAPTURE_RETENTION_MS - 1,
            expires_at_ms: now_ms() - 1,
            outcome: "ready".to_string(),
            selection: Some("expired private input".to_string()),
            instruction: Some("expired private instruction".to_string()),
            output: Some("expired private output".to_string()),
            phases: Vec::new(),
        };
        {
            let inner = store.inner.lock_or_recover();
            write_capture(&inner, &expired).unwrap();
        }

        let capture_root = root.join("transform-captures");
        let corrupt_path = capture_root.join("99-2.json");
        let mut corrupt = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&corrupt_path)
            .unwrap();
        corrupt.write_all(br#"{"schemaVersion":1"#).unwrap();
        corrupt.sync_all().unwrap();
        drop(corrupt);

        let stale_temp_path = capture_root.join(".capture-97-3-123-456.tmp");
        let mut stale_temp = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&stale_temp_path)
            .unwrap();
        stale_temp.write_all(b"stale private content").unwrap();
        drop(stale_temp);

        let unrelated_path = capture_root.join("notes.json");
        fs::write(&unrelated_path, b"unrelated").unwrap();
        let symlink_path = capture_root.join("96-4.json");
        let symlink_target = temp.path().join("outside-private-data");
        fs::write(&symlink_target, b"outside").unwrap();
        symlink(&symlink_target, &symlink_path).unwrap();

        let reloaded = TransformDiagnostics::default();
        reloaded.initialize(root.clone()).unwrap();
        let summaries = reloaded.list_captures().unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].capture_id, current.capture_id);
        let capture = reloaded.get_capture(&current.capture_id).unwrap().unwrap();
        assert_eq!(capture.selection.as_deref(), Some("current private input"));

        assert!(!corrupt_path.exists());
        assert!(!stale_temp_path.exists());
        assert!(!capture_root.join("98-1.json").exists());
        assert!(unrelated_path.exists());
        assert!(fs::symlink_metadata(&symlink_path)
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(reloaded.get_capture("96-4").is_err());
        assert_eq!(
            fs::metadata(&capture_root).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(capture_root.join(format!("{}.json", current.capture_id)))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    #[cfg(unix)]
    #[test]
    fn store_uses_restrictive_permissions_and_refuses_symlink_capture() {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("diagnostics");
        let store = TransformDiagnostics::default();
        store.initialize(root.clone()).unwrap();
        store.arm_next().unwrap();
        store.begin(7);
        store.finish(7, "failed");
        let summary = store.list_captures().unwrap().pop().unwrap();
        let capture_path = root
            .join("transform-captures")
            .join(format!("{}.json", summary.capture_id));
        assert_eq!(
            fs::metadata(&root).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&capture_path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::remove_file(&capture_path).unwrap();
        symlink(temp.path().join("elsewhere"), &capture_path).unwrap();
        assert!(store.get_capture(&summary.capture_id).is_err());
    }
}
