use crate::{MutexExt, State};
use crate::state::{AppState, DictationStatus};
use crate::transcriber;
use crate::{audio, audio_decode, injector, keyboard, streaming, vad};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{Emitter, Manager};

/// Max number of code identifiers fed to Whisper as an initial prompt. Whisper
/// truncates the prompt to its ~224-token context window anyway, so this keeps
/// the bias list focused on the most frequent (most useful) terms. This budget is
/// intentionally *smaller* than [`CORRECTION_TERMS`]: the Whisper prompt is
/// token-bound, while Smart Correction is not (see [`CORRECTION_TERMS`]).
const WHISPER_PROMPT_TERMS: usize = 96;

/// Max number of code identifiers fed to the Smart Correction matcher
/// (`correction.rs` Tier 1/2). Unlike the Whisper initial prompt this path has no
/// token budget — it's a couple of linear passes over the transcript — so a far
/// larger term list is a pure recall win. The folder scan ranks ALL identifiers
/// once; the top [`CORRECTION_TERMS`] feed correction and the top
/// [`WHISPER_PROMPT_TERMS`] (a rank-prefix of them) feed Whisper.
const CORRECTION_TERMS: usize = 500;

/// Scan `folder` for code identifiers and build the cached folder-scan prompt
/// string, ranked by descending frequency and capped at [`CORRECTION_TERMS`].
/// The cache holds the larger (top-500) list; the Whisper path takes its
/// [`WHISPER_PROMPT_TERMS`] rank-prefix at use time (see [`whisper_prefix`]).
/// Empty/blank folder returns an empty string. Delegates to the guarded,
/// pure-logic `vocab` module (file count / size caps live there).
fn build_code_vocab_prompt(folder: &str) -> String {
    let folder = folder.trim();
    if folder.is_empty() {
        return String::new();
    }
    let prompt = crate::vocab::build_vocab_prompt_from_dir(
        std::path::Path::new(folder),
        CORRECTION_TERMS,
    );
    tracing::info!(
        target: "pipeline",
        "code vocab: scanned {} -> {} chars",
        folder,
        prompt.len()
    );
    prompt
}

/// Take the top [`WHISPER_PROMPT_TERMS`] space-separated terms of a ranked
/// folder-scan prompt. The cached prompt holds the top-[`CORRECTION_TERMS`] list
/// ranked by descending frequency, so its first 96 words ARE the top-96 terms.
/// This is what keeps the Whisper budget decoupled from the (larger) correction
/// budget without a second walk or a second cache field.
fn whisper_prefix(prompt: &str) -> String {
    prompt
        .split_whitespace()
        .take(WHISPER_PROMPT_TERMS)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Return the code-aware vocabulary prompt for the current settings, or an empty
/// string when the feature is disabled.
///
/// When enabled, the built-in dev-term dictionary always contributes so the
/// feature works with no folder selected ("just works"). A configured project
/// folder is scanned (and cached) and its identifiers are placed *first* — they're
/// more specific than the generic built-ins, so they survive Whisper's prompt
/// truncation. Folder scanning still happens at most once per folder/enable change.
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
    if !enabled {
        return String::new();
    }

    let builtin = crate::vocab::builtin_terms_prompt();

    // Optional project scan layered on top of the built-ins. The cache holds the
    // top-CORRECTION_TERMS (500) ranked list; Whisper's token-bound prompt takes
    // only its top-WHISPER_PROMPT_TERMS (96) rank-prefix here.
    let folder_prompt = if folder.trim().is_empty() {
        String::new()
    } else if let Some(prompt) = cached {
        whisper_prefix(&prompt)
    } else {
        // Not yet scanned: build now and cache the full (top-500) list (only if
        // settings still match), then feed Whisper its top-96 prefix.
        let prompt = build_code_vocab_prompt(&folder);
        let mut dictation = app_state.dictation.lock_or_recover();
        if dictation.code_vocab_enabled && dictation.code_vocab_folder == folder {
            dictation.code_vocab_prompt = Some(prompt.clone());
            // Refresh the correction matcher so Smart Correction picks up this
            // folder's top-500 terms now, not just on the next settings change.
            // configure_dictation no longer prebuilds the prompt, so on the common
            // restart-with-persisted-folder path the matcher would otherwise stay
            // builtin-only for the whole session (it's rebuilt only here, in
            // configure_dictation, and in scan_code_vocab — and the transcription
            // path hits none of the other two). rebuild_correction_matcher writes a
            // separate leaf mutex, so there's no deadlock with the held guard.
            rebuild_correction_matcher(app_state, &dictation);
        }
        whisper_prefix(&prompt)
    };

    // Project identifiers first (most specific), built-in dictionary after.
    if folder_prompt.trim().is_empty() {
        builtin
    } else {
        format!("{} {}", folder_prompt.trim(), builtin)
    }
}

/// Merge the manual custom-vocabulary string and the code-aware vocabulary into a
/// single Whisper initial prompt. Returns `None` when both are blank so the
/// backend skips setting a prompt entirely.
///
/// `code` carries the folder-scan terms ahead of the built-in dictionary (see
/// [`resolve_code_vocab_prompt`]). The hand-typed `custom` list is concatenated
/// FIRST so user-entered terms keep precedence: Whisper truncates the initial
/// prompt to ~224 tokens keeping the START, so a large code-scan prompt must not
/// crowd out terms the user explicitly typed. Order is therefore custom, then
/// folder, then builtin — and the whole thing is deduped case-insensitively so the
/// same term never burns two slots of the budget (the Smart Correction matcher
/// dedupes separately).
fn combine_prompts(custom: &str, code: &str) -> Option<String> {
    let custom = custom.trim();
    let code = code.trim();
    match (custom.is_empty(), code.is_empty()) {
        (true, true) => None,
        (false, true) => Some(dedupe_prompt_terms(custom)),
        (true, false) => Some(dedupe_prompt_terms(code)),
        (false, false) => Some(dedupe_prompt_terms(&format!("{} {}", custom, code))),
    }
}

/// Collapse a space-joined prompt to its first occurrence of each term,
/// case-insensitively, preserving order and the first surface form. Used at the
/// combine step so cross-source repeats (folder scan vs. built-in dictionary vs.
/// custom vocabulary) don't waste Whisper's limited prompt budget.
fn dedupe_prompt_terms(prompt: &str) -> String {
    let mut seen = std::collections::HashSet::new();
    prompt
        .split_whitespace()
        .filter(|t| seen.insert(t.to_ascii_lowercase()))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Split the free-text custom-vocabulary field into individual written terms.
/// Entries are separated by commas or newlines (not spaces) so multi-word terms
/// like "API Gateway" survive as one entry. Blank entries are dropped.
fn parse_vocab_terms(s: &str) -> Vec<String> {
    s.split(|c| c == ',' || c == '\n' || c == '\r')
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .map(String::from)
        .collect()
}

/// Rebuild the post-model correction matcher from the current dictation settings
/// and store it in `app_state.correction_matcher`. Called on settings-change (in
/// `configure_dictation`) — never per-utterance. `dictation` is the already-held
/// lock guard; the matcher is stored under a separate leaf mutex.
fn rebuild_correction_matcher(
    app_state: &AppState,
    dictation: &crate::state::DictationState,
) {
    let code_enabled = dictation.code_vocab_enabled;
    let mut terms = parse_vocab_terms(&dictation.custom_vocabulary);
    if code_enabled {
        // Built-in dev dictionary + any cached project scan become correction
        // terms (their spoken forms are auto-derived: "useEffect" -> "use effect").
        for t in crate::vocab::builtin_terms_prompt().split_whitespace() {
            terms.push(t.to_string());
        }
        // The cached folder prompt holds the top-CORRECTION_TERMS (500) ranked
        // identifiers. Unlike the Whisper prompt path (which takes only the top-96
        // rank-prefix because it's token-bound), Smart Correction has no token
        // budget, so it consumes the full cached list — a big recall win.
        if let Some(folder_prompt) = &dictation.code_vocab_prompt {
            for t in folder_prompt.split_whitespace() {
                terms.push(t.to_string());
            }
        }
    }
    let matcher = crate::correction::CorrectionMatcher::build(
        &terms,
        &[],
        dictation.correction_fuzzy,
        // Gate the std* abbrev builtins on the dev-context (code-vocab) signal.
        code_enabled,
    );
    *app_state.correction_matcher.lock_or_recover() = Some(std::sync::Arc::new(matcher));
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
    app_handle: Option<tauri::AppHandle>,
}

impl Drop for FileTranscribeGuard<'_> {
    fn drop(&mut self) {
        self.app_state.file_transcribing.store(false, Ordering::SeqCst);
        if let Some(app_handle) = &self.app_handle {
            let _ = app_handle.emit("file-transcription-status-changed", false);
        }
    }
}

