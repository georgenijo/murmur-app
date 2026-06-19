use crate::{MutexExt, State};
use crate::state::{AppState, DictationStatus};
use crate::transcriber;
use crate::transcriber::preview;
use crate::{audio, audio_decode, injector, keyboard, vad};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Emitter;

/// How often the live-preview background task runs a partial transcription.
/// A ~1.4s cadence balances responsiveness against the CPU cost of repeated
/// whisper passes; the task additionally *skips* a tick if the previous pass is
/// still running, so a slow transcribe can never pile up.
const LIVE_PREVIEW_INTERVAL_MS: u64 = 1_400;

/// Spawn the optional live-preview background task for a recording session.
///
/// PREVIEW-ONLY: this never produces the authoritative injected text and never
/// touches `app_state.backend`. It owns a dedicated `PreviewTranscriber`
/// (separate whisper context/state) so it cannot race or corrupt the final
/// one-shot pass. No-op (returns immediately) when the feature is off or the
/// active backend isn't Whisper. The loop exits when `cancel` flips to false
/// (set on stop/cancel) or the model can't be loaded.
fn spawn_live_preview(
    app_handle: tauri::AppHandle,
    model_name: String,
    language: String,
    cancel: Arc<AtomicBool>,
    recording_id: u64,
) {
    use tauri::Manager;
    // True only while `cancel` is set AND this is still the active recording.
    // Guards against a stale task from a superseded session emitting after a
    // new recording has begun (rapid start-stop). The two tasks each own their
    // own whisper state, so this is purely about avoiding stale UI text. The
    // recording_id is read back through the managed `State` (the task is
    // 'static and can't borrow `AppState`).
    let still_current_handle = app_handle.clone();
    let still_current = move || {
        if !cancel.load(Ordering::Relaxed) {
            return false;
        }
        still_current_handle
            .try_state::<State>()
            .map(|s| s.app_state.recording_id.load(Ordering::SeqCst) == recording_id)
            .unwrap_or(false)
    };
    tokio::spawn(async move {
        // Resolve the model path off the chance the model isn't present yet.
        let model_path = match preview::preview_model_path(&model_name) {
            Some(p) => p.to_string_lossy().to_string(),
            None => {
                tracing::info!(target: "preview", "live preview: model '{}' not found, skipping", model_name);
                return;
            }
        };
        let lang_param = match language.as_str() {
            "auto" | "" => None,
            other => Some(other.to_string()),
        };

        // Build the dedicated preview engine on a blocking thread (model load +
        // GPU init). If it fails, silently skip preview for this session.
        let build = tokio::task::spawn_blocking(move || preview::PreviewTranscriber::new(&model_path)).await;
        let mut engine = match build {
            Ok(Ok(engine)) => engine,
            Ok(Err(e)) => {
                tracing::info!(target: "preview", "live preview: engine init failed ({}), skipping", e);
                return;
            }
            Err(e) => {
                tracing::info!(target: "preview", "live preview: engine init task panicked ({}), skipping", e);
                return;
            }
        };
        tracing::info!(target: "preview", "live preview: engine ready");

        let mut last_emitted: Option<String> = None;
        while still_current() {
            tokio::time::sleep(std::time::Duration::from_millis(LIVE_PREVIEW_INTERVAL_MS)).await;
            if !still_current() {
                break;
            }

            // Non-destructive snapshot of audio-so-far (16kHz mono).
            let snapshot = match audio::snapshot_samples() {
                Some(s) => s,
                None => continue, // recording ended between checks
            };
            let window = match preview::select_preview_window(
                &snapshot,
                crate::state::WHISPER_SAMPLE_RATE,
                preview::PREVIEW_WINDOW_SECS,
                preview::PREVIEW_MIN_SAMPLES,
            ) {
                Some(w) => w.to_vec(),
                None => continue, // not enough audio yet
            };

            // Run the (blocking) whisper pass on a dedicated thread so the async
            // runtime isn't stalled. The engine moves in and back out so it stays
            // owned by this loop (skip-if-running is implicit: we await here).
            let lang = lang_param.clone();
            let result = tokio::task::spawn_blocking(move || {
                let out = engine.transcribe_partial(&window, lang.as_deref());
                (engine, out)
            })
            .await;
            let out = match result {
                Ok((returned_engine, out)) => {
                    engine = returned_engine;
                    out
                }
                Err(e) => {
                    tracing::warn!(target: "preview", "live preview: pass panicked ({})", e);
                    continue;
                }
            };

            if !still_current() {
                break;
            }
            match out {
                Ok(Some(text)) => {
                    if last_emitted.as_deref() != Some(text.as_str()) {
                        last_emitted = Some(text.clone());
                        let _ = app_handle.emit("partial-transcript", text);
                    }
                }
                Ok(None) => { /* nothing worth showing this tick */ }
                Err(e) => {
                    tracing::warn!(target: "preview", "live preview: pass failed ({})", e);
                }
            }
        }
        tracing::info!(target: "preview", "live preview: loop exited");
    });
}

