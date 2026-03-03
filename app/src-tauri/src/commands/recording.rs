use crate::{MutexExt, State};
use crate::state::{AppState, DictationStatus};
use crate::transcriber;
use crate::{audio, injector, keyboard, vad};
use tauri::Emitter;

/// RAII guard that resets dictation status to Idle on drop,
/// ensuring status is restored on any early return or error path.
struct IdleGuard<'a> {
    app_state: &'a AppState,
    disarmed: bool,
}

impl<'a> IdleGuard<'a> {
    fn new(app_state: &'a AppState) -> Self {
        Self { app_state, disarmed: false }
    }

    fn disarm(&mut self) {
        self.disarmed = true;
    }
}

impl Drop for IdleGuard<'_> {
    fn drop(&mut self) {
        if !self.disarmed {
            let mut dictation = self.app_state.dictation.lock_or_recover();
            dictation.status = DictationStatus::Idle;
            keyboard::set_processing(false);
        }
    }
}

pub(crate) struct PipelineTimings {
    pub vad_ms: u64,
    pub inference_ms: u64,
    pub paste_ms: u64,
    pub rss_before_mb: u64,
    pub rss_after_mb: u64,
}

/// Shared transcription pipeline: model init -> transcribe -> inject text -> set idle
async fn run_transcription_pipeline(
    samples: &[f32],
    app_handle: &tauri::AppHandle,
    app_state: &AppState,
) -> Result<(String, PipelineTimings), String> {
    // Guard resets status to Idle on any return path (error or success)
    let _guard = IdleGuard::new(app_state);

    // Read all needed state in one lock
    let (model_name, language, auto_paste, paste_delay_ms, vad_sensitivity) = {
        let dictation = app_state.dictation.lock_or_recover();
        (dictation.model_name.clone(), dictation.language.clone(), dictation.auto_paste, dictation.auto_paste_delay_ms, dictation.vad_sensitivity)
    };

    // Pre-VAD signal level logging for mic diagnosis
    let rms = audio::compute_rms(samples);
    let peak = audio::compute_peak(samples);
    let device = audio::last_device_name().unwrap_or_else(|| "unknown".to_string());
    tracing::info!(target: "pipeline", "audio rms={:.4} peak={:.4} (device={})", rms, peak, device);

    // Phase: VAD -- filter out silence to prevent Whisper hallucination loops
    let vad_threshold = 1.0 - (vad_sensitivity as f32 / 100.0);
    let t_vad = std::time::Instant::now();
    let samples_for_transcription = match vad::vad_model_path() {
        Some(vad_path) if vad_path.exists() => {
            let vad_path_str = vad_path.to_string_lossy().to_string();
            let samples_owned = samples.to_vec();
            let vad_result = tokio::task::spawn_blocking(move || {
                vad::filter_speech(&vad_path_str, &samples_owned, vad_threshold)
            })
            .await
            .unwrap_or_else(|e| {
                Err(format!("VAD task panicked: {}", e))
            });

            match vad_result {
                Ok(vad::VadResult::NoSpeech) => {
                    tracing::info!(target: "pipeline", "VAD detected no speech ({} samples, {:?}), skipping transcription",
                        samples.len(), t_vad.elapsed());
                    return Ok((String::new(), PipelineTimings { vad_ms: t_vad.elapsed().as_millis() as u64, inference_ms: 0, paste_ms: 0, rss_before_mb: 0, rss_after_mb: 0 }));
                }
                Ok(vad::VadResult::Speech(trimmed)) => {
                    tracing::info!(target: "pipeline", "VAD trimmed {} -> {} samples ({:.0}% speech, {:?})",
                        samples.len(), trimmed.len(),
                        trimmed.len() as f64 / samples.len() as f64 * 100.0,
                        t_vad.elapsed());
                    trimmed
                }
                Err(e) => {
                    tracing::warn!(target: "pipeline", "VAD failed ({}), proceeding without filtering", e);
                    samples.to_vec()
                }
            }
        }
        _ => {
            // VAD model not available -- kick off background download for next time
            let handle = app_handle.clone();
            tokio::spawn(async move {
                if let Err(e) = super::models::ensure_vad_model(&handle).await {
                    tracing::warn!(target: "pipeline", "VAD model download failed ({}), skipping VAD", e);
                }
            });
            samples.to_vec()
        }
    };
    let vad_ms = t_vad.elapsed().as_millis() as u64;

    // Phase: Transcription (includes lazy model load on first run)
    let rss_before_mb = crate::resource_monitor::get_process_rss_mb();
    let t_transcribe = std::time::Instant::now();
    let text = {
        let mut backend = app_state.backend.lock_or_recover();
        backend.load_model(&model_name)?;
        backend.transcribe(&samples_for_transcription, &language)?
    };
    let inference_ms = t_transcribe.elapsed().as_millis() as u64;
    let rss_after_mb = crate::resource_monitor::get_process_rss_mb();
    tracing::info!(target: "pipeline", "transcription ({} samples): {:?}", samples_for_transcription.len(), t_transcribe.elapsed());

    // Update last_transcription_at for idle timeout tracking
    *app_state.last_transcription_at.lock_or_recover() = Some(std::time::Instant::now());

    // Phase: Text injection (clipboard write + optional osascript paste)
    let t_inject = std::time::Instant::now();
    if !text.is_empty() {
        let text_to_inject = text.clone();
        let (tx, rx) = tokio::sync::oneshot::channel::<Result<(), String>>();
        app_handle
            .run_on_main_thread(move || {
                let _ = tx.send(injector::inject_text(&text_to_inject, auto_paste, paste_delay_ms));
            })
            .map_err(|e| format!("Failed to dispatch to main thread: {}", e))?;
        let paste_hint = if cfg!(target_os = "macos") {
            "Text is in your clipboard -- press Cmd+V to paste manually."
        } else {
            "Text is in your clipboard -- press Ctrl+V to paste manually."
        };
        match tokio::time::timeout(std::time::Duration::from_secs(2), rx).await {
            Ok(Ok(Err(e))) => {
                tracing::error!(target: "pipeline", "Text injection failed: {}", e);
                let _ = app_handle.emit("auto-paste-failed", paste_hint);
            }
            Ok(Err(_)) => {
                tracing::warn!(target: "pipeline", "Text injection sender dropped");
                let _ = app_handle.emit("auto-paste-failed", paste_hint);
            }
            Err(_) => {
                tracing::warn!(target: "pipeline", "Text injection timed out");
                let _ = app_handle.emit("auto-paste-failed", paste_hint);
            }
            Ok(Ok(Ok(()))) => {}
        }
    }
    let paste_ms = t_inject.elapsed().as_millis() as u64;
    tracing::info!(target: "pipeline", "inject (clipboard + paste): {:?}", t_inject.elapsed());

    Ok((text, PipelineTimings { vad_ms, inference_ms, paste_ms, rss_before_mb, rss_after_mb }))
    // _guard drops here, setting status to Idle
}