struct SharedBackendChangeGuard(Arc<crate::benchmark::BenchmarkCoordinator>);

impl Drop for SharedBackendChangeGuard {
    fn drop(&mut self) {
        self.0.finish_shared_backend_change();
    }
}

/// Start loading the selected model after audio capture is live, overlapping the
/// expensive cold initialization with the user's speech. The normal pipeline
/// still calls `load_model`, so it either observes a hit or waits on this same
/// backend lock when a very short recording ends before preparation completes.
fn spawn_model_preparation(
    app_handle: tauri::AppHandle,
    model_name: String,
    recording_id: u64,
) {
    let queued_at = std::time::Instant::now();
    let _ = tauri::async_runtime::spawn_blocking(move || {
        let queue_ms = queued_at.elapsed().as_millis() as u64;
        let state = app_handle.state::<State>();
        let is_active = {
            let dictation = state.app_state.dictation.lock_or_recover();
            state.app_state.recording_id.load(Ordering::SeqCst) == recording_id
                && dictation.status == DictationStatus::Recording
                && dictation.model_name == model_name
        };
        if !is_active {
            return;
        }

        let lock_started = std::time::Instant::now();
        let mut backend = state.app_state.backend.lock_or_recover();
        let lock_wait_ms = lock_started.elapsed().as_millis() as u64;

        // The recording may have been cancelled while this worker waited for a
        // previous inference or model switch to release the backend.
        let is_still_active = {
            let dictation = state.app_state.dictation.lock_or_recover();
            state.app_state.recording_id.load(Ordering::SeqCst) == recording_id
                && dictation.status == DictationStatus::Recording
                && dictation.model_name == model_name
        };
        if !is_still_active {
            return;
        }

        let cache_hit = backend.is_model_loaded(&model_name);
        let rss_before_mb = crate::resource_monitor::get_process_rss_mb();
        let load_started = std::time::Instant::now();
        let result = backend.load_model(&model_name);
        let load_ms = load_started.elapsed().as_millis() as u64;
        let rss_after_mb = crate::resource_monitor::get_process_rss_mb();
        let total_ms = queued_at.elapsed().as_millis() as u64;
        match result {
            Ok(()) => tracing::info!(
                target: "pipeline",
                recording_id,
                model = model_name.as_str(),
                backend = backend.name(),
                cache_hit,
                queue_ms,
                lock_wait_ms,
                load_ms,
                total_ms,
                rss_before_mb,
                rss_after_mb,
                "model_prepare_complete"
            ),
            Err(error) => tracing::warn!(
                target: "pipeline",
                recording_id,
                model = model_name.as_str(),
                backend = backend.name(),
                cache_hit,
                lock_wait_ms,
                load_ms,
                total_ms,
                error = error.as_str(),
                "model_prepare_failed"
            ),
        }
    });
}

/// Warm an installed Core ML model after startup configuration. A newly linked
/// application binary can trigger one-time ANE specialization even when the
/// compiled model cache already exists. Starting it while the app is idle keeps
/// that cost away from the user's first short dictation; recording-start
/// preparation remains the fallback if recording begins first.
fn spawn_idle_model_preparation(
    app_handle: tauri::AppHandle,
    model_name: String,
    change_guard: SharedBackendChangeGuard,
) {
    let queued_at = std::time::Instant::now();
    let _ = tauri::async_runtime::spawn_blocking(move || {
        let _change_guard = change_guard;
        let state = app_handle.state::<State>();
        let is_current = {
            let dictation = state.app_state.dictation.lock_or_recover();
            dictation.status == DictationStatus::Idle && dictation.model_name == model_name
        };
        if !is_current {
            return;
        }

        let lock_started = std::time::Instant::now();
        let mut backend = state.app_state.backend.lock_or_recover();
        let lock_wait_ms = lock_started.elapsed().as_millis() as u64;
        let is_still_current = {
            let dictation = state.app_state.dictation.lock_or_recover();
            dictation.status == DictationStatus::Idle && dictation.model_name == model_name
        };
        if !is_still_current {
            return;
        }

        let cache_hit = backend.is_model_loaded(&model_name);
        let load_started = std::time::Instant::now();
        let result = backend.load_model(&model_name);
        let load_ms = load_started.elapsed().as_millis() as u64;
        let total_ms = queued_at.elapsed().as_millis() as u64;
        match result {
            Ok(()) => tracing::info!(
                target: "pipeline",
                model = model_name.as_str(),
                backend = backend.name(),
                cache_hit,
                lock_wait_ms,
                load_ms,
                total_ms,
                reason = "startup_configuration",
                "model_prepare_complete"
            ),
            Err(error) => tracing::info!(
                target: "pipeline",
                model = model_name.as_str(),
                backend = backend.name(),
                cache_hit,
                lock_wait_ms,
                load_ms,
                total_ms,
                reason = "startup_configuration",
                error = error.as_str(),
                "model_prepare_skipped"
            ),
        }
    });
}

#[derive(Default)]
pub(crate) struct PipelineTimings {
    pub vad_ms: u64,
    pub model_load_ms: u64,
    pub decode_ms: u64,
    pub inference_ms: u64,
    pub correction_ms: u64,
    pub paste_ms: u64,
    pub rss_before_mb: u64,
    pub rss_after_mb: u64,
    pub incremental_chunks: u32,
    pub streaming_inference_ms: u64,
    pub final_chunk_ms: u64,
}

#[allow(clippy::too_many_arguments)]
fn transcribe_with_coreml_vad_retry(
    backend: &mut dyn transcriber::TranscriptionBackend,
    model_name: &str,
    samples_for_transcription: &[f32],
    original_samples: &[f32],
    vad_trimmed: bool,
    language: &str,
    prompt: Option<&str>,
    smart_punctuation: bool,
) -> Result<String, String> {
    let text = backend.transcribe(samples_for_transcription, language, prompt, smart_punctuation)?;
    if transcriber::is_coreml_model(model_name) && vad_trimmed && text.trim().is_empty() {
        tracing::warn!(
            target: "pipeline",
            filtered_sample_count = samples_for_transcription.len(),
            original_sample_count = original_samples.len(),
            "coreml_empty_after_vad_retry_original"
        );
        return backend.transcribe(original_samples, language, prompt, smart_punctuation);
    }
    Ok(text)
}

