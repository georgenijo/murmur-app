use super::TranscriptionBackend;
use std::path::{Path, PathBuf};
use std::sync::Once;
use whisper_rs::{
    install_logging_hooks, FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters,
    WhisperState,
};

static INIT_LOGGING: Once = Once::new();

/// Short audio retains the established single-segment decode behavior, while
/// longer batch decodes need Whisper's timestamp-based continuation after an
/// early end-of-text token.
const SINGLE_SEGMENT_MAX_SAMPLES: usize = 12 * super::WHISPER_SAMPLE_RATE as usize;

/// Relative path under the platform data directory for app models.
const APP_MODELS_REL: &[&str] = &["local-dictation", "models"];

/// Suppress whisper.cpp verbose logging by installing a trampoline that routes to Rust's log crate
/// (which we don't configure, so logs go nowhere).
fn suppress_whisper_logs() {
    INIT_LOGGING.call_once(|| {
        install_logging_hooks();
    });
}

/// Build the app's primary models directory from a data dir root.
fn app_models_dir(data_dir: &Path) -> PathBuf {
    APP_MODELS_REL
        .iter()
        .fold(data_dir.to_path_buf(), |p, s| p.join(s))
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

    fn transcribe_with_single_segment(
        &mut self,
        samples: &[f32],
        language: &str,
        initial_prompt: Option<&str>,
        smart_punctuation: bool,
        single_segment: bool,
    ) -> Result<String, String> {
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
        params.set_single_segment(single_segment);
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
}

fn should_use_single_segment(sample_count: usize) -> bool {
    sample_count <= SINGLE_SEGMENT_MAX_SAMPLES
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
    fn transcribe(
        &mut self,
        samples: &[f32],
        language: &str,
        initial_prompt: Option<&str>,
        smart_punctuation: bool,
    ) -> Result<String, String> {
        self.transcribe_with_single_segment(
            samples,
            language,
            initial_prompt,
            smart_punctuation,
            should_use_single_segment(samples.len()),
        )
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
        let data_dir = dirs::data_dir()
            .ok_or_else(|| "Could not find application data directory".to_string())?;
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
            '.' | ',' | '!' | '?' | ';' | ':' | '"' | '\u{201C}' | '\u{201D}' | '\u{2018}'
            | '\u{2014}' | '\u{2013}' | '\u{2026}' | '\u{AB}' | '\u{BB}' | '\u{BF}' | '\u{A1}'
            | '\u{3002}' | '\u{3001}' | '\u{FF01}' | '\u{FF1F}' | '\u{30FB}' | '\u{300C}'
            | '\u{300D}' | '\u{300E}' | '\u{300F}' => result.push(' '),
            _ => result.push(c),
        }
    }

    result.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::{
        append_segment, should_use_single_segment, specific_model_exists, strip_punctuation,
        whisper_language_param, WhisperBackend, SINGLE_SEGMENT_MAX_SAMPLES,
    };
    use crate::transcriber::{parse_wav_to_samples, TranscriptionBackend};

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

    /// Proves the fix on the public path: `transcribe()` (which picks
    /// `single_segment` via [`should_use_single_segment`]) recovers the xlong
    /// fixture's final clause ("...throughout the day") instead of truncating
    /// at "...for dictating" as it did before the issue #269 fix.
    #[test]
    #[ignore = "requires installed tiny.en/base.en models and runs Whisper inference"]
    fn xlong_batch_transcription_recovers_full_tail_after_fix() {
        let wav = include_bytes!("../../../../bench/audio/xlong.wav");
        let samples = parse_wav_to_samples(wav).expect("decode xlong fixture");

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
                 final clause ('...throughout the day'), but got: {text:?}"
            );
            assert!(
                !lower.trim_end().ends_with("for dictating"),
                "{model_name}: transcribe() still truncates at 'for dictating', got: {text:?}"
            );
        }

        if !ran_any {
            eprintln!("skipping xlong fix proof: no whisper .en models installed");
        }
    }

    #[test]
    fn short_batch_audio_keeps_single_segment_mode() {
        assert!(should_use_single_segment(
            10 * crate::transcriber::WHISPER_SAMPLE_RATE as usize
        ));
    }

    #[test]
    fn single_segment_duration_boundary_is_inclusive() {
        assert!(should_use_single_segment(SINGLE_SEGMENT_MAX_SAMPLES));
        assert!(!should_use_single_segment(SINGLE_SEGMENT_MAX_SAMPLES + 1));
    }

    #[test]
    #[ignore = "requires an installed base.en model and runs Whisper inference"]
    fn xlong_truncation_repro_single_segment() {
        if !specific_model_exists("base.en") {
            eprintln!("skipping xlong regression: base.en is not installed");
            return;
        }

        let samples = parse_wav_to_samples(include_bytes!("../../../../bench/audio/xlong.wav"))
            .expect("xlong fixture should decode");
        assert!(!should_use_single_segment(samples.len()));

        let mut backend = WhisperBackend::new();
        backend.load_model("base.en").expect("base.en should load");
        let single_segment = backend
            .transcribe_with_single_segment(&samples, "en", None, true, true)
            .expect("single-segment transcription should succeed");
        let continued = backend
            .transcribe_with_single_segment(&samples, "en", None, true, false)
            .expect("timestamp-based continuation should succeed");

        eprintln!("single_segment=true: {single_segment}");
        eprintln!("single_segment=false: {continued}");

        let single_words = single_segment.split_whitespace().count();
        let continued_words = continued.split_whitespace().count();
        assert!(
            continued_words >= single_words + 4,
            "continuation should recover a meaningful tail: single={single_words}, continued={continued_words}"
        );

        let tail_terms = ["code", "comments", "chat", "messages", "throughout", "day"];
        let normalized_single = single_segment.to_ascii_lowercase();
        let normalized_continued = continued.to_ascii_lowercase();
        let single_tail_hits = tail_terms
            .iter()
            .filter(|term| {
                normalized_single
                    .split(|c: char| !c.is_alphanumeric())
                    .any(|word| word == **term)
            })
            .count();
        let continued_tail_hits = tail_terms
            .iter()
            .filter(|term| {
                normalized_continued
                    .split(|c: char| !c.is_alphanumeric())
                    .any(|word| word == **term)
            })
            .count();
        assert!(
            continued_tail_hits >= 4 && continued_tail_hits > single_tail_hits,
            "continued transcript should recover the reference tail: single_hits={single_tail_hits}, continued_hits={continued_tail_hits}"
        );
    }

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
        assert_eq!(
            strip_punctuation("It's state-of-the-art!"),
            "It's state-of-the-art"
        );
    }

    #[test]
    fn strip_unicode_dashes_and_ellipsis() {
        assert_eq!(
            strip_punctuation("Hello\u{2026} world\u{2014}really?"),
            "Hello world really"
        );
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
}