#[tauri::command]
pub async fn init_dictation(_state: tauri::State<'_, State>) -> Result<serde_json::Value, String> {
    tracing::info!(target: "pipeline", "init_dictation");
    Ok(serde_json::json!({
        "type": "initialized",
        "state": "idle"
    }))
}

#[tauri::command]
pub async fn process_audio(
    app_handle: tauri::AppHandle,
    audio_data: String,
    state: tauri::State<'_, State>,
) -> Result<serde_json::Value, String> {
    {
        let mut dictation = state.app_state.dictation.lock_or_recover();
        dictation.status = DictationStatus::Processing;
    }
    keyboard::set_processing(true);
    let _ = app_handle.emit("recording-status-changed", "processing");

    // Guard resets status to Idle if decode/parse fails before reaching the pipeline
    let mut guard = IdleGuard::new(&state.app_state);

    // Phase: Audio parse (base64 decode + WAV to samples)
    let t_parse = std::time::Instant::now();
    let wav_bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &audio_data)
        .map_err(|e| {
            let _ = app_handle.emit("recording-status-changed", "idle");
            format!("Failed to decode base64: {}", e)
        })?;
    let samples = transcriber::parse_wav_to_samples(&wav_bytes).map_err(|e| {
        let _ = app_handle.emit("recording-status-changed", "idle");
        e
    })?;
    tracing::info!(target: "pipeline", "audio parse (base64 + WAV): {:?}", t_parse.elapsed());

    // Pipeline has its own guard, so disarm this one
    guard.disarm();

    let t_total = std::time::Instant::now();
    let pipeline_result = run_transcription_pipeline(&samples, &app_handle, &state.app_state).await;
    keyboard::set_processing(false);
    let _ = app_handle.emit("recording-status-changed", "idle");
    let (text, timings) = pipeline_result?;

    let total_ms = t_total.elapsed().as_millis() as u64;
    let audio_secs = samples.len() as f64 / 16_000.0;
    let word_count = if text.trim().is_empty() { 0 } else { text.split_whitespace().count() };
    let char_count = text.len();
    let model_name = {
        let d = state.app_state.dictation.lock_or_recover();
        d.model_name.clone()
    };
    let backend_name = {
        let b = state.app_state.backend.lock_or_recover();
        b.name().to_string()
    };
    tracing::info!(
        target: "pipeline",
        vad_ms = timings.vad_ms,
        inference_ms = timings.inference_ms,
        paste_ms = timings.paste_ms,
        total_ms = total_ms,
        audio_secs = audio_secs,
        word_count = word_count,
        char_count = char_count,
        rss_before_mb = timings.rss_before_mb,
        rss_after_mb = timings.rss_after_mb,
        model = model_name.as_str(),
        backend = backend_name.as_str(),
        "transcription complete"
    );

    Ok(serde_json::json!({
        "type": "transcription",
        "text": text
    }))
}