/// Shared transcription pipeline: model init -> transcribe -> inject text -> set idle.
/// `recording_id` is checked against `app_state.cancelled_id` at checkpoints;
/// if cancelled, returns empty text without clipboard write or paste.
async fn run_transcription_pipeline(
    samples: &[f32],
    app_handle: &tauri::AppHandle,
    app_state: &AppState,
    recording_id: u64,
    incremental: Option<streaming::IncrementalTranscript>,
) -> Result<(String, PipelineTimings), String> {
    // Guard resets status to Idle on any return path (error or success),
    // but only if this recording is still the active one
    let _guard = IdleGuard::new(app_state, recording_id);

    // Read all needed state in one lock
    let (model_name, language, auto_paste, paste_delay_ms, vad_sensitivity, custom_vocabulary, smart_punctuation, save_transcript, save_audio, output_dir, app_profiles, voice_commands_enabled, voice_command_pairs, cleanup_enabled, cleanup_remove_filler, cleanup_capitalize, correction_enabled) = {
        let dictation = app_state.dictation.lock_or_recover();
        (dictation.model_name.clone(), dictation.language.clone(), dictation.auto_paste, dictation.auto_paste_delay_ms, dictation.vad_sensitivity, dictation.custom_vocabulary.clone(), dictation.smart_punctuation, dictation.save_transcript, dictation.save_audio, dictation.output_dir.clone(), dictation.app_profiles.clone(), dictation.voice_commands_enabled, dictation.voice_command_pairs.clone(), dictation.cleanup_enabled, dictation.cleanup_remove_filler, dictation.cleanup_capitalize, dictation.correction_enabled)
    };

    // Per-app profiles: if the frontmost app has a matching profile, its overrides
    // replace the global auto-paste / cleanup settings. The frontmost app is
    // detected once (a single osascript call) and reused for both resolutions. No
    // profiles (or no detected app) leaves the global values untouched.
    let (profile_auto_paste, profile_cleanup) = if app_profiles.is_empty() {
        (auto_paste, cleanup_enabled)
    } else {
        let bundle_id = crate::frontmost::frontmost_bundle_id();
        let resolved_paste = crate::frontmost::resolve_auto_paste(auto_paste, bundle_id.as_deref(), &app_profiles);
        let resolved_cleanup = crate::frontmost::resolve_cleanup(cleanup_enabled, bundle_id.as_deref(), &app_profiles);
        if resolved_paste != auto_paste || resolved_cleanup != cleanup_enabled {
            tracing::info!(target: "pipeline", "app profile override (frontmost={}): auto_paste {}->{}, cleanup {}->{}",
                bundle_id.as_deref().unwrap_or("unknown"), auto_paste, resolved_paste, cleanup_enabled, resolved_cleanup);
        }
        (resolved_paste, resolved_cleanup)
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
        return Ok((String::new(), PipelineTimings::default()));
    }

    let (text, mut timings) = if let Some(incremental) = incremental {
        tracing::info!(
            target: "pipeline",
            recording_id,
            chunks = incremental.chunk_count,
            streaming_inference_ms = incremental.streaming_inference_ms,
            final_chunk_ms = incremental.final_chunk_ms,
            "using authoritative incremental transcript"
        );
        (
            incremental.text,
            PipelineTimings {
                vad_ms: incremental.vad_ms,
                inference_ms: incremental.final_chunk_ms,
                decode_ms: incremental.final_chunk_ms,
                rss_before_mb: incremental.rss_before_mb,
                rss_after_mb: incremental.rss_after_mb,
                incremental_chunks: incremental.chunk_count,
                streaming_inference_ms: incremental.streaming_inference_ms,
                final_chunk_ms: incremental.final_chunk_ms,
                ..PipelineTimings::default()
            },
        )
    } else {
        // Batch fallback and all non-Whisper backends retain the pre-existing
        // full-buffer VAD + inference behavior.
        let vad_threshold = 1.0 - (vad_sensitivity as f32 / 100.0);
        let t_vad = std::time::Instant::now();
        let (samples_for_transcription, vad_trimmed) = match vad::vad_model_path() {
            Some(vad_path) if vad_path.exists() => {
                let vad_path_str = vad_path.to_string_lossy().to_string();
                let samples_owned = samples.to_vec();
                let vad_result = tokio::task::spawn_blocking(move || {
                    vad::filter_speech(&vad_path_str, &samples_owned, vad_threshold)
                })
                .await
                .unwrap_or_else(|e| Err(format!("VAD task panicked: {}", e)));

                match vad_result {
                    Ok(vad::VadResult::NoSpeech) => {
                        tracing::info!(target: "pipeline", "VAD detected no speech ({} samples, {:?}), skipping transcription",
                            samples.len(), t_vad.elapsed());
                        return Ok((String::new(), PipelineTimings {
                            vad_ms: t_vad.elapsed().as_millis() as u64,
                            ..PipelineTimings::default()
                        }));
                    }
                    Ok(vad::VadResult::Speech(trimmed)) => {
                        tracing::info!(target: "pipeline", "VAD trimmed {} -> {} samples ({:.0}% speech, {:?})",
                            samples.len(), trimmed.len(),
                            trimmed.len() as f64 / samples.len() as f64 * 100.0,
                            t_vad.elapsed());
                        let vad_trimmed = trimmed.len() != samples.len();
                        (trimmed, vad_trimmed)
                    }
                    Err(e) => {
                        tracing::warn!(target: "pipeline", "VAD failed ({}), proceeding without filtering", e);
                        (samples.to_vec(), false)
                    }
                }
            }
            _ => {
                let handle = app_handle.clone();
                tokio::spawn(async move {
                    if let Err(e) = super::models::ensure_vad_model(&handle).await {
                        tracing::warn!(target: "pipeline", "VAD model download failed ({}), skipping VAD", e);
                    }
                });
                (samples.to_vec(), false)
            }
        };
        let vad_ms = t_vad.elapsed().as_millis() as u64;

        if app_state.is_cancelled(recording_id) {
            tracing::info!(target: "pipeline", "cancelled before transcription (recording_id={})", recording_id);
            return Ok((String::new(), PipelineTimings {
                vad_ms,
                ..PipelineTimings::default()
            }));
        }

        let rss_before_mb = crate::resource_monitor::get_process_rss_mb();
        let t_transcribe = std::time::Instant::now();
        let sanitized = custom_vocabulary.replace('\0', "");
        let code_vocab = resolve_code_vocab_prompt(app_state);
        let prompt = combine_prompts(&sanitized, &code_vocab);
        let (text, model_load_ms, decode_ms) = {
            let load_started = std::time::Instant::now();
            let mut backend = app_state.backend.lock_or_recover();
            backend.load_model(&model_name)?;
            let model_load_ms = load_started.elapsed().as_millis() as u64;
            let decode_started = std::time::Instant::now();
            let text = transcribe_with_coreml_vad_retry(
                backend.as_mut(), &model_name, &samples_for_transcription, samples,
                vad_trimmed, &language, prompt.as_deref(), smart_punctuation,
            )?;
            let decode_ms = decode_started.elapsed().as_millis() as u64;
            (text, model_load_ms, decode_ms)
        };
        let inference_ms = t_transcribe.elapsed().as_millis() as u64;
        let rss_after_mb = crate::resource_monitor::get_process_rss_mb();
        tracing::info!(target: "pipeline", "transcription ({} samples): {:?}", samples_for_transcription.len(), t_transcribe.elapsed());
        (
            text,
            PipelineTimings {
                vad_ms,
                model_load_ms,
                decode_ms,
                inference_ms,
                rss_before_mb,
                rss_after_mb,
                ..PipelineTimings::default()
            },
        )
    };

    // Post-recognition transformation is backend-neutral and ordered in one
    // authoritative entry point. The effective cleanup toggle has already folded
    // in the existing per-app override; issue #245 will supply an opaque resolved
    // context handle without moving profile resolution into this module.
    let custom_commands: Vec<(String, String)> = voice_command_pairs
        .into_iter()
        .map(|vc| (vc.phrase, vc.replacement))
        .collect();
    let transform_context = crate::transcript_transform::TranscriptContext {
        session_id: app_state.next_transcript_session_id(),
        source: crate::transcript_transform::TranscriptSource::Live,
        context_handle: None,
        model: model_name.clone(),
        language: language.clone(),
        stages: crate::transcript_transform::TranscriptStageConfig {
            cleanup_enabled: profile_cleanup,
            cleanup_remove_filler,
            cleanup_capitalize,
            voice_commands_enabled,
            smart_correction_enabled: correction_enabled,
        },
    };
    let transform_resources = crate::transcript_transform::TranscriptTransformResources {
        custom_commands,
        correction_matcher: app_state.correction_matcher.lock_or_recover().clone(),
    };
    let transformed = crate::transcript_transform::transform_transcript(
        text,
        &transform_context,
        transform_resources,
    )
    .map_err(|error| error.to_string())?;
    let correction_ms = transformed
        .stage_duration_ms(crate::transcript_transform::SMART_CORRECTION_STAGE);
    let text = transformed.text;

    // Update last_transcription_at for idle timeout tracking
    *app_state.last_transcription_at.lock_or_recover() = Some(std::time::Instant::now());
    // Checkpoint 3: cancelled before text injection?
    if app_state.is_cancelled(recording_id) {
        tracing::info!(target: "pipeline", "cancelled before injection (recording_id={})", recording_id);
        timings.correction_ms = correction_ms;
        return Ok((String::new(), timings));
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

    timings.correction_ms = correction_ms;
    timings.paste_ms = paste_ms;
    Ok((text, timings))
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
        if state.benchmark.is_running() {
            tracing::warn!(target: "pipeline", "process_audio: blocked — benchmark in progress");
            return Err("Cannot process audio while a benchmark is in progress.".to_string());
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
    let pipeline_result = run_transcription_pipeline(
        &samples,
        &app_handle,
        &state.app_state,
        rid,
        None,
    )
    .await;
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
        recording_id = rid,
        vad_ms = timings.vad_ms,
        model_load_ms = timings.model_load_ms,
        decode_ms = timings.decode_ms,
        inference_ms = timings.inference_ms,
        incremental_chunks = timings.incremental_chunks,
        streaming_inference_ms = timings.streaming_inference_ms,
        final_chunk_ms = timings.final_chunk_ms,
        correction_ms = timings.correction_ms,
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
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, State>,
) -> Result<serde_json::Value, String> {
    tracing::info!(target: "pipeline", "configure_dictation: {}", options);

    let model = options.get("model").and_then(|v| v.as_str()).map(String::from);
    let language = options.get("language").and_then(|v| v.as_str()).map(String::from);

    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    if model
        .as_deref()
        .is_some_and(transcriber::is_coreml_model)
    {
        return Err(
            "Core ML transcription is available only on macOS 14 or newer with Apple Silicon"
                .to_string(),
        );
    }

    let mut dictation = state.app_state.dictation.lock_or_recover();

    let mut model_change_guard = if model
        .as_deref()
        .is_some_and(|requested| requested != dictation.model_name)
    {
        if !state.benchmark.try_start_shared_backend_change() {
            return Err(
                "Wait for the benchmark or current model preparation to finish before changing models."
                    .to_string(),
            );
        }
        Some(SharedBackendChangeGuard(state.benchmark.clone()))
    } else {
        None
    };

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

    // User-defined voice commands: array of { phrase, replacement }. Entries with
    // a blank phrase are skipped. Replaces the whole list when the key is present.
    if let Some(pairs) = options.get("voiceCommands").and_then(|v| v.as_array()) {
        dictation.voice_command_pairs = pairs
            .iter()
            .filter_map(|p| {
                let phrase = p.get("phrase").and_then(|v| v.as_str())?.trim().to_string();
                if phrase.is_empty() {
                    return None;
                }
                let replacement = p
                    .get("replacement")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                Some(crate::state::VoiceCommand { phrase, replacement })
            })
            .collect();
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
                let cleanup_override = p.get("cleanupOverride").and_then(|v| v.as_bool());
                Some(crate::state::AppProfile { bundle_id, label, auto_paste_override, cleanup_override })
            })
            .collect();
    }

    if let Some(cleanup_enabled) = options.get("cleanupEnabled").and_then(|v| v.as_bool()) {
        dictation.cleanup_enabled = cleanup_enabled;
    }

    if let Some(v) = options.get("cleanupRemoveFiller").and_then(|v| v.as_bool()) {
        dictation.cleanup_remove_filler = v;
    }

    if let Some(v) = options.get("cleanupCapitalize").and_then(|v| v.as_bool()) {
        dictation.cleanup_capitalize = v;
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
        // Invalidate the cached prompt; do NOT walk the tree synchronously here.
        // The explicit `scan_code_vocab` command owns the (single) walk for a
        // folder pick and caches the result, so prebuilding under the lock here
        // would just duplicate that walk — and choosing a folder fires both this
        // configure path and a scan concurrently. If no scan ever runs (folder set
        // programmatically), `resolve_code_vocab_prompt` lazily builds + caches the
        // prompt on the first transcription. Net: exactly one walk per folder.
        dictation.code_vocab_prompt = None;
        dictation.code_vocab_scan_id = None;
    }

    // Post-model correction toggles.
    if let Some(v) = options.get("correctionEnabled").and_then(|v| v.as_bool()) {
        dictation.correction_enabled = v;
    }
    if let Some(v) = options.get("correctionFuzzy").and_then(|v| v.as_bool()) {
        dictation.correction_fuzzy = v;
    }

    // Rebuild the correction matcher from the (now-updated) unified vocab +
    // correction settings. Built here on settings-change, never per-utterance.
    rebuild_correction_matcher(&state.app_state, &dictation);

    if let Some(idle_timeout) = options.get("idleTimeoutMinutes").and_then(|v| v.as_u64()) {
        let normalized = match idle_timeout {
            0 | 5 | 15 => idle_timeout as u32,
            _ => 5, // fall back to default
        };
        *state.app_state.idle_timeout_minutes.lock_or_recover() = normalized;
    }

    // If model changed, swap/reset the backend so the next transcription loads
    // the right engine for the selected model.
    let mut idle_preparation = None;
    if model_changed {
        let new_model = dictation.model_name.clone();
        if transcriber::is_coreml_model(&new_model) {
            idle_preparation = Some(new_model.clone());
        }
        drop(dictation); // Release dictation lock first
        let mut backend = state.app_state.backend.lock_or_recover();
        // Core ML must be classified first because its explicit model value also
        // starts with "parakeet", which is the sherpa backend's broad sentinel.
        let want_coreml = transcriber::is_coreml_model(&new_model);
        let want_parakeet = transcriber::parakeet::is_parakeet_model(&new_model);
        let desired_backend = if want_coreml {
            "coreml"
        } else if want_parakeet {
            "parakeet"
        } else {
            "whisper"
        };
        if backend.name() != desired_backend {
            *backend = if want_coreml {
                #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
                {
                    Box::new(transcriber::CoreMlBackend::new())
                }
                #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
                {
                    return Err(
                        "Core ML transcription is available only on macOS 14 or newer with Apple Silicon"
                            .to_string(),
                    );
                }
            } else if want_parakeet {
                Box::new(transcriber::ParakeetBackend::new())
            } else {
                Box::new(transcriber::WhisperBackend::new())
            };
            tracing::info!(target: "pipeline", "Switched transcription backend to {}", backend.name());
        } else {
            backend.reset();
        }
    }

    if let Some(model_name) = idle_preparation {
        // Treat warmup as activity so an already-expired idle timer cannot
        // immediately release the model this preparation is about to load.
        *state.app_state.last_transcription_at.lock_or_recover() =
            Some(std::time::Instant::now());
        spawn_idle_model_preparation(
            app_handle,
            model_name,
            model_change_guard
                .take()
                .expect("model changes hold the shared backend lease"),
        );
    }

    Ok(serde_json::json!({
        "type": "configured"
    }))
}