/// Max number of code identifiers to feed Whisper as an initial prompt. Whisper
/// truncates the prompt to its context window anyway; this keeps the bias list
/// focused on the most frequent (most useful) terms.
const MAX_CODE_VOCAB_TERMS: usize = 96;

/// Scan `folder` for code identifiers and build a Whisper initial-prompt string.
/// Empty/blank folder returns an empty string. Delegates to the guarded,
/// pure-logic `vocab` module (file count / size caps live there).
fn build_code_vocab_prompt(folder: &str) -> String {
    let folder = folder.trim();
    if folder.is_empty() {
        return String::new();
    }
    let prompt = crate::vocab::build_vocab_prompt_from_dir(
        std::path::Path::new(folder),
        MAX_CODE_VOCAB_TERMS,
    );
    tracing::info!(
        target: "pipeline",
        "code vocab: scanned {} -> {} chars",
        folder,
        prompt.len()
    );
    prompt
}

/// Return the code-aware vocabulary prompt for the current settings, or an empty
/// string when the feature is disabled. Uses the cached prompt when present;
/// otherwise scans the folder once and caches the result so subsequent
/// utterances don't rescan.
fn resolve_code_vocab_prompt(app_state: &AppState) -> String {
    // Fast path under the lock: feature off => nothing to do.
    let (enabled, folder, cached) = {
        let dictation = app_state.dictation.lock_or_recover();
        (
            dictation.code_vocab_enabled,
            dictation.code_vocab_folder.clone(),
            dictation.code_vocab_prompt.clone(),
        )
    };
    if !enabled || folder.trim().is_empty() {
        return String::new();
    }
    if let Some(prompt) = cached {
        return prompt;
    }
    // Not yet scanned: build now and cache (only if settings still match).
    let prompt = build_code_vocab_prompt(&folder);
    let mut dictation = app_state.dictation.lock_or_recover();
    if dictation.code_vocab_enabled && dictation.code_vocab_folder == folder {
        dictation.code_vocab_prompt = Some(prompt.clone());
    }
    prompt
}

/// Merge the manual custom-vocabulary string and the code-aware vocabulary into a
/// single Whisper initial prompt. Returns `None` when both are blank so the
/// backend skips setting a prompt entirely.
fn combine_prompts(custom: &str, code: &str) -> Option<String> {
    let custom = custom.trim();
    let code = code.trim();
    match (custom.is_empty(), code.is_empty()) {
        (true, true) => None,
        (false, true) => Some(custom.to_string()),
        (true, false) => Some(code.to_string()),
        (false, false) => Some(format!("{} {}", custom, code)),
    }
}

/// RAII guard that resets dictation status to Idle on drop,
/// ensuring status is restored on any early return or error path.
///
/// Tracks `recording_id` so it only resets state if this recording is still
/// the active one — prevents a stale pipeline from clobbering a new recording
/// that started after an Escape cancel.
struct IdleGuard<'a> {
    app_state: &'a AppState,
    recording_id: u64,
    disarmed: bool,
}

impl<'a> IdleGuard<'a> {
    fn new(app_state: &'a AppState, recording_id: u64) -> Self {
        Self { app_state, recording_id, disarmed: false }
    }

    fn disarm(&mut self) {
        self.disarmed = true;
    }
}

impl Drop for IdleGuard<'_> {
    fn drop(&mut self) {
        if !self.disarmed {
            // Lock first, then check recording_id — prevents TOCTOU where
            // a concurrent start advances the ID between our check and the
            // status write.
            let mut dictation = self.app_state.dictation.lock_or_recover();
            let current_rid = self.app_state.recording_id.load(Ordering::SeqCst);
            if current_rid != self.recording_id {
                return;
            }
            dictation.status = DictationStatus::Idle;
            keyboard::set_processing(false);
        }
    }
}

/// RAII guard that clears `file_transcribing` on drop, so a file transcription
/// releases its mutual-exclusion claim on every return path (early errors, `?`,
/// or success).
struct FileTranscribeGuard<'a> {
    app_state: &'a AppState,
}

impl Drop for FileTranscribeGuard<'_> {
    fn drop(&mut self) {
        self.app_state.file_transcribing.store(false, Ordering::SeqCst);
    }
}

pub(crate) struct PipelineTimings {
    pub vad_ms: u64,
    pub inference_ms: u64,
    pub paste_ms: u64,
    pub rss_before_mb: u64,
    pub rss_after_mb: u64,
}