#[tauri::command]
pub async fn get_status(state: tauri::State<'_, State>) -> Result<serde_json::Value, String> {
    let dictation = state.app_state.dictation.lock_or_recover();
    Ok(serde_json::json!({
        "type": "status",
        "state": dictation.status,
        "model": dictation.model_name,
        "language": dictation.language
    }))
}

#[tauri::command]
pub async fn configure_dictation(
    options: serde_json::Value,
    state: tauri::State<'_, State>,
) -> Result<serde_json::Value, String> {
    tracing::info!(target: "pipeline", "configure_dictation: {}", options);

    let model = options.get("model").and_then(|v| v.as_str()).map(String::from);
    let language = options.get("language").and_then(|v| v.as_str()).map(String::from);

    let mut dictation = state.app_state.dictation.lock_or_recover();

    let model_changed = if let Some(m) = model {
        if m != dictation.model_name {
            dictation.model_name = m;
            true
        } else {
            false
        }
    } else {
        false
    };

    if let Some(l) = language {
        dictation.language = l;
    }

    if let Some(auto_paste) = options.get("autoPaste").and_then(|v| v.as_bool()) {
        dictation.auto_paste = auto_paste;
    }

    if let Some(delay) = options.get("autoPasteDelayMs").and_then(|v| v.as_u64()) {
        dictation.auto_paste_delay_ms = delay.clamp(10, 500);
    }

    if let Some(sensitivity) = options.get("vadSensitivity").and_then(|v| v.as_u64()) {
        dictation.vad_sensitivity = (sensitivity as u32).clamp(0, 100);
    }

    if let Some(idle_timeout) = options.get("idleTimeoutMinutes").and_then(|v| v.as_u64()) {
        let normalized = match idle_timeout {
            0 | 5 | 15 => idle_timeout as u32,
            _ => 5, // fall back to default
        };
        *state.app_state.idle_timeout_minutes.lock_or_recover() = normalized;
    }

    // If model changed, swap backend type if needed, or just reset for reload
    if model_changed {
        let new_model = dictation.model_name.clone();
        drop(dictation); // Release dictation lock first
        let mut backend = state.app_state.backend.lock_or_recover();
        let needs_swap = transcriber::is_moonshine_model(&new_model) != (backend.name() == "moonshine");
        if needs_swap {
            *backend = if transcriber::is_moonshine_model(&new_model) {
                Box::new(transcriber::MoonshineBackend::new())
            } else {
                Box::new(transcriber::WhisperBackend::new())
            };
            tracing::info!(target: "pipeline", "Switched transcription backend to {}", backend.name());
        } else {
            backend.reset();
        }
    }

    Ok(serde_json::json!({
        "type": "configured"
    }))
}