/// Live progress emitted (throttled) while [`scan_code_vocab`] walks a folder.
/// Field names are part of the frontend contract — do not rename.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VocabScanProgress {
    scan_id: String,
    current_path: String,
    files_read: usize,
    dirs_skipped: usize,
    terms_so_far: usize,
    done: bool,
    adopted: bool,
}

/// One ranked vocabulary term surfaced to the frontend pop-out: the written form
/// plus how many times it appeared across the scan. Serialized camelCase (already
/// single-word, so `term`/`freq` are unchanged). Rank is the array index + 1.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RankedTermJson {
    term: String,
    freq: u32,
}

/// Result of a [`scan_code_vocab`] run, returned to the frontend. Field names are
/// part of the frontend contract — do not rename.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VocabScanSummary {
    files: usize,
    skipped: usize,
    terms: usize,
    bytes: u64,
    capped: bool,
    ms: u64,
    /// Top ~12 written forms — a preview of what the scan biases toward.
    sample_terms: Vec<String>,
    /// Full ranked list of terms actually kept (<= [`CORRECTION_TERMS`]), ordered
    /// by descending frequency. Rank = array index + 1. Powers the "all scanned
    /// terms" pop-out and is the exact list fed to Smart Correction.
    ranked_terms: Vec<RankedTermJson>,
    /// How many of `ranked_terms` feed Whisper's initial prompt =
    /// min([`WHISPER_PROMPT_TERMS`], ranked_terms.len()). The first `whisper_count`
    /// entries are the Whisper budget; the rest are correction-only.
    whisper_count: usize,
    /// Whether this result was adopted into the live dictation settings. False
    /// means a newer scan or settings change superseded it while it was walking.
    adopted: bool,
}

