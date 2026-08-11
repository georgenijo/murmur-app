//! Guided, capture-only recording for a private personal benchmark corpus.
//!
//! The recorder uses the production capture worker and 16 kHz resampling path,
//! but deliberately does not enter dictation, inference, history, transform, or
//! clipboard delivery. Voice recordings and their reference text stay in a
//! fixed local Application Support directory and are never logged.

use crate::audio_lifecycle::{self, AudioCancelReason, AudioLifecycleEvent};
use crate::microphone_preview::{
    CLIPPING_SAMPLE_THRESHOLD, QUIET_PEAK_THRESHOLD, QUIET_RMS_THRESHOLD,
};
use crate::state::DictationStatus;
use crate::{MutexExt, State};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tauri::{Emitter, Manager};

const CORPUS_SCHEMA_VERSION: u32 = 1;
const CORPUS_ID: &str = "personal-v1";
const MAX_PROMPT_ID_BYTES: usize = 64;
const MAX_LABEL_BYTES: usize = 120;
const MAX_REFERENCE_BYTES: usize = 4_000;
const MAX_DEVICE_LABEL_BYTES: usize = 256;
const EXPECTED_PROMPT_COUNT: usize = 20;
const MAX_BENCHMARK_WAV_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone)]
struct ActiveCapture {
    capture_id: u64,
    request: CorpusStartRequest,
    phase: CapturePhase,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CapturePhase {
    Starting,
    Recording,
    Saving,
    Cancelling,
}

pub(crate) struct CorpusRecorderState {
    next_capture_id: AtomicU64,
    active: Mutex<Option<ActiveCapture>>,
}

impl Default for CorpusRecorderState {
    fn default() -> Self {
        Self {
            next_capture_id: AtomicU64::new(1),
            active: Mutex::new(None),
        }
    }
}

impl CorpusRecorderState {
    pub(crate) fn is_active(&self) -> bool {
        self.active.lock_or_recover().is_some()
    }

    fn claim(&self, request: CorpusStartRequest) -> Result<u64, String> {
        let mut active = self.active.lock_or_recover();
        if active.is_some() {
            return Err("A corpus recording is already active".to_string());
        }
        let capture_id = self.next_capture_id.fetch_add(1, Ordering::SeqCst);
        *active = Some(ActiveCapture {
            capture_id,
            request,
            phase: CapturePhase::Starting,
        });
        Ok(capture_id)
    }

    fn current(&self) -> Option<ActiveCapture> {
        self.active.lock_or_recover().clone()
    }

    fn clear_if(&self, capture_id: u64) {
        let mut active = self.active.lock_or_recover();
        if active
            .as_ref()
            .is_some_and(|capture| capture.capture_id == capture_id)
        {
            *active = None;
        }
    }

