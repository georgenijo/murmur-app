use super::TranscriptionBackend;
use std::path::{Path, PathBuf};
use std::sync::Once;
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperState,
    install_logging_hooks,
};

static INIT_LOGGING: Once = Once::new();

/// Relative path under the platform data directory for app models.
const APP_MODELS_REL: &[&str] = &["local-dictation", "models"];

/// Above this many samples, whisper.cpp's `single_segment` mode is unsafe:
/// small models can emit an early end-of-text token roughly 24s into a 28s
/// window, and `single_segment` force-skips the *entire* 30s decode window
/// on early EOT (discarding the unread tail) instead of resuming decode from
/// the last emitted timestamp. 12s gives comfortable headroom under the
/// ~24s failure point observed in production while covering every
/// streaming window (10s) with margin to spare.
///
/// Streaming's incremental windows ([`super::chunking::WINDOW_SAMPLES`], 10s)
/// stay under this threshold and keep `single_segment = true`, which is
/// correct and desired there: each window is short enough that early EOT
/// isn't observed in practice, and single-segment output avoids spurious
/// segment splits within a short utterance. Longer batch/file transcriptions
/// cross the threshold and get multi-segment decoding so the tail isn't
/// silently dropped.
const SINGLE_SEGMENT_MAX_SECONDS: usize = 12;

/// Decide whether whisper should be constrained to a single output segment
/// for a given sample count. See [`SINGLE_SEGMENT_MAX_SECONDS`] for why.
fn should_use_single_segment(sample_count: usize) -> bool {
    sample_count <= SINGLE_SEGMENT_MAX_SECONDS * super::WHISPER_SAMPLE_RATE as usize
}

/// Suppress whisper.cpp verbose logging by installing a trampoline that routes to Rust's log crate
/// (which we don't configure, so logs go nowhere).
fn suppress_whisper_logs() {
    INIT_LOGGING.call_once(|| {
        install_logging_hooks();
    });
}

/// Build the app's primary models directory from a data dir root.
fn app_models_dir(data_dir: &Path) -> PathBuf {
    APP_MODELS_REL.iter().fold(data_dir.to_path_buf(), |p, s| p.join(s))
}

/// Get all potential model directories to search.
fn get_model_search_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Ok(custom_path) = std::env::var("WHISPER_MODEL_DIR") {
        paths.push(PathBuf::from(custom_path));
    }

    if let Some(data_dir) = dirs::data_dir() {
        paths.push(app_models_dir(&data_dir));
        paths.push(data_dir.join("pywhispercpp").join("models"));
    }

    if let Some(home) = dirs::home_dir() {
        paths.push(home.join(".cache").join("whisper.cpp"));
        paths.push(home.join(".cache").join("whisper"));
        paths.push(home.join(".whisper").join("models"));
    }

    paths
}

/// Get the path to a specific model file, searching multiple locations.
fn get_model_path(model_name: &str) -> Result<PathBuf, String> {
    let filename = format!("ggml-{}.bin", model_name);
    let search_paths = get_model_search_paths();

    for dir in &search_paths {
        let path = dir.join(&filename);
        if path.exists() {
            return Ok(path);
        }
    }

    let searched_locations = search_paths
        .iter()
        .map(|p| format!("  - {}", p.display()))
        .collect::<Vec<_>>()
        .join("\n");

    Err(format!(
        "Model '{}' not found. Searched locations:\n{}\n\nDownload from: https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}",
        filename, searched_locations, filename
    ))
}

pub fn specific_model_exists(model_name: &str) -> bool {
    get_model_path(model_name).is_ok()
}

pub struct WhisperBackend {
    context: Option<WhisperContext>,
    state: Option<WhisperState>,
    loaded_model_name: Option<String>,
}

impl WhisperBackend {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for WhisperBackend {
    fn default() -> Self {
        Self {
            context: None,
            state: None,
            loaded_model_name: None,
        }
    }
}

impl TranscriptionBackend for WhisperBackend {
    fn name(&self) -> &str {
        "whisper"
    }