/// Register an explicit scan as the latest settings intent before walking. This
/// makes the command own the folder transition even when the frontend's
/// configure call is still in flight, while the scan id lets later changes
/// supersede it deterministically.
fn begin_code_vocab_scan(app_state: &AppState, scan_id: &str, folder: &str) {
    let mut dictation = app_state.dictation.lock_or_recover();
    let folder_changed = dictation.code_vocab_folder != folder;
    let was_disabled = !dictation.code_vocab_enabled;
    dictation.code_vocab_enabled = true;
    dictation.code_vocab_folder = folder.to_string();
    dictation.code_vocab_scan_id = Some(scan_id.to_string());
    if folder_changed || was_disabled {
        dictation.code_vocab_prompt = None;
        rebuild_correction_matcher(app_state, &dictation);
    }
}

/// Adopt a completed scan only when it is still the latest run and the settings
/// still target the folder it walked. Returns false for overlapping scans and
/// for enable/folder changes made during the walk.
fn complete_code_vocab_scan(
    app_state: &AppState,
    scan_id: &str,
    folder: &str,
    prompt: String,
) -> bool {
    let mut dictation = app_state.dictation.lock_or_recover();
    let is_active = dictation.code_vocab_scan_id.as_deref() == Some(scan_id);
    let adopted = is_active
        && dictation.code_vocab_enabled
        && dictation.code_vocab_folder == folder;
    if adopted {
        dictation.code_vocab_prompt = Some(prompt);
        dictation.code_vocab_scan_id = None;
        rebuild_correction_matcher(app_state, &dictation);
    } else if is_active {
        dictation.code_vocab_scan_id = None;
    }
    adopted
}

/// Invalidate one explicit scan without disturbing a newer overlapping scan.
fn cancel_code_vocab_scan_id(app_state: &AppState, scan_id: &str) -> bool {
    let mut dictation = app_state.dictation.lock_or_recover();
    if dictation.code_vocab_scan_id.as_deref() != Some(scan_id) {
        return false;
    }
    dictation.code_vocab_scan_id = None;
    true
}

#[tauri::command]
pub fn cancel_code_vocab_scan(state: tauri::State<'_, State>, scan_id: String) -> bool {
    cancel_code_vocab_scan_id(&state.app_state, &scan_id)
}

/// Emit a throttled `vocab-scan-progress` tick carrying the live running counts
/// and the path being read/skipped. `force` bypasses the throttle (used for skip
/// rows so struck-through dependency/build dirs always render live). Updates
/// `*last_emit` when it fires. A free fn (not a closure) so both walk callbacks
/// can call it without one capturing the other and tripping the borrow checker.
#[allow(clippy::too_many_arguments)]
fn emit_scan_progress(
    handle: &tauri::AppHandle,
    scan_id: &str,
    last_emit: &mut std::time::Instant,
    current_path: String,
    files_read: usize,
    dirs_skipped: usize,
    terms_so_far: usize,
    force: bool,
) {
    if force
        || last_emit.elapsed().as_millis() >= VOCAB_PROGRESS_INTERVAL_MS
        || files_read.is_multiple_of(VOCAB_PROGRESS_EVERY_FILES)
    {
        *last_emit = std::time::Instant::now();
        let _ = handle.emit(
            "vocab-scan-progress",
            VocabScanProgress {
                scan_id: scan_id.to_string(),
                current_path,
                files_read,
                dirs_skipped,
                terms_so_far,
                done: false,
                adopted: false,
            },
        );
    }
}

/// Number of top written forms surfaced in [`VocabScanSummary::sample_terms`].
const VOCAB_SAMPLE_TERMS: usize = 12;
/// Emit a progress event at most this often (alongside the per-N-files gate) so a
/// fast walk doesn't flood the event channel.
const VOCAB_PROGRESS_INTERVAL_MS: u128 = 50;
/// Also emit progress every this many files, so a slow-but-steady walk still
/// reports even within one throttle window.
const VOCAB_PROGRESS_EVERY_FILES: usize = 10;