    fn set_phase_if(&self, capture_id: u64, phase: CapturePhase) {
        let mut active = self.active.lock_or_recover();
        if let Some(capture) = active
            .as_mut()
            .filter(|capture| capture.capture_id == capture_id)
        {
            capture.phase = phase;
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CorpusStartRequest {
    prompt_index: u32,
    prompt_id: String,
    label: String,
    reference: String,
    device_id: Option<String>,
    device_label: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CorpusRecordingEntry {
    entry_id: String,
    prompt_index: u32,
    prompt_id: String,
    label: String,
    reference: String,
    take: u32,
    selected: bool,
    file_name: String,
    sha256: String,
    recorded_at: String,
    sample_rate: u32,
    duration_ms: u64,
    peak: f32,
    rms: f32,
    clipping_percent: f32,
    device_label: String,
    quality_warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CorpusManifest {
    schema_version: u32,
    corpus_id: String,
    contains_real_user_data: bool,
    source: String,
    sample_rate: u32,
    created_at: String,
    updated_at: String,
    #[serde(default)]
    recordings: Vec<CorpusRecordingEntry>,
}

impl CorpusManifest {
    fn new(now: String) -> Self {
        Self {
            schema_version: CORPUS_SCHEMA_VERSION,
            corpus_id: CORPUS_ID.to_string(),
            contains_real_user_data: true,
            source: "guided-local-recording".to_string(),
            sample_rate: crate::state::WHISPER_SAMPLE_RATE,
            created_at: now.clone(),
            updated_at: now,
            recordings: Vec::new(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CorpusSummary {
    corpus_directory: String,
    recordings: Vec<CorpusRecordingEntry>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CorpusStartResponse {
    capture_id: u64,
    state: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CorpusStopResponse {
    corpus_directory: String,
    recording: CorpusRecordingEntry,
}

pub(crate) struct CorpusBenchmarkFixture {
    pub id: String,
    pub label: String,
    pub reference: String,
    pub wav: Vec<u8>,
}

fn corpus_root() -> Result<PathBuf, String> {
    if let Some(root) = std::env::var_os("MURMUR_BENCH_CORPUS_DIR") {
        let root = PathBuf::from(root);
        if root.is_absolute() {
            return Ok(root);
        }
        return Err("MURMUR_BENCH_CORPUS_DIR must be an absolute path".to_string());
    }
    dirs::data_dir()
        .map(|root| root.join("Murmur Benchmark Corpus").join("v1"))
        .ok_or_else(|| "Could not locate the local Application Support directory".to_string())
}

fn validate_request(request: &CorpusStartRequest) -> Result<(), String> {
    let prompt_id = request.prompt_id.as_bytes();
    if prompt_id.is_empty()
        || prompt_id.len() > MAX_PROMPT_ID_BYTES
        || !prompt_id
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
    {
        return Err(
            "Corpus prompt ID must contain only lowercase letters, numbers, and hyphens"
                .to_string(),
        );
    }
    if request.prompt_index == 0 || request.prompt_index > 999 {
        return Err("Corpus prompt index must be between 1 and 999".to_string());
    }
    if request.label.trim().is_empty() || request.label.len() > MAX_LABEL_BYTES {
        return Err("Corpus prompt label is invalid".to_string());
    }
    if request.reference.trim().is_empty() || request.reference.len() > MAX_REFERENCE_BYTES {
        return Err("Corpus reference text is invalid".to_string());
    }
    if request.device_label.len() > MAX_DEVICE_LABEL_BYTES {
        return Err("Microphone label is too long".to_string());
    }
    if request
        .device_id
        .as_ref()
        .is_some_and(|device| device.len() > 1_024)
    {
        return Err("Microphone identifier is too long".to_string());
    }
    Ok(())
}

fn read_manifest(root: &Path) -> Result<CorpusManifest, String> {
    let path = root.join("manifest.json");
    if !path.exists() {
        return Ok(CorpusManifest::new(Utc::now().to_rfc3339()));
    }
    let bytes = std::fs::read(&path)
        .map_err(|error| format!("Could not read the personal corpus manifest: {error}"))?;
    let manifest: CorpusManifest = serde_json::from_slice(&bytes)
        .map_err(|error| format!("The personal corpus manifest is invalid: {error}"))?;
    if manifest.schema_version != CORPUS_SCHEMA_VERSION || manifest.corpus_id != CORPUS_ID {
        return Err("The personal corpus manifest uses an unsupported format".to_string());
    }
    Ok(manifest)
}

pub(crate) fn load_benchmark_fixtures() -> Result<Vec<CorpusBenchmarkFixture>, String> {
    let root = corpus_root()?;
    let manifest = read_manifest(&root)?;
    if !manifest.contains_real_user_data || manifest.source != "guided-local-recording" {
        return Err("The personal corpus manifest has unexpected provenance".to_string());
    }

    let mut selected = manifest
        .recordings
        .into_iter()
        .filter(|recording| recording.selected)
        .collect::<Vec<_>>();
    selected.sort_by_key(|recording| recording.prompt_index);
    if selected.len() != EXPECTED_PROMPT_COUNT {
        return Err(format!(
            "Personal corpus is incomplete: expected {EXPECTED_PROMPT_COUNT} selected prompts, found {}",
            selected.len()
        ));
    }
    let unique_ids = selected
        .iter()
        .map(|recording| recording.prompt_id.as_str())
        .collect::<std::collections::HashSet<_>>();
    if unique_ids.len() != EXPECTED_PROMPT_COUNT {
        return Err("The personal corpus contains duplicate selected prompts".to_string());
    }

    let audio_dir = root.join("audio");
    selected
        .into_iter()
        .map(|recording| {
            let relative = Path::new(&recording.file_name);
            if relative.components().count() != 1
                || relative.file_name().and_then(|name| name.to_str())
                    != Some(recording.file_name.as_str())
            {
                return Err("The personal corpus contains an invalid audio filename".to_string());
            }
            let path = audio_dir.join(relative);
            let metadata = std::fs::metadata(&path)
                .map_err(|_| "A selected personal corpus WAV is missing".to_string())?;
            if !metadata.is_file() || metadata.len() > MAX_BENCHMARK_WAV_BYTES {
                return Err("A selected personal corpus WAV is invalid or too large".to_string());
            }
            let wav = std::fs::read(&path)
                .map_err(|_| "A selected personal corpus WAV could not be read".to_string())?;
            let actual_sha256 = format!("{:x}", Sha256::digest(&wav));
            if !actual_sha256.eq_ignore_ascii_case(&recording.sha256) {
                return Err(
                    "A selected personal corpus WAV failed integrity validation".to_string()
                );
            }
            Ok(CorpusBenchmarkFixture {
                id: recording.prompt_id,
                label: recording.label,
                reference: recording.reference,
                wav,
            })
        })
        .collect()
}

fn audio_quality(samples: &[f32]) -> (f32, f32, f32, Vec<String>) {
    let peak = samples
        .iter()
        .map(|sample| sample.abs())
        .fold(0.0_f32, f32::max);
    let rms = if samples.is_empty() {
        0.0
    } else {
        (samples
            .iter()
            .map(|sample| f64::from(*sample) * f64::from(*sample))
            .sum::<f64>()
            / samples.len() as f64)
            .sqrt() as f32
    };
    let clipping_percent = if samples.is_empty() {
        0.0
    } else {
        samples
            .iter()
            .filter(|sample| sample.abs() >= CLIPPING_SAMPLE_THRESHOLD)
            .count() as f32
            / samples.len() as f32
            * 100.0
    };
    let duration_ms = samples.len() as u64 / 16;
    let mut warnings = Vec::new();
    if duration_ms < 1_000 {
        warnings.push("Recording is shorter than one second".to_string());
    }
    if rms < QUIET_RMS_THRESHOLD || peak < QUIET_PEAK_THRESHOLD {
        warnings.push("Input is very quiet; move closer or raise microphone gain".to_string());
    }
    if clipping_percent > 0.1 {
        warnings.push("Input is clipping; lower microphone gain".to_string());
    }
    (peak, rms, clipping_percent, warnings)
}

fn persist_recording(
    capture_id: u64,
    request: CorpusStartRequest,
    samples: Vec<f32>,
) -> Result<CorpusStopResponse, String> {
    if samples.is_empty() {
        return Err("No microphone audio was captured".to_string());
    }
    let root = corpus_root()?;
    let audio_dir = root.join("audio");
    std::fs::create_dir_all(&audio_dir).map_err(|error| {
        format!("Could not create the personal corpus audio directory: {error}")
    })?;
    std::fs::create_dir_all(root.join("reports")).map_err(|error| {
        format!("Could not create the personal corpus report directory: {error}")
    })?;

    let mut manifest = read_manifest(&root)?;
    let take = manifest
        .recordings
        .iter()
        .filter(|recording| recording.prompt_id == request.prompt_id)
        .map(|recording| recording.take)
        .max()
        .unwrap_or(0)
        + 1;
    let file_name = format!(
        "{:03}-{}-take-{:02}.wav",
        request.prompt_index, request.prompt_id, take
    );
    let final_path = audio_dir.join(&file_name);
    if final_path.exists() {
        return Err("The next corpus recording filename already exists".to_string());
    }
    let temporary_path = audio_dir.join(format!(".{file_name}.{capture_id}.tmp"));
    crate::file_output::write_wav(&temporary_path, &samples)?;
    std::fs::rename(&temporary_path, &final_path).map_err(|error| {
        let _ = std::fs::remove_file(&temporary_path);
        format!("Could not publish the personal corpus recording: {error}")
    })?;

    let bytes = std::fs::read(&final_path)
        .map_err(|error| format!("Could not verify the personal corpus recording: {error}"))?;
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    let duration_ms = samples.len() as u64 / 16;
    let (peak, rms, clipping_percent, quality_warnings) = audio_quality(&samples);
    let recorded_at = Utc::now().to_rfc3339();
    let entry = CorpusRecordingEntry {
        entry_id: format!("{}-take-{take:02}", request.prompt_id),
        prompt_index: request.prompt_index,
        prompt_id: request.prompt_id.clone(),
        label: request.label,
        reference: request.reference,
        take,
        selected: true,
        file_name,
        sha256,
        recorded_at: recorded_at.clone(),
        sample_rate: crate::state::WHISPER_SAMPLE_RATE,
        duration_ms,
        peak,
        rms,
        clipping_percent,
        device_label: request.device_label,
        quality_warnings,
    };
    for existing in &mut manifest.recordings {
        if existing.prompt_id == request.prompt_id {
            existing.selected = false;
        }
    }
    manifest.updated_at = recorded_at;
    manifest.recordings.push(entry.clone());

    let manifest_bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| format!("Could not serialize the personal corpus manifest: {error}"))?;
    let manifest_path = root.join("manifest.json");
    let temporary_manifest = root.join(format!(".manifest.{capture_id}.tmp"));
    if let Err(error) = std::fs::write(&temporary_manifest, manifest_bytes)
        .and_then(|_| std::fs::rename(&temporary_manifest, &manifest_path))
    {
        let _ = std::fs::remove_file(&temporary_manifest);
        let _ = std::fs::remove_file(&final_path);
        return Err(format!(
            "Could not update the personal corpus manifest: {error}"
        ));
    }

    tracing::info!(
        target: "system",
        prompt_index = request.prompt_index,
        take,
        duration_ms,
        quality_warning_count = entry.quality_warnings.len(),
        "personal corpus recording saved"
    );
    Ok(CorpusStopResponse {
        corpus_directory: root.to_string_lossy().into_owned(),
        recording: entry,
    })
}

fn emit_status(app_handle: &tauri::AppHandle, state: &str, error: Option<&str>) {
    let _ = app_handle.emit(
        "corpus-recording-status",
        serde_json::json!({ "state": state, "error": error }),
    );
}

pub(crate) fn handle_audio_lifecycle(
    app_handle: tauri::AppHandle,
    capture_id: u64,
    event: AudioLifecycleEvent,
) {
    let state = app_handle.state::<State>();
    let Some(current) = state.corpus.current() else {
        return;
    };
    if current.capture_id != capture_id {
        return;
    }
    match event {
        AudioLifecycleEvent::Ready => {
            state
                .corpus
                .set_phase_if(capture_id, CapturePhase::Recording);
            emit_status(&app_handle, "recording", None);
        }
        AudioLifecycleEvent::StillConnecting => emit_status(&app_handle, "starting", None),
        AudioLifecycleEvent::Recovering { .. } => emit_status(&app_handle, "recovering", None),
        AudioLifecycleEvent::InitializationFailed { error, .. } => {
            emit_status(&app_handle, "error", Some(&error));
        }
        AudioLifecycleEvent::RecoveryStalled => emit_status(
            &app_handle,
            "error",
            Some("Microphone recovery is taking longer than expected"),
        ),
        AudioLifecycleEvent::Interrupted { .. } => {
            state.corpus.clear_if(capture_id);
            emit_status(&app_handle, "idle", None);
        }
        AudioLifecycleEvent::Idle => {
            // A normal Stop reaches audio-idle before the WAV and manifest are
            // durably published. Keep ownership until that save finishes so a
            // racing next take cannot update the same manifest concurrently.
            if current.phase != CapturePhase::Saving {
                state.corpus.clear_if(capture_id);
                emit_status(&app_handle, "idle", None);
            }
        }
    }
}

#[tauri::command]
pub async fn start_corpus_recording(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, State>,
    request: CorpusStartRequest,
) -> Result<CorpusStartResponse, String> {
    validate_request(&request)?;
    let device_id = request
        .device_id
        .as_deref()
        .filter(|device| !device.is_empty() && *device != "system_default")
        .map(str::to_string);
    let capture_id = {
        // Serialize the ownership claim with dictation and transform starts.
        // Core Audio startup itself happens after this short transition lock.
        let _transition = state.app_state.recording_transition.lock().await;
        let dictation = state.app_state.dictation.lock_or_recover();
        if dictation.status != DictationStatus::Idle {
            return Err("Finish the current dictation before recording corpus audio".to_string());
        }
        if state.app_state.file_transcribing.load(Ordering::SeqCst) {
            return Err("Finish the current file transcription first".to_string());
        }
        if state.benchmark.is_running() {
            return Err("Finish the current benchmark first".to_string());
        }
        if state.app_state.microphone_preview.is_active() {
            return Err("Stop the microphone test first".to_string());
        }
        if state.query.status().blocks_pipeline() {
            return Err("Finish the current voice query first".to_string());
        }
        if state.app_state.transform_status().blocks_recording()
            || state.transform_runtime.is_transform_busy()
        {
            return Err("Finish the current transform first".to_string());
        }
        state.corpus.claim(request)?
    };
    emit_status(&app_handle, "starting", None);
    let start_handle = app_handle.clone();
    let result = match tokio::task::spawn_blocking(move || {
        audio_lifecycle::start_corpus_recording(start_handle, device_id, capture_id)
    })
    .await
    {
        Ok(result) => result,
        Err(error) => Err(format!("Corpus recorder task failed: {error}")),
    };
    if let Err(error) = result {
        state.corpus.clear_if(capture_id);
        emit_status(&app_handle, "error", Some(&error));
        return Err(error);
    }
    Ok(CorpusStartResponse {
        capture_id,
        state: "recording",
    })
}

#[tauri::command]
pub async fn stop_corpus_recording(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, State>,
) -> Result<CorpusStopResponse, String> {
    let active = state
        .corpus
        .current()
        .ok_or_else(|| "No corpus recording is active".to_string())?;
    state
        .corpus
        .set_phase_if(active.capture_id, CapturePhase::Saving);
    emit_status(&app_handle, "saving", None);
    let capture_id = active.capture_id;
    let result = match tokio::task::spawn_blocking(move || {
        audio_lifecycle::stop_corpus_recording(capture_id)
    })
    .await
    {
        Ok(Ok(samples)) => {
            let request = active.request;
            tokio::task::spawn_blocking(move || persist_recording(capture_id, request, samples))
                .await
                .map_err(|error| format!("Corpus recorder save task failed: {error}"))?
        }
        Ok(Err(error)) => Err(error),
        Err(error) => Err(format!("Corpus recorder stop task failed: {error}")),
    };
    state.corpus.clear_if(capture_id);
    match result {
        Ok(response) => {
            emit_status(&app_handle, "idle", None);
            Ok(response)
        }
        Err(error) => {
            emit_status(&app_handle, "error", Some(&error));
            Err(error)
        }
    }
}

#[tauri::command]
pub async fn cancel_corpus_recording(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, State>,
) -> Result<bool, String> {
    let Some(active) = state.corpus.current() else {
        return Ok(false);
    };
    state
        .corpus
        .set_phase_if(active.capture_id, CapturePhase::Cancelling);
    emit_status(&app_handle, "recovering", None);
    let capture_id = active.capture_id;
    let result = tokio::task::spawn_blocking(move || {
        audio_lifecycle::cancel_corpus_capture(capture_id, AudioCancelReason::User)
    })
    .await
    .map_err(|error| format!("Corpus recorder cancel task failed: {error}"));
    match result {
        Ok(Ok(true)) => Ok(true),
        Ok(Ok(false)) => {
            state.corpus.clear_if(capture_id);
            emit_status(&app_handle, "idle", None);
            Ok(false)
        }
        Ok(Err(error)) | Err(error) => {
            state.corpus.clear_if(capture_id);
            emit_status(&app_handle, "error", Some(&error));
            Err(error)
        }
    }
}

#[tauri::command]
pub async fn get_corpus_summary() -> Result<CorpusSummary, String> {
    tokio::task::spawn_blocking(|| {
        let root = corpus_root()?;
        std::fs::create_dir_all(root.join("audio"))
            .map_err(|error| format!("Could not create the personal corpus directory: {error}"))?;
        std::fs::create_dir_all(root.join("reports")).map_err(|error| {
            format!("Could not create the personal corpus report directory: {error}")
        })?;
        let manifest = read_manifest(&root)?;
        Ok(CorpusSummary {
            corpus_directory: root.to_string_lossy().into_owned(),
            recordings: manifest.recordings,
        })
    })
    .await
    .map_err(|error| format!("Corpus summary task failed: {error}"))?
}

#[tauri::command]
pub fn open_corpus_folder(app_handle: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let root = corpus_root()?;
    std::fs::create_dir_all(&root)
        .map_err(|error| format!("Could not create the personal corpus directory: {error}"))?;
    app_handle
        .opener()
        .open_path(root.to_string_lossy().into_owned(), None::<&str>)
        .map_err(|error| format!("Could not open the personal corpus folder: {error}"))
}

pub(crate) fn plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::new("internal-benchmark")
        .invoke_handler(tauri::generate_handler![
            start_corpus_recording,
            stop_corpus_recording,
            cancel_corpus_recording,
            get_corpus_summary,
            open_corpus_folder,
        ])
        .build()
}