    fn load_model(&mut self, model_name: &str) -> Result<(), String> {
        if let Some(ref loaded) = self.loaded_model_name {
            if loaded == model_name {
                let rss = crate::resource_monitor::get_process_rss_mb();
                tracing::info!(target: "pipeline", rss_mb = rss, "whisper_cache_hit");
                return Ok(());
            }
            self.reset();
        }

        suppress_whisper_logs();

        let model_path = get_model_path(model_name)?;
        let path_str = model_path
            .to_str()
            .ok_or_else(|| "Model path contains invalid UTF-8 characters".to_string())?;

        let mut params = WhisperContextParameters::default();
        // Murmur consumes segment text, not DTW token timestamps. Flash
        // attention therefore gives Metal/CUDA a fused, lower-memory path
        // without removing any output the application uses.
        params.flash_attn(true);

        let gpu_backend = if cfg!(target_os = "macos") {
            "metal"
        } else if cfg!(target_os = "linux") && std::path::Path::new("/dev/nvidia0").exists() {
            "cuda"
        } else {
            "cpu"
        };
        tracing::info!(target: "pipeline", model = model_name, gpu = gpu_backend, "whisper_model_loading");

        let ctx = WhisperContext::new_with_params(path_str, params)
            .map_err(|e| format!("Failed to load whisper model: {}", e))?;

        let state = ctx
            .create_state()
            .map_err(|e| format!("Failed to create whisper state: {}", e))?;
        self.context = Some(ctx);
        self.state = Some(state);
        self.loaded_model_name = Some(model_name.to_string());
        let rss = crate::resource_monitor::get_process_rss_mb();
        tracing::info!(target: "pipeline", rss_mb = rss, gpu = gpu_backend, "whisper_cache_miss");
        Ok(())
    }

    fn is_model_loaded(&self, model_name: &str) -> bool {
        self.loaded_model_name.as_deref() == Some(model_name)
    }
    fn transcribe(&mut self, samples: &[f32], language: &str, initial_prompt: Option<&str>, smart_punctuation: bool) -> Result<String, String> {
        let state = self
            .state
            .as_mut()
            .ok_or_else(|| "Whisper state not initialized. Call load_model() first.".to_string())?;
        tracing::info!(target: "pipeline", "whisper: reusing cached state for transcription");

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(whisper_language_param(language));
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_suppress_blank(true);
        params.set_single_segment(should_use_single_segment(samples.len()));
        if let Some(prompt) = initial_prompt {
            params.set_initial_prompt(prompt);
        }
        params.set_debug_mode(false);

        state
            .full(params, samples)
            .map_err(|e| format!("Transcription failed: {}", e))?;

        let num_segments = state.full_n_segments();

        let mut text = String::new();
        for i in 0..num_segments {
            let segment = state
                .get_segment(i)
                .ok_or_else(|| format!("Failed to get segment {}", i))?;
            let segment_text = segment
                .to_str()
                .map_err(|e| format!("Failed to get text for segment {}: {}", i, e))?;
            append_segment(&mut text, segment_text);
        }

        let trimmed = text.trim().to_string();
        if smart_punctuation {
            Ok(trimmed)
        } else {
            Ok(strip_punctuation(&trimmed))
        }
    }

    fn token_count(&self, text: &str) -> Option<usize> {
        let ctx = self.context.as_ref()?;
        ctx.tokenize(text, 1024).ok().map(|tokens| tokens.len())
    }