/// Scan `folder` for code identifiers off the UI thread, emitting throttled
/// `vocab-scan-progress` events as it walks, and return a [`VocabScanSummary`].
///
/// On a non-empty result this also adopts the scan: it stores the folder, enables
/// code-aware vocabulary, caches the built prompt, and rebuilds the correction
/// matcher — mirroring `configure_dictation` so the scan takes effect immediately
/// without a second round-trip. The filesystem walk and prompt build run inside
/// `spawn_blocking`; no std mutex is held across an `.await`.
#[tauri::command]
pub async fn scan_code_vocab(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, State>,
    folder: String,
    scan_id: String,
) -> Result<VocabScanSummary, String> {
    let folder_trimmed = folder.trim().to_string();
    if folder_trimmed.is_empty() {
        return Err("No folder selected to scan.".to_string());
    }
    if scan_id.trim().is_empty() {
        return Err("Missing scan id.".to_string());
    }

    begin_code_vocab_scan(&state.app_state, &scan_id, &folder_trimmed);

    tracing::info!(target: "pipeline", "scan_code_vocab: start");

    // Walk + prompt build are blocking (filesystem + CPU). Run them off the async
    // runtime; the closure emits throttled progress as files are read.
    let emit_handle = app_handle.clone();
    let emit_scan_id = scan_id.clone();
    let scan_folder = folder_trimmed.clone();
    let t_start = std::time::Instant::now();
    let (prompt, summary) = tokio::task::spawn_blocking(move || {
        let path = std::path::Path::new(&scan_folder);

        // Running scan counters shared across the file and skip callbacks. Interior
        // mutability (Cell/RefCell) lets BOTH `FnMut` closures capture the same
        // state by shared reference — two closures can't each capture the same
        // locals by `&mut`, which is why this isn't plain `let mut`.
        let files_read = std::cell::Cell::new(0usize);
        let dirs_skipped = std::cell::Cell::new(0usize);
        let last_emit = std::cell::Cell::new(std::time::Instant::now());
        // Running distinct-term count (case-insensitive) surfaced by the walk's
        // accumulator, so the live `terms_so_far` count tracks the eventual prompt
        // size instead of sitting at zero until the walk finishes. The walk already
        // folds each file in, so we read this count rather than re-extracting (which
        // would double the tokenization CPU across a 1000-file/32MB scan).
        let terms_so_far = std::cell::Cell::new(0usize);
        let walk_start = std::time::Instant::now();

        // Throttled emit reading the shared counters; `force` bypasses the throttle.
        let emit = |current_path: String, force: bool| {
            let mut le = last_emit.get();
            emit_scan_progress(
                &emit_handle,
                &emit_scan_id,
                &mut le,
                current_path,
                files_read.get(),
                dirs_skipped.get(),
                terms_so_far.get(),
                force,
            );
            last_emit.set(le);
        };

        let outcome = crate::vocab::collect_source_files(
            path,
            |file_path, distinct_terms| {
                files_read.set(files_read.get() + 1);
                // Track the live distinct-term tally the walker already computed
                // (no second extraction). Clamp it to CORRECTION_TERMS — the same
                // ceiling the final summary.terms is capped at — so the live counter
                // and the settled "done" total share one ceiling and the number
                // never visibly drops from e.g. "1,832 terms" back to "500 terms"
                // at completion. This is only the live counter; the authoritative
                // ranked term count still comes from the summary.
                terms_so_far.set(distinct_terms.min(CORRECTION_TERMS));
                // Throttle: emit on a time window OR every N files, never per-file.
                emit(file_path.to_string_lossy().to_string(), false);
            },
            |skip_path| {
                dirs_skipped.set(dirs_skipped.get() + 1);
                // Force-emit skip rows so the struck-through dependency/build dirs
                // (node_modules, target, .git) always render live, even between
                // file reads — that live skip stream is the point of the feature.
                emit(skip_path.to_string_lossy().to_string(), true);
            },
        );

        // Rank the folded accumulator ONCE at the larger correction budget. The
        // walk dropped each file's contents as it read it, so this ranking is the
        // only term-level data we keep — bounded by unique-term count, not bytes.
        let ranked = outcome.vocab.ranked(CORRECTION_TERMS);
        // The cached prompt string IS this ranked list joined; Whisper later takes
        // its top-WHISPER_PROMPT_TERMS prefix, correction consumes the whole thing.
        let prompt = crate::vocab::ranked_terms_to_prompt(&ranked);
        // Preview: top written forms (ranking is descending-frequency, so the
        // first VOCAB_SAMPLE_TERMS ARE the strip's preview chips).
        let sample_terms: Vec<String> = ranked
            .iter()
            .take(VOCAB_SAMPLE_TERMS)
            .map(|r| r.term.clone())
            .collect();
        // Full ranked list for the pop-out (<= CORRECTION_TERMS), plus how many of
        // them feed Whisper (min(96, kept)).
        let ranked_terms: Vec<RankedTermJson> = ranked
            .iter()
            .map(|r| RankedTermJson { term: r.term.clone(), freq: r.freq })
            .collect();
        let whisper_count = ranked_terms.len().min(WHISPER_PROMPT_TERMS);
        let terms = ranked_terms.len();

        let summary = VocabScanSummary {
            files: outcome.files_read,
            skipped: outcome.dirs_skipped,
            terms,
            bytes: outcome.total_bytes,
            capped: outcome.capped,
            ms: walk_start.elapsed().as_millis() as u64,
            sample_terms,
            ranked_terms,
            whisper_count,
            adopted: false,
        };
        (prompt, summary)
    })
    .await
    .map_err(|e| format!("Vocab scan task panicked: {}", e))?;

    let mut summary = summary;
    summary.adopted = complete_code_vocab_scan(
        &state.app_state,
        &scan_id,
        &folder_trimmed,
        prompt,
    );

    // Final progress tick so the UI lands on the accurate adopted/superseded
    // state before the command result resolves.
    let _ = app_handle.emit(
        "vocab-scan-progress",
        VocabScanProgress {
            scan_id: scan_id.clone(),
            current_path: String::new(),
            files_read: summary.files,
            dirs_skipped: summary.skipped,
            terms_so_far: summary.terms,
            done: true,
            adopted: summary.adopted,
        },
    );

    tracing::info!(
        target: "pipeline",
        files = summary.files,
        skipped = summary.skipped,
        terms = summary.terms,
        bytes = summary.bytes,
        capped = summary.capped,
        adopted = summary.adopted,
        ms = t_start.elapsed().as_millis() as u64,
        "scan_code_vocab: complete"
    );

    Ok(summary)
}