/// Shared transcription pipeline: model init -> transcribe -> inject text -> set idle.
/// `recording_id` is checked against `app_state.cancelled_id` at checkpoints;
/// if cancelled, returns empty text without clipboard write or paste.
async fn run_transcription_pipeline(
    samples: &[f32],
    app_handle: &tauri::AppHandle,
    app_state: &AppState,
    recording_id: u64,
) -> Result<(String, PipelineTimings), String> {
    // Guard resets status to Idle on any return path (error or success),
    // but only if this recording is still the active one
    let _guard = IdleGuard::new(app_state, recording_id);

    // Read all needed state in one lock
    let (model_name, language, auto_paste, paste_delay_ms, vad_sensitivity, custom_vocabulary, smart_punctuation, save_transcript, save_audio, output_dir, app_profiles, voice_commands_enabled, cleanup_enabled) = {
        let dictation = app_state.dictation.lock_or_recover();
        (dictation.model_name.clone(), dictation.language.clone(), dictation.auto_paste, dictation.auto_paste_delay_ms, dictation.vad_sensitivity, dictation.custom_vocabulary.clone(), dictation.smart_punctuation, dictation.save_transcript, dictation.save_audio, dictation.output_dir.clone(), dictation.app_profiles.clone(), dictation.voice_commands_enabled, dictation.cleanup_enabled)
    };

    // Per-app profiles: if the frontmost app has a matching profile that sets an
    // auto-paste override, honor it instead of the global setting. No match (or
    // no detected app) leaves the global value untouched.
    let profile_auto_paste = if app_profiles.is_empty() {
        auto_paste
    } else {
        let bundle_id = crate::frontmost::frontmost_bundle_id();
        let resolved = crate::frontmost::resolve_auto_paste(auto_paste, bundle_id.as_deref(), &app_profiles);
        if resolved != auto_paste {
            tracing::info!(target: "pipeline", "app profile override: auto_paste {} -> {} (frontmost={})", auto_paste, resolved, bundle_id.as_deref().unwrap_or("unknown"));
        }
        resolved
    };

    // When saving to a file, suppress auto-paste into the focused app. The
    // clipboard write inside `inject_text` is unconditional, so text remains
    // copyable regardless of these toggles.
    let effective_auto_paste = profile_auto_paste && !(save_transcript || save_audio);

    // Pre-VAD signal level logging for mic diagnosis
    let rms = audio::compute_rms(samples);
    let peak = audio::compute_peak(samples);
    let device = audio::last_device_name().unwrap_or_else(|| "unknown".to_string());
    tracing::info!(target: "pipeline", "audio rms={:.4} peak={:.4} (device={})", rms, peak, device);

    // Checkpoint 1: cancelled before VAD?
    if app_state.is_cancelled(recording_id) {
        tracing::info!(target: "pipeline", "cancelled before VAD (recording_id={})", recording_id);
        return Ok((String::new(), PipelineTimings { vad_ms: 0, inference_ms: 0, paste_ms: 0, rss_before_mb: 0, rss_after_mb: 0 }));
    }

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

    // Checkpoint 2: cancelled before transcription?
    if app_state.is_cancelled(recording_id) {
        tracing::info!(target: "pipeline", "cancelled before transcription (recording_id={})", recording_id);
        return Ok((String::new(), PipelineTimings { vad_ms, inference_ms: 0, paste_ms: 0, rss_before_mb: 0, rss_after_mb: 0 }));
    }

    // Phase: Transcription (includes lazy model load on first run)
    let rss_before_mb = crate::resource_monitor::get_process_rss_mb();
    let t_transcribe = std::time::Instant::now();
    let text = {
        // Combine the manual custom vocabulary with the code-aware vocabulary
        // (when enabled). Both feed Whisper's initial prompt; the manual list
        // comes first so user-typed terms take precedence within the context window.
        let sanitized = custom_vocabulary.replace('\0', "");
        let code_vocab = resolve_code_vocab_prompt(app_state);
        let prompt = combine_prompts(&sanitized, &code_vocab);
        let mut backend = app_state.backend.lock_or_recover();
        backend.load_model(&model_name)?;
        backend.transcribe(&samples_for_transcription, &language, prompt.as_deref(), smart_punctuation)?
    };
    let inference_ms = t_transcribe.elapsed().as_millis() as u64;
    let rss_after_mb = crate::resource_monitor::get_process_rss_mb();
    tracing::info!(target: "pipeline", "transcription ({} samples): {:?}", samples_for_transcription.len(), t_transcribe.elapsed());

    // Phase: rule-based cleanup (filler removal + punctuation/spacing tidy).
    // Runs on the transcript before injection and file output so what the user
    // pastes matches what's saved. Conservative and independent of any other
    // post-processing (e.g. voice commands), operating only on its own setting.
    // Runs BEFORE voice commands so filler/punctuation tidy never mangles the
    // structural tokens (e.g. inserted "\n" from "new line") that voice commands
    // emit downstream.
    let text = if cleanup_enabled && !text.trim().is_empty() {
        let cleaned = crate::cleanup::clean_transcript(&text, crate::cleanup::CleanupOptions::default());
        tracing::info!(target: "pipeline", "cleanup applied ({} -> {} chars)", text.len(), cleaned.len());
        cleaned
    } else {
        text
    };

    // Phase: Voice commands -- rewrite spoken command tokens (e.g. "new line",
    // "scratch that") before the text reaches file output / clipboard / paste.
    let text = crate::voice_commands::apply_voice_commands(&text, voice_commands_enabled);

    // Update last_transcription_at for idle timeout tracking
    *app_state.last_transcription_at.lock_or_recover() = Some(std::time::Instant::now());
    // Checkpoint 3: cancelled before text injection?
    if app_state.is_cancelled(recording_id) {
        tracing::info!(target: "pipeline", "cancelled before injection (recording_id={})", recording_id);
        return Ok((String::new(), PipelineTimings { vad_ms, inference_ms, paste_ms: 0, rss_before_mb, rss_after_mb }));
    }

    // Phase: File output (optional) -- persist audio/transcript before injection.
    // Non-fatal: a write failure is logged and surfaced to the UI, but the text
    // is already on its way to the clipboard. Uses the original (pre-VAD) samples.
    if save_audio || save_transcript {
        if let Err(e) = crate::file_output::write_dictation_outputs(
            samples, &text, save_audio, save_transcript, &output_dir,
        ) {
            tracing::warn!(target: "pipeline", "file output failed: {}", e);
            let _ = app_handle.emit(
                "file-output-failed",
                "Couldn't save dictation to file. Text is still in your clipboard.",
            );
        }
    }

    // Phase: Text injection (clipboard write + optional osascript paste)
    let t_inject = std::time::Instant::now();
    if !text.is_empty() {
        let text_to_inject = text.clone();
        let (tx, rx) = tokio::sync::oneshot::channel::<Result<(), String>>();
        app_handle
            .run_on_main_thread(move || {
                let _ = tx.send(injector::inject_text(&text_to_inject, effective_auto_paste, paste_delay_ms));
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
    let rid = {
        let mut dictation = state.app_state.dictation.lock_or_recover();
        // Same mutual exclusion as start_native_recording: this legacy base64
        // path also runs the shared Whisper backend, so refuse while a file
        // transcription holds the slot. Checked under the dictation lock.
        if state.app_state.file_transcribing.load(Ordering::SeqCst) {
            tracing::warn!(target: "pipeline", "process_audio: blocked — file transcription in progress");
            return Err("Cannot process audio while a file transcription is in progress.".to_string());
        }
        dictation.status = DictationStatus::Processing;
        state.app_state.recording_id.load(Ordering::SeqCst)
    };
    keyboard::set_processing(true);
    let _ = app_handle.emit("recording-status-changed", "processing");

    // Guard resets status to Idle if decode/parse fails before reaching the pipeline
    let mut guard = IdleGuard::new(&state.app_state, rid);

    // Phase: Audio parse (base64 decode + WAV to samples)
    let t_parse = std::time::Instant::now();
    let wav_bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &audio_data)
        .map_err(|e| {
            if state.app_state.recording_id.load(Ordering::SeqCst) == rid {
                let _ = app_handle.emit("recording-status-changed", "idle");
            }
            format!("Failed to decode base64: {}", e)
        })?;
    let samples = transcriber::parse_wav_to_samples(&wav_bytes).map_err(|e| {
        if state.app_state.recording_id.load(Ordering::SeqCst) == rid {
            let _ = app_handle.emit("recording-status-changed", "idle");
        }
        e
    })?;
    tracing::info!(target: "pipeline", "audio parse (base64 + WAV): {:?}", t_parse.elapsed());

    // Pipeline has its own guard, so disarm this one
    guard.disarm();

    let t_total = std::time::Instant::now();
    let pipeline_result = run_transcription_pipeline(&samples, &app_handle, &state.app_state, rid).await;
    // Only emit idle if this recording wasn't cancelled/superseded.
    // Hold the dictation lock across the check+emit to prevent a concurrent
    // start from interleaving a "recording" status between our check and emit.
    {
        let _dictation = state.app_state.dictation.lock_or_recover();
        if state.app_state.recording_id.load(Ordering::SeqCst) == rid {
            keyboard::set_processing(false);
            let _ = app_handle.emit("recording-status-changed", "idle");
        }
    }
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

    if let Some(vocab) = options.get("customVocabulary").and_then(|v| v.as_str()) {
        dictation.custom_vocabulary = vocab.to_string();
    }

    if let Some(sp) = options.get("smartPunctuation").and_then(|v| v.as_bool()) {
        dictation.smart_punctuation = sp;
    }

    if let Some(vc) = options.get("voiceCommandsEnabled").and_then(|v| v.as_bool()) {
        dictation.voice_commands_enabled = vc;
    }

    if let Some(save_transcript) = options.get("saveTranscript").and_then(|v| v.as_bool()) {
        dictation.save_transcript = save_transcript;
    }

    if let Some(save_audio) = options.get("saveAudio").and_then(|v| v.as_bool()) {
        dictation.save_audio = save_audio;
    }

    if let Some(output_dir) = options.get("outputDir").and_then(|v| v.as_str()) {
        dictation.output_dir = output_dir.to_string();
    }

    // Per-app profiles: array of { bundleId, label, autoPasteOverride }. A
    // missing/null autoPasteOverride means "no override". Entries without a
    // bundleId are skipped. Replaces the whole list when the key is present.
    if let Some(profiles) = options.get("appProfiles").and_then(|v| v.as_array()) {
        dictation.app_profiles = profiles
            .iter()
            .filter_map(|p| {
                let bundle_id = p.get("bundleId").and_then(|v| v.as_str())?.trim().to_string();
                if bundle_id.is_empty() {
                    return None;
                }
                let label = p
                    .get("label")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                // null/absent -> None (use global); otherwise the boolean override.
                let auto_paste_override = p.get("autoPasteOverride").and_then(|v| v.as_bool());
                Some(crate::state::AppProfile { bundle_id, label, auto_paste_override })
            })
            .collect();
    }

    if let Some(cleanup_enabled) = options.get("cleanupEnabled").and_then(|v| v.as_bool()) {
        dictation.cleanup_enabled = cleanup_enabled;
    }

    // Live streaming preview (#129): preview-only overlay text while recording.
    // Default-off; only honored for the Whisper backend at recording time.
    if let Some(live_preview) = options.get("livePreviewEnabled").and_then(|v| v.as_bool()) {
        dictation.live_preview_enabled = live_preview;
    }

    // Code-aware vocabulary: a toggle plus a folder to scan for identifiers.
    // Changing either invalidates the cached prompt so the next transcription
    // (or the explicit prebuild below) rescans. Disabling clears the cache so we
    // don't hold a stale prompt in memory.
    let mut code_vocab_dirty = false;
    if let Some(enabled) = options.get("codeVocabEnabled").and_then(|v| v.as_bool()) {
        if enabled != dictation.code_vocab_enabled {
            dictation.code_vocab_enabled = enabled;
            code_vocab_dirty = true;
        }
    }
    if let Some(folder) = options.get("codeVocabFolder").and_then(|v| v.as_str()) {
        if folder != dictation.code_vocab_folder {
            dictation.code_vocab_folder = folder.to_string();
            code_vocab_dirty = true;
        }
    }
    if code_vocab_dirty {
        if dictation.code_vocab_enabled && !dictation.code_vocab_folder.trim().is_empty() {
            // Prebuild the prompt now (on the configure call) so the first
            // utterance isn't slowed by a directory walk. configure is rare and
            // the scan is guarded (file count / size caps), so doing it under the
            // lock here is acceptable.
            let prompt = build_code_vocab_prompt(&dictation.code_vocab_folder);
            dictation.code_vocab_prompt = Some(prompt);
        } else {
            // Disabled or no folder: clear any stale cached prompt.
            dictation.code_vocab_prompt = None;
        }
    }

    if let Some(idle_timeout) = options.get("idleTimeoutMinutes").and_then(|v| v.as_u64()) {
        let normalized = match idle_timeout {
            0 | 5 | 15 => idle_timeout as u32,
            _ => 5, // fall back to default
        };
        *state.app_state.idle_timeout_minutes.lock_or_recover() = normalized;
    }

    // If model changed, swap/reset the backend so the next transcription loads
    // the right engine for the selected model.
    if model_changed {
        let new_model = dictation.model_name.clone();
        drop(dictation); // Release dictation lock first
        let mut backend = state.app_state.backend.lock_or_recover();
        // --- Parakeet backend dispatch (removable): this is the only call site
        // that constructs ParakeetBackend. To remove the backend, revert this
        // block to a plain `backend.reset();`. ---
        let want_parakeet = transcriber::parakeet::is_parakeet_model(&new_model);
        if want_parakeet != (backend.name() == "parakeet") {
            *backend = if want_parakeet {
                Box::new(transcriber::ParakeetBackend::new())
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
    if keyboard::is_app_disabled() {
        tracing::info!(target: "pipeline", "start_native_recording: app disabled — ignoring");
        return Ok(serde_json::json!({ "type": "app_disabled", "state": "idle" }));
    }
    // Check and update status in one lock; assign recording ID in the same
    // critical section so no concurrent cancel/start can slip between them.
    let rid = {
        let mut dictation = state.app_state.dictation.lock_or_recover();
        // Refuse if a file transcription holds the shared Whisper backend.
        // Checked under the dictation lock (which `transcribe_file` takes only
        // after claiming the flag) so the two paths can't both start.
        if state.app_state.file_transcribing.load(Ordering::SeqCst) {
            tracing::warn!(target: "pipeline", "start_native_recording: blocked — file transcription in progress");
            return Ok(serde_json::json!({
                "type": "busy_transcribing_file",
                "state": "idle"
            }));
        }
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
                let rid = state.app_state.next_recording_id();
                dictation.status = DictationStatus::Recording;
                rid
            }
        }
    };
    tracing::info!(target: "pipeline", "start_native_recording: device={} recording_id={}", device_name.as_deref().unwrap_or("system_default"), rid);
    if let Err(e) = audio::start_recording(Some(app_handle.clone()), device_name) {
        tracing::error!(target: "audio", "start_native_recording: audio failed: {}", e);
        let mut dictation = state.app_state.dictation.lock_or_recover();
        dictation.status = DictationStatus::Idle;
        return Err(e);
    }
    let _ = app_handle.emit("recording-status-changed", "recording");
    tracing::info!(target: "pipeline", "start_native_recording: started");

    // Live streaming preview (#129): start the optional throttled preview task.
    // Default-off and Whisper-only; entirely separate from the final pipeline.
    maybe_start_live_preview(&app_handle, &state.app_state);

    Ok(serde_json::json!({
        "type": "recording_started",
        "state": "recording"
    }))
}

/// Start the live-preview task iff the setting is on AND the active backend is
/// Whisper. Reads the gate under the existing locks and arms the shared cancel
/// flag before spawning. No-op otherwise — so default-off users and Parakeet
/// users see byte-for-byte the current behavior.
fn maybe_start_live_preview(app_handle: &tauri::AppHandle, app_state: &AppState) {
    let (enabled, model_name, language) = {
        let d = app_state.dictation.lock_or_recover();
        (d.live_preview_enabled, d.model_name.clone(), d.language.clone())
    };
    let backend_name = {
        let b = app_state.backend.lock_or_recover();
        b.name().to_string()
    };
    if !preview::should_run_preview(enabled, &backend_name) {
        return;
    }
    // Arm the cancel flag for this session, then spawn.
    app_state.live_preview_active.store(true, Ordering::SeqCst);
    let cancel = Arc::clone(&app_state.live_preview_active);
    let recording_id = app_state.recording_id.load(Ordering::SeqCst);
    tracing::info!(target: "preview", "live preview: starting (model={}, recording_id={})", model_name, recording_id);
    spawn_live_preview(app_handle.clone(), model_name, language, cancel, recording_id);
}

/// Stop any running live-preview task and clear the overlay's preview text.
/// Idempotent and safe to call when no preview was running.
fn stop_live_preview(app_handle: &tauri::AppHandle, app_state: &AppState) {
    // Only act if a preview was actually armed, so we don't emit a spurious
    // clear event for default-off users.
    if app_state.live_preview_active.swap(false, Ordering::SeqCst) {
        tracing::info!(target: "preview", "live preview: stopping");
        // Tell the overlay to drop any partial text it's showing.
        let _ = app_handle.emit("partial-transcript", "");
    }
}

#[tauri::command]
pub async fn stop_native_recording(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, State>,
) -> Result<serde_json::Value, String> {
    // Atomic check-and-set + rid capture in a single lock to avoid TOCTOU gap
    let rid = {
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
                state.app_state.recording_id.load(Ordering::SeqCst)
            }
        }
    };
    keyboard::set_processing(true);
    tracing::info!(target: "pipeline", "stop_native_recording: stopping");
    // Stop the live-preview task first so it can't snapshot mid-teardown or race
    // the authoritative final pass. No-op when preview wasn't running.
    stop_live_preview(&app_handle, &state.app_state);
    let _ = app_handle.emit("recording-status-changed", "processing");

    // Guard resets status to Idle if stop_recording fails or samples are empty;
    // disarmed before handing off to run_transcription_pipeline (which has its own guard)
    let mut guard = IdleGuard::new(&state.app_state, rid);

    // Phase: Audio teardown + 16kHz resample
    let t_total = std::time::Instant::now();
    let samples = audio::stop_recording().map_err(|e| {
        tracing::error!(target: "audio", "stop_native_recording: stop_recording failed: {}", e);
        if state.app_state.recording_id.load(Ordering::SeqCst) == rid {
            let _ = app_handle.emit("recording-status-changed", "idle");
        }
        e
    })?;
    tracing::info!(target: "pipeline", "audio teardown + resample: {:?}", t_total.elapsed());

    if samples.is_empty() {
        tracing::info!(target: "pipeline", "stop_native_recording: no audio captured");
        // guard drops on return, resetting status to Idle
        if state.app_state.recording_id.load(Ordering::SeqCst) == rid {
            let _ = app_handle.emit("recording-status-changed", "idle");
        }
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
        if state.app_state.recording_id.load(Ordering::SeqCst) == rid {
            let _ = app_handle.emit("recording-status-changed", "idle");
        }
        return Ok(serde_json::json!({
            "type": "transcription",
            "text": "",
            "state": "idle"
        }));
    }

    // Hand off status management to the pipeline's own guard
    guard.disarm();

    let pipeline_result = run_transcription_pipeline(&samples, &app_handle, &state.app_state, rid).await;
    // Only emit idle if this recording wasn't cancelled/superseded by a new one.
    // Hold the dictation lock across the check+emit to prevent a concurrent
    // start from interleaving a "recording" status between our check and emit.
    {
        let _dictation = state.app_state.dictation.lock_or_recover();
        if state.app_state.recording_id.load(Ordering::SeqCst) == rid {
            keyboard::set_processing(false);
            let _ = app_handle.emit("recording-status-changed", "idle");
        }
    }
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

/// Cancel an in-progress recording or transcription.
///
/// - **Recording**: stops audio capture, discards samples, resets to Idle.
/// - **Processing**: marks the current recording_id as cancelled so the
///   pipeline discards its result at the next checkpoint; immediately
///   emits idle status so the UI resets without waiting for whisper.
/// - **Idle**: no-op.
#[tauri::command]
pub async fn cancel_native_recording(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, State>,
) -> Result<(), String> {
    let (prev_status, rid) = {
        let mut dictation = state.app_state.dictation.lock_or_recover();
        let prev = dictation.status;
        let rid = state.app_state.recording_id.load(Ordering::SeqCst);
        match prev {
            DictationStatus::Idle => return Ok(()),
            DictationStatus::Recording | DictationStatus::Processing => {
                dictation.status = DictationStatus::Idle;
            }
        }
        (prev, rid)
    };

    // Stop any live-preview task regardless of which phase we're cancelling from.
    stop_live_preview(&app_handle, &state.app_state);

    let stop_err = match prev_status {
        DictationStatus::Recording => {
            // Stop audio capture and discard samples
            if let Err(e) = audio::stop_recording() {
                tracing::error!(target: "audio", "cancel_native_recording: stop_recording failed: {}", e);
                Some(e)
            } else {
                tracing::info!(target: "pipeline", "cancel_native_recording: recording discarded");
                None
            }
        }
        DictationStatus::Processing => {
            // Mark current recording as cancelled — pipeline will check at next checkpoint
            state.app_state.cancel_recording(rid);
            tracing::info!(target: "pipeline", "cancel_native_recording: processing cancelled (recording_id={})", rid);
            None
        }
        DictationStatus::Idle => unreachable!(),
    };

    // Always emit feedback so the UI resets, even if stop_recording failed
    keyboard::set_processing(false);
    let _ = app_handle.emit("recording-status-changed", "idle");
    let _ = app_handle.emit("recording-cancelled", ());

    match stop_err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

#[tauri::command]
pub async fn count_vocab_tokens(
    text: String,
    state: tauri::State<'_, State>,
) -> Result<Option<usize>, String> {
    let backend = state.app_state.backend.lock_or_recover();
    Ok(backend.token_count(&text))
}

/// Transcribe an existing audio file (WAV/MP3/M4A) through the same Whisper
/// backend and settings as live dictation.
///
/// Unlike live recording, this path deliberately does **not** inject/paste the
/// result or drive the Idle/Recording/Processing status machine — a file
/// transcription is a one-shot "give me the text" action whose result is shown
/// in the UI. It decodes, downmixes to mono, resamples to 16kHz, applies VAD,
/// transcribes, and returns the text plus the decoded audio duration (seconds).
#[tauri::command]
pub async fn transcribe_file(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, State>,
    file_path: String,
) -> Result<serde_json::Value, String> {
    // Mutual exclusion with live dictation: both share one Whisper backend.
    // Claim the slot first (so a racing `start_native_recording` is blocked),
    // then refuse if a live recording/processing is already underway. The guard
    // releases the claim on every return path below.
    if state.app_state.file_transcribing.swap(true, Ordering::SeqCst) {
        return Err("Already transcribing a file.".to_string());
    }
    let _file_guard = FileTranscribeGuard { app_state: &state.app_state };
    {
        let dictation = state.app_state.dictation.lock_or_recover();
        if dictation.status != DictationStatus::Idle {
            return Err(
                "Can't transcribe a file while recording or processing live audio. \
                 Stop the current recording first."
                    .to_string(),
            );
        }
    }

    // Log only the extension as a structured field — never the raw path, which
    // would carry the user's home dir/username into telemetry (release builds
    // strip string fields from pipeline events, but not the message text).
    let ext = std::path::Path::new(&file_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    tracing::info!(target: "pipeline", ext = ext, "transcribe_file: start");

    // Phase: decode + downmix + resample to 16kHz mono (off the async runtime).
    let t_decode = std::time::Instant::now();
    let path_for_decode = file_path.clone();
    let samples = tokio::task::spawn_blocking(move || audio_decode::decode_to_mono_16k(&path_for_decode))
        .await
        .map_err(|e| format!("Decode task panicked: {}", e))??;
    if samples.is_empty() {
        return Err("No audio samples decoded from file".to_string());
    }
    // Precise seconds (f64): integer division floors sub-second clips to 0.
    let duration_secs = samples.len() as f64 / 16_000.0;
    tracing::info!(target: "pipeline", "transcribe_file: decoded {} samples (~{:.1}s) in {:?}",
        samples.len(), duration_secs, t_decode.elapsed());

    // Read the settings shared with live dictation in one lock.
    let (model_name, language, vad_sensitivity, custom_vocabulary, smart_punctuation) = {
        let dictation = state.app_state.dictation.lock_or_recover();
        (dictation.model_name.clone(), dictation.language.clone(), dictation.vad_sensitivity,
         dictation.custom_vocabulary.clone(), dictation.smart_punctuation)
    };

    // Phase: VAD — skip silence, best-effort with fallback to full audio (mirrors live).
    let vad_threshold = 1.0 - (vad_sensitivity as f32 / 100.0);
    let samples_for_transcription = match vad::vad_model_path() {
        Some(vad_path) if vad_path.exists() => {
            let vad_path_str = vad_path.to_string_lossy().to_string();
            let samples_owned = samples.clone();
            let vad_result = tokio::task::spawn_blocking(move || {
                vad::filter_speech(&vad_path_str, &samples_owned, vad_threshold)
            })
            .await
            .unwrap_or_else(|e| Err(format!("VAD task panicked: {}", e)));

            match vad_result {
                Ok(vad::VadResult::NoSpeech) => {
                    tracing::info!(target: "pipeline", "transcribe_file: VAD detected no speech");
                    return Ok(serde_json::json!({
                        "type": "file_transcription", "text": "", "duration": duration_secs
                    }));
                }
                Ok(vad::VadResult::Speech(trimmed)) => trimmed,
                Err(e) => {
                    tracing::warn!(target: "pipeline", "transcribe_file: VAD failed ({}), proceeding without filtering", e);
                    samples
                }
            }
        }
        _ => {
            // VAD model missing — kick off a background download for next time.
            let handle = app_handle.clone();
            tokio::spawn(async move {
                if let Err(e) = super::models::ensure_vad_model(&handle).await {
                    tracing::warn!(target: "pipeline", "transcribe_file: VAD model download failed ({})", e);
                }
            });
            samples
        }
    };

    // Phase: transcription (lazy model load), mirroring run_transcription_pipeline.
    let t_transcribe = std::time::Instant::now();
    let text = {
        let sanitized = custom_vocabulary.replace('\0', "");
        let code_vocab = resolve_code_vocab_prompt(&state.app_state);
        let prompt = combine_prompts(&sanitized, &code_vocab);
        let mut backend = state.app_state.backend.lock_or_recover();
        backend.load_model(&model_name)?;
        backend.transcribe(&samples_for_transcription, &language, prompt.as_deref(), smart_punctuation)?
    };
    *state.app_state.last_transcription_at.lock_or_recover() = Some(std::time::Instant::now());

    let word_count = if text.trim().is_empty() { 0 } else { text.split_whitespace().count() };
    tracing::info!(
        target: "pipeline",
        inference_ms = t_transcribe.elapsed().as_millis() as u64,
        audio_secs = duration_secs,
        word_count = word_count,
        model = model_name.as_str(),
        "file transcription complete"
    );

    Ok(serde_json::json!({
        "type": "file_transcription",
        "text": text,
        "duration": duration_secs
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_guard_resets_status_on_drop() {
        let app_state = AppState::default();
        let rid = app_state.next_recording_id();
        {
            let mut dictation = app_state.dictation.lock().unwrap();
            dictation.status = DictationStatus::Processing;
        }
        {
            let _guard = IdleGuard::new(&app_state, rid);
            // guard drops here
        }
        let dictation = app_state.dictation.lock().unwrap();
        assert_eq!(dictation.status, DictationStatus::Idle);
    }

    #[test]
    fn idle_guard_disarm_prevents_reset() {
        let app_state = AppState::default();
        let rid = app_state.next_recording_id();
        {
            let mut dictation = app_state.dictation.lock().unwrap();
            dictation.status = DictationStatus::Processing;
        }
        {
            let mut guard = IdleGuard::new(&app_state, rid);
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
        let rid = app_state.next_recording_id();
        {
            let _guard = IdleGuard::new(&app_state, rid);
        }
        assert!(!keyboard::is_processing());
    }

    #[test]
    fn file_transcribe_guard_clears_flag_on_drop() {
        use std::sync::atomic::Ordering;
        let app_state = AppState::default();
        app_state.file_transcribing.store(true, Ordering::SeqCst);
        {
            let _guard = FileTranscribeGuard { app_state: &app_state };
            // guard drops here
        }
        assert!(!app_state.file_transcribing.load(Ordering::SeqCst));
    }

    #[test]
    fn idle_guard_skips_reset_when_recording_superseded() {
        let app_state = AppState::default();
        let rid1 = app_state.next_recording_id(); // 1
        {
            let mut dictation = app_state.dictation.lock().unwrap();
            dictation.status = DictationStatus::Recording;
        }
        // Simulate new recording starting (increments rid)
        let _rid2 = app_state.next_recording_id(); // 2
        {
            let _guard = IdleGuard::new(&app_state, rid1);
            // guard drops here, but rid1 != current_rid(2), so no reset
        }
        let dictation = app_state.dictation.lock().unwrap();
        assert_eq!(dictation.status, DictationStatus::Recording);
    }

    #[test]
    fn generation_counter_cancel_current_recording() {
        let app_state = AppState::default();
        let id = app_state.next_recording_id(); // 1
        assert!(!app_state.is_cancelled(id));
        app_state.cancel_recording(id);
        assert!(app_state.is_cancelled(id));
    }

    #[test]
    fn generation_counter_new_recording_not_cancelled() {
        let app_state = AppState::default();
        let id1 = app_state.next_recording_id(); // 1
        app_state.cancel_recording(id1);
        let id2 = app_state.next_recording_id(); // 2
        assert!(app_state.is_cancelled(id1));
        assert!(!app_state.is_cancelled(id2));
    }

    #[test]
    fn generation_counter_monotonic_ids() {
        let app_state = AppState::default();
        let id1 = app_state.next_recording_id();
        let id2 = app_state.next_recording_id();
        let id3 = app_state.next_recording_id();
        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
        assert_eq!(id3, 3);
    }
}