#[tauri::command]
pub async fn start_native_recording(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, State>,
    device_name: Option<String>,
) -> Result<serde_json::Value, String> {
    // Check and update status in one lock
    {
        let mut dictation = state.app_state.dictation.lock_or_recover();
        match dictation.status {
            DictationStatus::Recording => {
                tracing::warn!(target: "pipeline", "start_native_recording: already recording");
                return Ok(serde_json::json!({
                    "type": "already_recording",
                    "state": "recording"
                }));
            }
            DictationStatus::Processing => {
                tracing::warn!(target: "pipeline", "start_native_recording: currently processing");
                return Ok(serde_json::json!({
                    "type": "already_processing",
                    "state": "processing"
                }));
            }
            DictationStatus::Idle => {
                dictation.status = DictationStatus::Recording;
            }
        }
    }

    tracing::info!(target: "pipeline", "start_native_recording: device={}", device_name.as_deref().unwrap_or("system_default"));
    if let Err(e) = audio::start_recording(Some(app_handle.clone()), device_name) {
        tracing::error!(target: "audio", "start_native_recording: audio failed: {}", e);
        let mut dictation = state.app_state.dictation.lock_or_recover();
        dictation.status = DictationStatus::Idle;
        return Err(e);
    }
    let _ = app_handle.emit("recording-status-changed", "recording");
    tracing::info!(target: "pipeline", "start_native_recording: started");

    Ok(serde_json::json!({
        "type": "recording_started",
        "state": "recording"
    }))
}