#[tauri::command]
pub async fn start_native_recording(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, State>,
    device_name: Option<String>,
) -> Result<serde_json::Value, String> {
    // Hold through cpal readiness and the recording event. A quick release can
    // invoke stop while start_recording is waiting for its capture thread; the
    // stop command must observe the fully-started recorder, never a midpoint.
    let _transition = state.app_state.recording_transition.lock().await;
    if keyboard::is_app_disabled() {
        tracing::info!(target: "pipeline", "start_native_recording: app disabled — ignoring");
        return Ok(serde_json::json!({ "type": "app_disabled", "state": "idle" }));
    }
    // Check and update status in one lock; assign recording ID in the same
    // critical section so no concurrent cancel/start can slip between them.
    let (rid, model_name, language, vad_sensitivity, custom_vocabulary, smart_punctuation, streaming_prompt_ready) = {
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
        if state.benchmark.is_running() {
            tracing::warn!(target: "pipeline", "start_native_recording: blocked — benchmark in progress");
            return Ok(serde_json::json!({
                "type": "busy_benchmarking",
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
                (
                    rid,
                    dictation.model_name.clone(),
                    dictation.language.clone(),
                    dictation.vad_sensitivity,
                    dictation.custom_vocabulary.clone(),
                    dictation.smart_punctuation,
                    !dictation.code_vocab_enabled
                        || dictation.code_vocab_folder.trim().is_empty()
                        || dictation.code_vocab_prompt.is_some(),
                )
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
    *state.app_state.last_transcription_at.lock_or_recover() = Some(std::time::Instant::now());
    let _ = app_handle.emit("recording-status-changed", "recording");
    tracing::info!(target: "pipeline", "start_native_recording: started");
    spawn_model_preparation(app_handle.clone(), model_name.clone(), rid);
    if streaming_prompt_ready {
        let sanitized = custom_vocabulary.replace('\0', "");
        let code_vocab = resolve_code_vocab_prompt(&state.app_state);
        let prompt = combine_prompts(&sanitized, &code_vocab);
        streaming::start_session(
            app_handle.clone(),
            streaming::StreamingConfig {
                recording_id: rid,
                model_name,
                language,
                prompt,
                smart_punctuation,
                vad_threshold: 1.0 - (vad_sensitivity as f32 / 100.0),
            },
        )
        .await;
    } else {
        tracing::info!(
            target: "pipeline",
            recording_id = rid,
            "incremental transcription skipped until code vocabulary cache is ready"
        );
    }

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
    let transition = state.app_state.recording_transition.lock().await;
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
    let _ = app_handle.emit("recording-status-changed", "processing");
    let streaming_session = streaming::begin_finish(&state.app_state, rid).await;

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
    // Audio is now detached from the recorder state. Let cancel or another
    // rejected start inspect Processing while inference continues.
    drop(transition);
    tracing::info!(target: "pipeline", "audio teardown + resample: {:?}", t_total.elapsed());

    if samples.is_empty() {
        streaming::discard(streaming_session);
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
        streaming::discard(streaming_session);
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

    let incremental = streaming::finalize(app_handle.clone(), streaming_session, &samples).await;
    let pipeline_result = run_transcription_pipeline(
        &samples,
        &app_handle,
        &state.app_state,
        rid,
        incremental,
    )
    .await;
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
        recording_id = rid,
        vad_ms = timings.vad_ms,
        model_load_ms = timings.model_load_ms,
        decode_ms = timings.decode_ms,
        inference_ms = timings.inference_ms,
        incremental_chunks = timings.incremental_chunks,
        streaming_inference_ms = timings.streaming_inference_ms,
        final_chunk_ms = timings.final_chunk_ms,
        correction_ms = timings.correction_ms,
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
    let _transition = state.app_state.recording_transition.lock().await;
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
    streaming::cancel(&state.app_state, rid).await;

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
    let _file_guard = FileTranscribeGuard {
        app_state: &state.app_state,
        app_handle: Some(app_handle.clone()),
    };
    let _ = app_handle.emit("file-transcription-status-changed", true);
    {
        let dictation = state.app_state.dictation.lock_or_recover();
        if state.benchmark.is_running() {
            return Err("Wait for the benchmark to finish before transcribing a file.".to_string());
        }
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
    let (samples_for_transcription, vad_trimmed) = match vad::vad_model_path() {
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
                Ok(vad::VadResult::Speech(trimmed)) => {
                    let vad_trimmed = trimmed.len() != samples.len();
                    (trimmed, vad_trimmed)
                }
                Err(e) => {
                    tracing::warn!(target: "pipeline", "transcribe_file: VAD failed ({}), proceeding without filtering", e);
                    (samples.clone(), false)
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
            (samples.clone(), false)
        }
    };

    // Phase: transcription (lazy model load), mirroring run_transcription_pipeline.
    let t_transcribe = std::time::Instant::now();
    let (text, model_load_ms, decode_ms) = {
        let sanitized = custom_vocabulary.replace('\0', "");
        let code_vocab = resolve_code_vocab_prompt(&state.app_state);
        let prompt = combine_prompts(&sanitized, &code_vocab);
        let load_started = std::time::Instant::now();
        let mut backend = state.app_state.backend.lock_or_recover();
        backend.load_model(&model_name)?;
        let model_load_ms = load_started.elapsed().as_millis() as u64;
        let decode_started = std::time::Instant::now();
        let text = transcribe_with_coreml_vad_retry(
            backend.as_mut(), &model_name, &samples_for_transcription, &samples,
            vad_trimmed, &language, prompt.as_deref(), smart_punctuation,
        )?;
        let decode_ms = decode_started.elapsed().as_millis() as u64;
        (text, model_load_ms, decode_ms)
    };
    // Imported files retain their existing raw-ASR output. They still pass through
    // the same authoritative transformation entry point with every stage disabled,
    // leaving delivery/UI behavior byte-for-byte unchanged.
    let transform_context = crate::transcript_transform::TranscriptContext {
        session_id: state.app_state.next_transcript_session_id(),
        source: crate::transcript_transform::TranscriptSource::File,
        context_handle: None,
        model: model_name.clone(),
        language: language.clone(),
        stages: crate::transcript_transform::TranscriptStageConfig::verbatim(),
    };
    let text = crate::transcript_transform::transform_transcript(
        text,
        &transform_context,
        crate::transcript_transform::TranscriptTransformResources::empty(),
    )
    .map_err(|error| error.to_string())?
    .text;

    *state.app_state.last_transcription_at.lock_or_recover() = Some(std::time::Instant::now());

    let word_count = if text.trim().is_empty() { 0 } else { text.split_whitespace().count() };
    tracing::info!(
        target: "pipeline",
        model_load_ms,
        decode_ms,
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
    use std::collections::VecDeque;
    use std::path::PathBuf;

    struct RetryTestBackend {
        responses: VecDeque<String>,
        sample_counts: Vec<usize>,
    }

    impl RetryTestBackend {
        fn new(responses: &[&str]) -> Self {
            Self {
                responses: responses.iter().map(|response| response.to_string()).collect(),
                sample_counts: Vec::new(),
            }
        }
    }

    impl transcriber::TranscriptionBackend for RetryTestBackend {
        fn name(&self) -> &str { "retry-test" }
        fn load_model(&mut self, _model_name: &str) -> Result<(), String> { Ok(()) }
        fn is_model_loaded(&self, _model_name: &str) -> bool { true }
        fn transcribe(&mut self, samples: &[f32], _language: &str, _initial_prompt: Option<&str>, _smart_punctuation: bool) -> Result<String, String> {
            self.sample_counts.push(samples.len());
            self.responses.pop_front().ok_or_else(|| "unexpected extra transcription attempt".to_string())
        }
        fn token_count(&self, _text: &str) -> Option<usize> { None }
        fn model_exists(&self) -> bool { true }
        fn models_dir(&self) -> Result<PathBuf, String> { Ok(std::env::temp_dir()) }
        fn reset(&mut self) {}
    }

    #[test]
    fn empty_coreml_result_after_vad_retries_original_audio_once() {
        let filtered = vec![0.0; 8_000];
        let original = vec![0.0; 16_000];
        let mut backend = RetryTestBackend::new(&["", "recovered words"]);
        let text = transcribe_with_coreml_vad_retry(
            &mut backend, transcriber::COREML_MODEL_NAME, &filtered, &original,
            true, "auto", None, true,
        ).unwrap();
        assert_eq!(text, "recovered words");
        assert_eq!(backend.sample_counts, vec![8_000, 16_000]);
    }

    #[test]
    fn still_empty_coreml_retry_is_bounded_to_two_attempts() {
        let filtered = vec![0.0; 8_000];
        let original = vec![0.0; 16_000];
        let mut backend = RetryTestBackend::new(&["", ""]);
        let text = transcribe_with_coreml_vad_retry(
            &mut backend, transcriber::COREML_MODEL_NAME, &filtered, &original,
            true, "auto", None, true,
        ).unwrap();
        assert!(text.is_empty());
        assert_eq!(backend.sample_counts, vec![8_000, 16_000]);
    }

    #[test]
    fn empty_result_without_coreml_vad_trim_is_not_retried() {
        let samples = vec![0.0; 8_000];
        let mut non_coreml = RetryTestBackend::new(&[""]);
        let text = transcribe_with_coreml_vad_retry(
            &mut non_coreml, "base.en", &samples, &samples,
            true, "en", None, true,
        ).unwrap();
        assert!(text.is_empty());
        assert_eq!(non_coreml.sample_counts, vec![8_000]);

        let mut untrimmed_coreml = RetryTestBackend::new(&[""]);
        let text = transcribe_with_coreml_vad_retry(
            &mut untrimmed_coreml, transcriber::COREML_MODEL_NAME, &samples, &samples,
            false, "auto", None, true,
        ).unwrap();
        assert!(text.is_empty());
        assert_eq!(untrimmed_coreml.sample_counts, vec![8_000]);
    }

    #[test]
    fn dedupe_prompt_drops_case_insensitive_repeats() {
        // "Tauri"/"tauri" collapse to the first surface form; order preserved.
        let out = dedupe_prompt_terms("Tauri useEffect tauri useState USEEFFECT");
        assert_eq!(out, "Tauri useEffect useState");
    }

    #[test]
    fn dedupe_prompt_is_noop_when_unique() {
        let out = dedupe_prompt_terms("alpha beta gamma");
        assert_eq!(out, "alpha beta gamma");
    }

    #[test]
    fn combine_prompts_dedupes_across_sources() {
        // Cross-source overlap: the folder/builtin `code` carries "Tauri" and the
        // custom list repeats it (different case). It must appear exactly once,
        // and order keeps the user-typed custom list first (precedence under
        // Whisper's start-keeping prompt truncation), then folder/builtin (code).
        let code = "Tauri serde useEffect";
        let custom = "tauri myProject SERDE";
        let combined = combine_prompts(custom, code).unwrap();
        assert_eq!(combined, "tauri myProject SERDE useEffect");
        // Each shared term appears exactly once (case-insensitively).
        let n_tauri = combined.split(' ').filter(|t| t.eq_ignore_ascii_case("tauri")).count();
        assert_eq!(n_tauri, 1, "combined={:?}", combined);
        let n_serde = combined.split(' ').filter(|t| t.eq_ignore_ascii_case("serde")).count();
        assert_eq!(n_serde, 1, "combined={:?}", combined);
    }

    #[test]
    fn combine_prompts_handles_single_and_empty_sources() {
        assert_eq!(combine_prompts("", ""), None);
        assert_eq!(combine_prompts("custom dupCustom", "").as_deref(), Some("custom dupCustom"));
        assert_eq!(combine_prompts("", "code dupe Dupe").as_deref(), Some("code dupe"));
    }

    #[test]
    fn budget_constants_decoupled_96_and_500() {
        // The two budgets are intentionally distinct; Whisper's is the smaller.
        assert_eq!(WHISPER_PROMPT_TERMS, 96);
        assert_eq!(CORRECTION_TERMS, 500);
        assert!(WHISPER_PROMPT_TERMS < CORRECTION_TERMS);
    }

    #[test]
    fn whisper_prefix_takes_top_96_of_ranked_prompt() {
        // A ranked prompt longer than the Whisper budget is truncated to its first
        // WHISPER_PROMPT_TERMS terms (which, given descending-frequency ranking,
        // are the top-96). The full string is what correction consumes.
        let full: String = (0..200)
            .map(|i| format!("term{:03}", i))
            .collect::<Vec<_>>()
            .join(" ");
        let prefix = whisper_prefix(&full);
        let words: Vec<&str> = prefix.split_whitespace().collect();
        assert_eq!(words.len(), WHISPER_PROMPT_TERMS, "Whisper budget caps at 96");
        // Prefix is the leading slice of the full ranked list.
        assert_eq!(words[0], "term000");
        assert_eq!(words[WHISPER_PROMPT_TERMS - 1], format!("term{:03}", WHISPER_PROMPT_TERMS - 1));
        // Shorter-than-budget prompt passes through unchanged.
        assert_eq!(whisper_prefix("alpha beta gamma"), "alpha beta gamma");
        assert_eq!(whisper_prefix(""), "");
    }

    #[test]
    fn summary_serializes_ranked_terms_and_whisper_count_camel_case() {
        // rankedTerms / whisperCount must serialize camelCase per the frontend
        // contract; each ranked entry is { term, freq }.
        let summary = VocabScanSummary {
            files: 3,
            skipped: 1,
            terms: 2,
            bytes: 42,
            capped: false,
            ms: 7,
            sample_terms: vec!["fooBar".into()],
            ranked_terms: vec![
                RankedTermJson { term: "fooBar".into(), freq: 5 },
                RankedTermJson { term: "barBaz".into(), freq: 2 },
            ],
            whisper_count: 2,
            adopted: true,
        };
        let v = serde_json::to_value(&summary).unwrap();
        // New camelCase keys present; snake_case absent.
        assert!(v.get("rankedTerms").is_some(), "rankedTerms missing: {v}");
        assert!(v.get("whisperCount").is_some(), "whisperCount missing: {v}");
        assert!(v.get("ranked_terms").is_none(), "snake_case leaked: {v}");
        assert!(v.get("whisper_count").is_none(), "snake_case leaked: {v}");
        assert_eq!(v["whisperCount"], 2);
        assert_eq!(v["adopted"], true);
        // sampleTerms (existing field) stays camelCase too.
        assert!(v.get("sampleTerms").is_some(), "sampleTerms missing: {v}");
        // Each ranked entry carries term + freq.
        let first = &v["rankedTerms"][0];
        assert_eq!(first["term"], "fooBar");
        assert_eq!(first["freq"], 5);
    }

    #[test]
    fn progress_serializes_scan_id_camel_case() {
        let progress = VocabScanProgress {
            scan_id: "scan-42".into(),
            current_path: "/project/src/main.rs".into(),
            files_read: 1,
            dirs_skipped: 0,
            terms_so_far: 3,
            done: false,
            adopted: false,
        };
        let value = serde_json::to_value(progress).unwrap();
        assert_eq!(value["scanId"], "scan-42");
        assert!(value.get("scan_id").is_none());
    }

    #[test]
    fn newer_scan_prevents_older_result_adoption() {
        let app_state = AppState::default();
        begin_code_vocab_scan(&app_state, "scan-a", "/project");
        begin_code_vocab_scan(&app_state, "scan-b", "/project");

        assert!(!complete_code_vocab_scan(
            &app_state,
            "scan-a",
            "/project",
            "staleTerm".into(),
        ));
        assert!(complete_code_vocab_scan(
            &app_state,
            "scan-b",
            "/project",
            "currentTerm".into(),
        ));

        let dictation = app_state.dictation.lock_or_recover();
        assert_eq!(dictation.code_vocab_prompt.as_deref(), Some("currentTerm"));
        assert!(dictation.code_vocab_scan_id.is_none());
    }

    #[test]
    fn cancellation_only_invalidates_the_matching_scan() {
        let app_state = AppState::default();
        begin_code_vocab_scan(&app_state, "scan-a", "/project");

        assert!(!cancel_code_vocab_scan_id(&app_state, "scan-b"));
        assert_eq!(
            app_state.dictation.lock_or_recover().code_vocab_scan_id.as_deref(),
            Some("scan-a"),
        );
        assert!(cancel_code_vocab_scan_id(&app_state, "scan-a"));
        assert!(!complete_code_vocab_scan(
            &app_state,
            "scan-a",
            "/project",
            "canceledTerm".into(),
        ));
        assert!(app_state.dictation.lock_or_recover().code_vocab_prompt.is_none());
    }

    #[test]
    fn settings_changes_during_scan_report_non_adoption() {
        for change in ["disable", "folder"] {
            let app_state = AppState::default();
            begin_code_vocab_scan(&app_state, "scan-a", "/project-a");
            {
                let mut dictation = app_state.dictation.lock_or_recover();
                if change == "disable" {
                    dictation.code_vocab_enabled = false;
                } else {
                    dictation.code_vocab_folder = "/project-b".into();
                }
                dictation.code_vocab_prompt = None;
                dictation.code_vocab_scan_id = None;
            }

            assert!(
                !complete_code_vocab_scan(
                    &app_state,
                    "scan-a",
                    "/project-a",
                    "staleTerm".into(),
                ),
                "{change} must supersede the in-flight scan",
            );
            let dictation = app_state.dictation.lock_or_recover();
            assert!(dictation.code_vocab_prompt.is_none());
        }
    }

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
            let _guard = FileTranscribeGuard {
                app_state: &app_state,
                app_handle: None,
            };
            // guard drops here
        }
        assert!(!app_state.file_transcribing.load(Ordering::SeqCst));
    }

    #[test]
    fn shared_backend_change_guard_releases_coordinator_on_drop() {
        let coordinator = Arc::new(crate::benchmark::BenchmarkCoordinator::new());
        assert!(coordinator.try_start_shared_backend_change());
        {
            let _guard = SharedBackendChangeGuard(coordinator.clone());
            assert!(!coordinator.try_start());
        }
        assert!(coordinator.try_start());
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
