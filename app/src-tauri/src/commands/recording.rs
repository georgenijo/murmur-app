use crate::{MutexExt, State};
use crate::state::{AppState, DictationStatus};
use crate::transcriber;
use crate::{audio, injector, keyboard};
use crate::{log_info, log_warn, log_error};
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

/// Shared transcription pipeline: model init → transcribe → inject text → set idle
async fn run_transcription_pipeline(
    samples: &[f32],
    app_handle: &tauri::AppHandle,
    app_state: &AppState,
) -> Result<String, String> {
    // Guard resets status to Idle on any return path (error or success)
    let _guard = IdleGuard::new(app_state);

    // Read all needed state in one lock
    let (model_name, language, auto_paste) = {
        let dictation = app_state.dictation.lock_or_recover();
        (dictation.model_name.clone(), dictation.language.clone(), dictation.auto_paste)
    };

    // Phase: Transcription (includes lazy model load on first run)
    let t_transcribe = std::time::Instant::now();
    let text = {
        let mut backend = app_state.backend.lock_or_recover();
        backend.load_model(&model_name)?;
        backend.transcribe(samples, &language)?
    };
    log_info!("pipeline: transcription ({} samples): {:?}", samples.len(), t_transcribe.elapsed());

    // Phase: Text injection (clipboard write + optional osascript paste)
    let t_inject = std::time::Instant::now();
    if !text.is_empty() {
        let text_to_inject = text.clone();
        let (tx, rx) = tokio::sync::oneshot::channel::<Result<(), String>>();
        app_handle
            .run_on_main_thread(move || {
                let _ = tx.send(injector::inject_text(&text_to_inject, auto_paste));
            })
            .map_err(|e| format!("Failed to dispatch to main thread: {}", e))?;
        match tokio::time::timeout(std::time::Duration::from_secs(2), rx).await {
            Ok(Ok(Err(e))) => log_error!("Text injection failed: {}", e),
            Ok(Err(_)) => log_warn!("Text injection sender dropped"),
            Err(_) => log_warn!("Text injection timed out"),
            Ok(Ok(Ok(()))) => {}
        }
    }
    log_info!("pipeline: inject (clipboard + paste): {:?}", t_inject.elapsed());

    Ok(text)
    // _guard drops here, setting status to Idle
}

#[tauri::command]
pub async fn init_dictation(_state: tauri::State<'_, State>) -> Result<serde_json::Value, String> {
    log_info!("init_dictation");
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
    log_info!("pipeline: audio parse (base64 + WAV): {:?}", t_parse.elapsed());

    // Pipeline has its own guard, so disarm this one
    guard.disarm();

    let pipeline_result = run_transcription_pipeline(&samples, &app_handle, &state.app_state).await;
    keyboard::set_processing(false);
    let _ = app_handle.emit("recording-status-changed", "idle");
    let text = pipeline_result?;

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
            log_info!("Switched transcription backend to {}", backend.name());
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
                log_warn!("start_native_recording: already recording");
                return Ok(serde_json::json!({
                    "type": "already_recording",
                    "state": "recording"
                }));
            }
            DictationStatus::Processing => {
                log_warn!("start_native_recording: currently processing");
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

    log_info!("start_native_recording: device={}", device_name.as_deref().unwrap_or("system_default"));
    if let Err(e) = audio::start_recording(Some(app_handle.clone()), device_name) {
        log_error!("start_native_recording: audio failed: {}", e);
        let mut dictation = state.app_state.dictation.lock_or_recover();
        dictation.status = DictationStatus::Idle;
        return Err(e);
    }
    let _ = app_handle.emit("recording-status-changed", "recording");
    log_info!("start_native_recording: started");

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
                log_warn!("stop_native_recording: not recording");
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
    log_info!("stop_native_recording: stopping");
    let _ = app_handle.emit("recording-status-changed", "processing");

    // Guard resets status to Idle if stop_recording fails or samples are empty;
    // disarmed before handing off to run_transcription_pipeline (which has its own guard)
    let mut guard = IdleGuard::new(&state.app_state);

    // Phase: Audio teardown + 16kHz resample
    let t_total = std::time::Instant::now();
    let samples = audio::stop_recording().map_err(|e| {
        log_error!("stop_native_recording: stop_recording failed: {}", e);
        let _ = app_handle.emit("recording-status-changed", "idle");
        e
    })?;
    log_info!("pipeline: audio teardown + resample: {:?}", t_total.elapsed());

    if samples.is_empty() {
        log_info!("stop_native_recording: no audio captured");
        // guard drops on return, resetting status to Idle
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
    let text = pipeline_result.map_err(|e| {
        log_error!("stop_native_recording: pipeline failed: {}", e);
        e
    })?;

    let recording_secs = samples.len() / 16_000;
    let word_count = if text.trim().is_empty() { 0 } else { text.split_whitespace().count() };
    let approx_tokens = (word_count as f64 * 1.3).round() as usize;
    log_info!("pipeline: total end-to-end: {:?} (duration={}s words={} tokens={} chars={})",
        t_total.elapsed(), recording_secs, word_count, approx_tokens, text.len());

    // Broadcast transcription result to all windows (so the main window can update
    // its history even when recording was initiated from the overlay).
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
    log_info!("cancel_native_recording: speculative recording discarded");
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
            // guard drops here, but disarmed — no reset
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