#[tauri::command]
pub async fn stop_native_recording(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, State>,
) -> Result<serde_json::Value, String> {
    // Atomic check-and-set in a single lock to avoid TOCTOU gap
    {
        let mut dictation = state.app_state.dictation.lock_or_recover();
        match dictation.status {
            DictationStatus::Processing => return Ok(serde_json::json!({
                "type": "already_processing",
                "state": "processing"
            })),
            DictationStatus::Idle => {
                tracing::warn!(target: "pipeline", "stop_native_recording: not recording");
                return Ok(serde_json::json!({
                    "type": "not_recording",
                    "state": "idle"
                }));
            }
            DictationStatus::Recording => {
                dictation.status = DictationStatus::Processing;
            }
        }
    }
    keyboard::set_processing(true);
    tracing::info!(target: "pipeline", "stop_native_recording: stopping");
    let _ = app_handle.emit("recording-status-changed", "processing");

    // Guard resets status to Idle if stop_recording fails or samples are empty;
    // disarmed before handing off to run_transcription_pipeline (which has its own guard)
    let mut guard = IdleGuard::new(&state.app_state);

    // Phase: Audio teardown + 16kHz resample
    let t_total = std::time::Instant::now();
    let samples = audio::stop_recording().map_err(|e| {
        tracing::error!(target: "audio", "stop_native_recording: stop_recording failed: {}", e);
        let _ = app_handle.emit("recording-status-changed", "idle");
        e
    })?;
    tracing::info!(target: "pipeline", "audio teardown + resample: {:?}", t_total.elapsed());

    if samples.is_empty() {
        tracing::info!(target: "pipeline", "stop_native_recording: no audio captured");
        // guard drops on return, resetting status to Idle
        let _ = app_handle.emit("recording-status-changed", "idle");
        return Ok(serde_json::json!({
            "type": "transcription",
            "text": "",
            "state": "idle"
        }));
    }

    /// Minimum recording duration to process. Recordings shorter than this
    /// are discarded as phantom triggers (e.g. from residual key presses).
    const MIN_RECORDING_SAMPLES: usize = 4_800; // 0.3s at 16kHz

    if samples.len() < MIN_RECORDING_SAMPLES {
        tracing::info!(target: "pipeline", "stop_native_recording: recording too short ({}ms), discarding",
            samples.len() / 16); // samples / 16_000 * 1000
        let _ = app_handle.emit("recording-status-changed", "idle");
        return Ok(serde_json::json!({
            "type": "transcription",
            "text": "",
            "state": "idle"
        }));
    }

    // Hand off status management to the pipeline's own guard
    guard.disarm();

    let pipeline_result = run_transcription_pipeline(&samples, &app_handle, &state.app_state).await;
    keyboard::set_processing(false);
    let _ = app_handle.emit("recording-status-changed", "idle");
    let (text, timings) = pipeline_result.map_err(|e| {
        tracing::error!(target: "pipeline", "stop_native_recording: pipeline failed: {}", e);
        e
    })?;

    let total_ms = t_total.elapsed().as_millis() as u64;
    let audio_secs = samples.len() as f64 / 16_000.0;
    let word_count = if text.trim().is_empty() { 0 } else { text.split_whitespace().count() };
    let char_count = text.len();
    let model_name = {
        let d = state.app_state.dictation.lock_or_recover();
        d.model_name.clone()
    };
    let backend_name = {
        let b = state.app_state.backend.lock_or_recover();
        b.name().to_string()
    };

    tracing::info!(
        target: "pipeline",
        vad_ms = timings.vad_ms,
        inference_ms = timings.inference_ms,
        paste_ms = timings.paste_ms,
        total_ms = total_ms,
        audio_secs = audio_secs,
        word_count = word_count,
        char_count = char_count,
        rss_before_mb = timings.rss_before_mb,
        rss_after_mb = timings.rss_after_mb,
        model = model_name.as_str(),
        backend = backend_name.as_str(),
        "transcription complete"
    );

    // Broadcast transcription result to all windows (so the main window can update
    // its history even when recording was initiated from the overlay).
    let recording_secs = samples.len() / 16_000;
    if !text.is_empty() {
        let _ = app_handle.emit("transcription-complete", serde_json::json!({
            "text": text,
            "duration": recording_secs
        }));
    }

    Ok(serde_json::json!({
        "type": "transcription",
        "text": text,
        "state": "idle"
    }))
}

/// Cancel an in-progress recording without transcribing (used by Both mode
/// to silently discard the speculative recording from a short tap).
#[tauri::command]
pub async fn cancel_native_recording(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, State>,
) -> Result<(), String> {
    {
        let mut dictation = state.app_state.dictation.lock_or_recover();
        if dictation.status != DictationStatus::Recording {
            return Ok(());
        }
        dictation.status = DictationStatus::Idle;
    }
    // Stop audio capture and discard samples
    let _ = audio::stop_recording();
    let _ = app_handle.emit("recording-status-changed", "idle");
    tracing::info!(target: "pipeline", "cancel_native_recording: speculative recording discarded");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_guard_resets_status_on_drop() {
        let app_state = AppState::default();
        {
            let mut dictation = app_state.dictation.lock().unwrap();
            dictation.status = DictationStatus::Processing;
        }
        {
            let _guard = IdleGuard::new(&app_state);
            // guard drops here
        }
        let dictation = app_state.dictation.lock().unwrap();
        assert_eq!(dictation.status, DictationStatus::Idle);
    }

    #[test]
    fn idle_guard_disarm_prevents_reset() {
        let app_state = AppState::default();
        {
            let mut dictation = app_state.dictation.lock().unwrap();
            dictation.status = DictationStatus::Processing;
        }
        {
            let mut guard = IdleGuard::new(&app_state);
            guard.disarm();
            // guard drops here, but disarmed -- no reset
        }
        let dictation = app_state.dictation.lock().unwrap();
        assert_eq!(dictation.status, DictationStatus::Processing);
    }

    #[test]
    fn idle_guard_calls_set_processing_false() {
        keyboard::set_processing(true);
        let app_state = AppState::default();
        {
            let _guard = IdleGuard::new(&app_state);
        }
        assert!(!keyboard::is_processing());
    }
}