    fn model_exists(&self) -> bool {
        let search_paths = get_model_search_paths();
        for dir in &search_paths {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    if entry.path().extension().and_then(|e| e.to_str()) == Some("bin") {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn models_dir(&self) -> Result<PathBuf, String> {
        let data_dir =
            dirs::data_dir().ok_or_else(|| "Could not find application data directory".to_string())?;
        Ok(app_models_dir(&data_dir))
    }

    fn reset(&mut self) {
        tracing::info!(target: "pipeline", "whisper: releasing cached state and model");
        drop(self.state.take());
        drop(self.context.take());
        self.loaded_model_name = None;
    }
}

/// Map a frontend language setting to whisper's `set_language` argument.
/// `"auto"` (and an empty string, as a defensive fallback) => `None`, which
/// makes whisper auto-detect the spoken language. Any other value is passed
/// through as an ISO code for whisper to honor. Whisper-only: other backends
/// ignore the language entirely.
fn whisper_language_param(language: &str) -> Option<&str> {
    match language {
        "auto" | "" => None,
        other => Some(other),
    }
}

/// Append a whisper segment's text to the accumulated transcript, inserting a
/// separating space only when neither side already has whitespace at the
/// join. Whisper.cpp segments *usually* carry their own leading space (it's
/// part of the BPE token), which is why the single-segment path historically
/// got away with a bare `push_str`. Multi-segment decoding (see
/// [`should_use_single_segment`]) makes the join point observable more often,
/// so this guards against words gluing together across a segment boundary
/// (e.g. "...dictating" + "The next..." => "...dictatingThe next...").
fn append_segment(text: &mut String, segment_text: &str) {
    let needs_space = !text.is_empty()
        && !text.ends_with(char::is_whitespace)
        && !segment_text.starts_with(char::is_whitespace);
    if needs_space {
        text.push(' ');
    }
    text.push_str(segment_text);
}

fn strip_punctuation(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut result = String::with_capacity(input.len());

    for (i, &c) in chars.iter().enumerate() {
        match c {
            // Apostrophe: keep if between two alphanumeric chars (contractions)
            '\'' | '\u{2019}' => {
                let prev_alnum = i > 0 && chars[i - 1].is_alphanumeric();
                let next_alnum = i + 1 < chars.len() && chars[i + 1].is_alphanumeric();
                if prev_alnum && next_alnum {
                    result.push(c);
                }
            }
            // Hyphen: keep if between two alphanumeric chars (compound words)
            '-' => {
                let prev_alnum = i > 0 && chars[i - 1].is_alphanumeric();
                let next_alnum = i + 1 < chars.len() && chars[i + 1].is_alphanumeric();
                if prev_alnum && next_alnum {
                    result.push(c);
                }
            }
            // Strip sentence and quotation punctuation
            '.' | ',' | '!' | '?' | ';' | ':' | '"' | '\u{201C}' | '\u{201D}'
            | '\u{2018}' | '\u{2014}' | '\u{2013}' | '\u{2026}'
            | '\u{AB}' | '\u{BB}' | '\u{BF}' | '\u{A1}'
            | '\u{3002}' | '\u{3001}' | '\u{FF01}' | '\u{FF1F}'
            | '\u{30FB}' | '\u{300C}' | '\u{300D}' | '\u{300E}' | '\u{300F}' => result.push(' '),
            _ => result.push(c),
        }
    }

    result.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::{
        append_segment, get_model_path, should_use_single_segment, specific_model_exists,
        strip_punctuation, whisper_language_param, TranscriptionBackend, WhisperBackend,
    };

    #[test]
    fn language_auto_maps_to_none() {
        assert_eq!(whisper_language_param("auto"), None);
    }

    #[test]
    fn language_empty_maps_to_none() {
        // Defensive: an empty/unset value should fall back to auto-detect, not
        // be passed to whisper as a bogus empty language code.
        assert_eq!(whisper_language_param(""), None);
    }

    #[test]
    fn language_iso_code_passes_through() {
        assert_eq!(whisper_language_param("en"), Some("en"));
        assert_eq!(whisper_language_param("es"), Some("es"));
        assert_eq!(whisper_language_param("ja"), Some("ja"));
    }

    #[test]
    fn strip_basic_sentence_punctuation() {
        assert_eq!(strip_punctuation("Hello, world!"), "Hello world");
    }

    #[test]
    fn strip_preserves_apostrophe_in_contraction() {
        assert_eq!(strip_punctuation("Don't do that."), "Don't do that");
    }

    #[test]
    fn strip_preserves_hyphen_in_compound() {
        assert_eq!(strip_punctuation("It's state-of-the-art!"), "It's state-of-the-art");
    }

    #[test]
    fn strip_unicode_dashes_and_ellipsis() {
        assert_eq!(strip_punctuation("Hello\u{2026} world\u{2014}really?"), "Hello world really");
    }

    #[test]
    fn strip_empty_string() {
        assert_eq!(strip_punctuation(""), "");
    }

    #[test]
    fn strip_whitespace_only_with_punctuation() {
        assert_eq!(strip_punctuation("   .   "), "");
    }

    #[test]
    fn strip_preserves_french_contraction() {
        assert_eq!(strip_punctuation("c'est la vie!"), "c'est la vie");
    }

    // --- should_use_single_segment ---------------------------------------

    #[test]
    fn streaming_windows_stay_single_segment() {
        // Streaming's incremental decode window (see transcriber::chunking::
        // WINDOW_SAMPLES) is 10s of 16kHz audio. That must remain
        // single-segment: streaming behavior is correct today and this issue
        // must not change it.
        let window_samples = 10 * super::super::WHISPER_SAMPLE_RATE as usize;
        assert!(should_use_single_segment(window_samples));
    }

    #[test]
    fn streaming_step_size_stays_single_segment() {
        // STEP_SAMPLES (8s) is smaller than WINDOW_SAMPLES but exercise it
        // too since some call sites may transcribe partial windows.
        let step_samples = 8 * super::super::WHISPER_SAMPLE_RATE as usize;
        assert!(should_use_single_segment(step_samples));
    }

    #[test]
    fn xlong_batch_audio_disables_single_segment() {
        // The production repro: a 28s batch/file transcription must not be
        // constrained to a single segment, or whisper.cpp's early-EOT
        // force-skip silently drops the tail.
        let xlong_samples = 28 * super::super::WHISPER_SAMPLE_RATE as usize;
        assert!(!should_use_single_segment(xlong_samples));
    }

    #[test]
    fn boundary_exactly_at_threshold_is_single_segment() {
        let boundary = 12 * super::super::WHISPER_SAMPLE_RATE as usize;
        assert!(should_use_single_segment(boundary));
    }

    #[test]
    fn boundary_one_sample_over_threshold_is_multi_segment() {
        let boundary = 12 * super::super::WHISPER_SAMPLE_RATE as usize + 1;
        assert!(!should_use_single_segment(boundary));
    }

    #[test]
    fn zero_samples_is_single_segment() {
        assert!(should_use_single_segment(0));
    }

    // --- append_segment ----------------------------------------------------

    #[test]
    fn append_segment_first_segment_starts_text() {
        let mut text = String::new();
        append_segment(&mut text, " Hello");
        assert_eq!(text, " Hello");
    }

    #[test]
    fn append_segment_preserves_existing_leading_space() {
        // Mirrors whisper.cpp's normal behavior: each segment's text already
        // carries its own leading space as part of the BPE token, so no
        // extra separator should be inserted.
        let mut text = String::new();
        append_segment(&mut text, " Hello");
        append_segment(&mut text, " world.");
        assert_eq!(text.trim(), "Hello world.");
    }

    #[test]
    fn append_segment_inserts_space_when_both_sides_lack_one() {
        // Defensive case: if a segment boundary ever lacks whitespace on
        // both sides, words must not glue together.
        let mut text = String::new();
        append_segment(&mut text, "Hello");
        append_segment(&mut text, "world");
        assert_eq!(text, "Hello world");
    }

    #[test]
    fn append_segment_does_not_double_space() {
        let mut text = String::new();
        append_segment(&mut text, "Hello ");
        append_segment(&mut text, " world");
        assert_eq!(text, "Hello  world");
    }

    // --- model-backed repro + fix proof (require installed .en models) ----
    //
    // Run on a machine with the models installed:
    //   cargo test -- --ignored --nocapture --test-threads=1 xlong

    /// Reproduces the bug directly: same model, same audio, only the
    /// `single_segment` flag flipped. `single_segment = true` must truncate
    /// the 28s xlong fixture (fewer words recovered) relative to
    /// `single_segment = false`, matching the production repro in issue #269.
    #[test]
    #[ignore]
    fn xlong_truncation_repro_single_segment() {
        use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

        let wav = include_bytes!("../../../../bench/audio/xlong.wav");
        let samples =
            crate::transcriber::parse_wav_to_samples(wav).expect("decode xlong fixture");

        let mut ran_any = false;
        for model_name in ["tiny.en", "base.en"] {
            let model_path = match get_model_path(model_name) {
                Ok(path) => path,
                Err(_) => {
                    eprintln!("skipping {model_name}: model not installed");
                    continue;
                }
            };
            ran_any = true;
            let path_str = model_path.to_str().expect("model path is valid UTF-8");

            let run = |single_segment: bool| -> String {
                let ctx = WhisperContext::new_with_params(
                    path_str,
                    WhisperContextParameters::default(),
                )
                .expect("load whisper model");
                let mut state = ctx.create_state().expect("create whisper state");
                let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
                params.set_language(Some("en"));
                params.set_print_special(false);
                params.set_print_progress(false);
                params.set_print_realtime(false);
                params.set_print_timestamps(false);
                params.set_suppress_blank(true);
                params.set_single_segment(single_segment);
                state.full(params, &samples).expect("transcribe xlong fixture");

                let mut text = String::new();
                for i in 0..state.full_n_segments() {
                    let segment = state.get_segment(i).expect("segment");
                    let segment_text = segment.to_str().expect("segment text");
                    append_segment(&mut text, segment_text);
                }
                text.trim().to_string()
            };

            let truncated = run(true);
            let complete = run(false);
            let truncated_words = truncated.split_whitespace().count();
            let complete_words = complete.split_whitespace().count();

            eprintln!(
                "{model_name}: single_segment=true -> {truncated_words} words: {truncated:?}"
            );
            eprintln!(
                "{model_name}: single_segment=false -> {complete_words} words: {complete:?}"
            );

            assert!(
                complete_words > truncated_words,
                "{model_name}: expected single_segment=false to recover more words than \
                 single_segment=true (got {complete_words} vs {truncated_words}); this was \
                 supposed to reproduce the issue #269 truncation"
            );
        }

        if !ran_any {
            panic!(
                "no whisper .en models installed to run this repro (need at least one of \
                 tiny.en, base.en)"
            );
        }
    }

    /// Proves the fix: the normal, public `transcribe()` path (which now
    /// picks `single_segment` via [`should_use_single_segment`]) recovers the
    /// xlong fixture's final clause ("...throughout the day") instead of
    /// truncating at "...for dictating" as it did before this issue's fix.
    #[test]
    #[ignore]
    fn xlong_batch_transcription_recovers_full_tail_after_fix() {
        let wav = include_bytes!("../../../../bench/audio/xlong.wav");
        let samples =
            crate::transcriber::parse_wav_to_samples(wav).expect("decode xlong fixture");

        let mut ran_any = false;
        for model_name in ["tiny.en", "base.en"] {
            if !specific_model_exists(model_name) {
                eprintln!("skipping {model_name}: model not installed");
                continue;
            }
            ran_any = true;

            let mut backend = WhisperBackend::new();
            backend.load_model(model_name).expect("load model");
            let text = backend
                .transcribe(&samples, "en", None, true)
                .expect("transcribe xlong fixture");

            eprintln!(
                "{model_name}: {} words: {text:?}",
                text.split_whitespace().count()
            );

            let lower = text.to_lowercase();
            assert!(
                lower.contains("throughout the day"),
                "{model_name}: expected the fixed batch transcribe() to recover the fixture's \
                 final clause ('...throughout the day'), but truncated output was: {text:?}"
            );
            assert!(
                !lower.trim_end().ends_with("for dictating"),
                "{model_name}: transcribe() still truncates at 'for dictating', got: {text:?}"
            );
        }

        if !ran_any {
            panic!(
                "no whisper .en models installed to run this fix proof (need at least one of \
                 tiny.en, base.en)"
            );
        }
    }
}
