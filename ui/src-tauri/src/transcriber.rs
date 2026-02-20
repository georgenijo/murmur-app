use crate::state::WHISPER_SAMPLE_RATE;
use hound::WavReader;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Once;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, install_whisper_log_trampoline};

static INIT_LOGGING: Once = Once::new();

/// Suppress whisper.cpp verbose logging by installing a trampoline that routes to Rust's log crate
/// (which we don't configure, so logs go nowhere)
fn suppress_whisper_logs() {
    INIT_LOGGING.call_once(|| {
        // This routes whisper.cpp logs through Rust's log crate
        // Since we don't have a logger configured, they get discarded
        install_whisper_log_trampoline();
    });
}

/// Get all potential model directories to search
fn get_model_search_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    // Check environment variable first
    if let Ok(custom_path) = std::env::var("WHISPER_MODEL_DIR") {
        paths.push(PathBuf::from(custom_path));
    }

    // App's own data directory
    if let Some(data_dir) = dirs::data_dir() {
        paths.push(data_dir.join("local-dictation").join("models"));
        paths.push(data_dir.join("pywhispercpp").join("models"));
    }

    // Home directory locations
    if let Some(home) = dirs::home_dir() {
        paths.push(home.join(".cache").join("whisper.cpp"));
        paths.push(home.join(".cache").join("whisper"));
        paths.push(home.join(".whisper").join("models"));
    }

    paths
}

/// Get the path to a specific model file, searching multiple locations
pub fn get_model_path(model_name: &str) -> Result<PathBuf, String> {
    let filename = format!("ggml-{}.bin", model_name);
    let search_paths = get_model_search_paths();

    for dir in &search_paths {
        let path = dir.join(&filename);
        if path.exists() {
            return Ok(path);
        }
    }

    // Model not found - provide helpful error message
    let searched_locations = search_paths
        .iter()
        .map(|p| format!("  - {}", p.display()))
        .collect::<Vec<_>>()
        .join("\n");

    Err(format!(
        "Model '{}' not found. Searched locations:\n{}\n\nDownload from: https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}",
        filename,
        searched_locations,
        filename
    ))
}

/// Check if any whisper model (.bin) file exists in any of the search paths
pub fn check_model_exists() -> bool {
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

/// Get the primary models directory (for downloads)
pub fn get_models_dir() -> Result<PathBuf, String> {
    let data_dir = dirs::data_dir()
        .ok_or_else(|| "Could not find application data directory".to_string())?;
    Ok(data_dir.join("local-dictation").join("models"))
}

/// Initialize a WhisperContext for the given model
pub fn init_whisper_context(model_name: &str) -> Result<WhisperContext, String> {
    // Suppress verbose whisper.cpp logging
    suppress_whisper_logs();

    let model_path = get_model_path(model_name)?;
    let path_str = model_path.to_str()
        .ok_or_else(|| "Model path contains invalid UTF-8 characters".to_string())?;

    let params = WhisperContextParameters::default();
    WhisperContext::new_with_params(path_str, params)
        .map_err(|e| format!("Failed to load whisper model: {}", e))
}

/// Parse WAV audio bytes and convert to f32 samples for whisper
pub fn parse_wav_to_samples(wav_bytes: &[u8]) -> Result<Vec<f32>, String> {
    let cursor = Cursor::new(wav_bytes);
    let reader = WavReader::new(cursor)
        .map_err(|e| format!("Failed to parse WAV: {}", e))?;

    let spec = reader.spec();

    // Whisper expects 16kHz mono audio
    if spec.sample_rate != WHISPER_SAMPLE_RATE {
        return Err(format!("Expected {}Hz sample rate, got {}", WHISPER_SAMPLE_RATE, spec.sample_rate));
    }
    if spec.channels != 1 {
        return Err(format!("Expected mono audio, got {} channels", spec.channels));
    }

    // Convert i16 samples to f32 (normalized to -1.0 to 1.0)
    let samples: Vec<f32> = reader
        .into_samples::<i16>()
        .map(|s| s.map(|v| v as f32 / i16::MAX as f32))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to decode WAV samples: {}", e))?;

    Ok(samples)
}

/// Normalize a whisper segment's text for streaming display.
/// Uses trim_end() (not trim()) to preserve leading spaces that act as word separators
/// between segments. Without leading spaces, concatenated segments run words together.
fn normalize_segment_text(raw: &str) -> Option<String> {
    let text = raw.trim_end().to_string();
    if text.is_empty() { None } else { Some(text) }
}

/// Transcribe audio samples using the given WhisperContext and an explicit SamplingStrategy.
///
/// `on_segment`: optional callback fired as each segment is decoded during inference.
/// Enables live display of partial text in the UI while whisper is still running.
pub fn transcribe_with_strategy(
    ctx: &WhisperContext,
    samples: &[f32],
    language: &str,
    strategy: SamplingStrategy,
    on_segment: Option<impl Fn(String) + Send + 'static>,
) -> Result<String, String> {
    let mut state = ctx.create_state()
        .map_err(|e| format!("Failed to create whisper state: {}", e))?;

    let mut params = FullParams::new(strategy);
    params.set_language(Some(language));
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_suppress_blank(true);
    // Note: set_single_segment is intentionally omitted so that whisper fires
    // new_segment_callback progressively as each segment is decoded.
    params.set_debug_mode(false);

    if let Some(cb) = on_segment {
        // set_segment_callback_safe_lossy handles the unsafe FFI trampoline internally.
        // The closure must be FnMut — we wrap the Fn in a mut wrapper.
        let mut cb = cb;
        params.set_segment_callback_safe_lossy(move |data: whisper_rs::SegmentCallbackData| {
            if let Some(text) = normalize_segment_text(&data.text) {
                cb(text);
            }
        });
    }

    state.full(params, samples)
        .map_err(|e| format!("Transcription failed: {}", e))?;

    let num_segments = state.full_n_segments()
        .map_err(|e| format!("Failed to get segments: {}", e))?;

    let mut text = String::new();
    for i in 0..num_segments {
        let segment = state
            .full_get_segment_text(i)
            .map_err(|e| format!("Failed to get segment {}: {}", i, e))?;
        text.push_str(&segment);
    }

    Ok(text.trim().to_string())
}

/// Transcribe audio samples using the default Greedy strategy.
///
/// This is the production entry point. Use `transcribe_with_strategy` to specify
/// a different sampling strategy (e.g. for benchmarking).
pub fn transcribe(
    ctx: &WhisperContext,
    samples: &[f32],
    language: &str,
    on_segment: Option<impl Fn(String) + Send + 'static>,
) -> Result<String, String> {
    transcribe_with_strategy(ctx, samples, language, SamplingStrategy::Greedy { best_of: 1 }, on_segment)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::c_int;
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    #[test]
    fn segment_text_preserves_leading_space() {
        // Whisper segments typically start with a leading space as a word separator
        assert_eq!(normalize_segment_text(" hello world"), Some(" hello world".into()));
    }

    #[test]
    fn segment_text_strips_trailing_whitespace() {
        assert_eq!(normalize_segment_text(" hello world  \n"), Some(" hello world".into()));
    }

    #[test]
    fn segment_text_rejects_empty() {
        assert_eq!(normalize_segment_text(""), None);
        assert_eq!(normalize_segment_text("   "), None);
        assert_eq!(normalize_segment_text("\n"), None);
    }

    #[test]
    fn segment_text_concatenation_preserves_spacing() {
        // Simulates how the frontend concatenates streamed segments
        let segments = vec![" The quick brown", " fox jumped", " over the lazy dog"];
        let result: String = segments.iter()
            .filter_map(|s| normalize_segment_text(s))
            .collect();
        assert_eq!(result, " The quick brown fox jumped over the lazy dog");
    }

    const DEFAULT_MODEL: &str = "base.en";

    fn load_test_samples() -> Vec<f32> {
        let wav_path = std::env::var("BENCH_AUDIO_WAV")
            .expect("BENCH_AUDIO_WAV env var required — point it at a 16kHz mono WAV file.\n\
                     Record one with: ffmpeg -f avfoundation -i \":0\" -ar 16000 -ac 1 -t 5 /tmp/bench.wav");
        let wav_bytes = std::fs::read(&wav_path)
            .unwrap_or_else(|e| panic!("Failed to read {}: {}", wav_path, e));
        parse_wav_to_samples(&wav_bytes)
            .unwrap_or_else(|e| panic!("Failed to parse {}: {}", wav_path, e))
    }

    fn load_bench_context() -> WhisperContext {
        let model = std::env::var("BENCH_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());
        eprintln!("Loading model: {} (override with BENCH_MODEL env var)", model);
        init_whisper_context(&model)
            .unwrap_or_else(|e| panic!("Failed to load model '{}': {}", model, e))
    }

    struct BenchConfig {
        name: String,
        strategy: SamplingStrategy,
        temperature: Option<f32>,
        temperature_inc: Option<f32>,
        n_threads: Option<c_int>,
    }

    struct BenchResult {
        name: String,
        total_ms: u128,
        ttfs_ms: Option<u128>,
        segment_count: usize,
        text: String,
    }

    impl std::fmt::Display for BenchResult {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{:<35} total={:>6}ms  ttfs={:>6}  segments={:>2}  chars={}",
                self.name,
                self.total_ms,
                self.ttfs_ms.map_or("n/a".to_string(), |t| format!("{}ms", t)),
                self.segment_count,
                self.text.len(),
            )
        }
    }

    fn run_bench(ctx: &WhisperContext, samples: &[f32], language: &str, config: &BenchConfig) -> BenchResult {
        let first_segment_at: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
        let segment_count = Arc::new(Mutex::new(0usize));

        let fst = Arc::clone(&first_segment_at);
        let sc = Arc::clone(&segment_count);

        let on_segment = move |_text: String| {
            let mut count = sc.lock().unwrap();
            *count += 1;
            if *count == 1 {
                *fst.lock().unwrap() = Some(Instant::now());
            }
        };

        let mut state = ctx.create_state().expect("Failed to create whisper state");
        let mut params = FullParams::new(config.strategy.clone());
        params.set_language(Some(language));
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_suppress_blank(true);
        params.set_debug_mode(false);

        if let Some(t) = config.temperature {
            params.set_temperature(t);
        }
        if let Some(ti) = config.temperature_inc {
            params.set_temperature_inc(ti);
        }
        if let Some(n) = config.n_threads {
            params.set_n_threads(n);
        }

        let mut cb = on_segment;
        params.set_segment_callback_safe_lossy(move |data: whisper_rs::SegmentCallbackData| {
            let text = data.text.trim().to_string();
            if !text.is_empty() {
                cb(text);
            }
        });

        let start = Instant::now();
        let inference_ok = state.full(params, samples);
        let total_elapsed = start.elapsed();

        let text = if inference_ok.is_ok() {
            let n = state.full_n_segments().unwrap_or(0);
            let mut t = String::new();
            for i in 0..n {
                if let Ok(seg) = state.full_get_segment_text(i) {
                    t.push_str(&seg);
                }
            }
            t.trim().to_string()
        } else {
            eprintln!("  [{}] transcription failed: {:?}", config.name, inference_ok);
            String::new()
        };

        let ttfs_ms = first_segment_at.lock().unwrap().map(|at| (at - start).as_millis());
        let segments = *segment_count.lock().unwrap();

        BenchResult {
            name: config.name.clone(),
            total_ms: total_elapsed.as_millis(),
            ttfs_ms,
            segment_count: segments,
            text,
        }
    }

    #[test]
    #[ignore] // Requires model + GPU + BENCH_AUDIO_WAV env var
    fn benchmark_strategies() {
        let ctx = load_bench_context();
        let samples = load_test_samples();
        let language = std::env::var("BENCH_LANG").unwrap_or_else(|_| "en".to_string());

        let configs = vec![
            // Baseline: whisper default
            BenchConfig {
                name: "BeamSearch(5)".into(),
                strategy: SamplingStrategy::BeamSearch { beam_size: 5, patience: -1.0 },
                temperature: None, temperature_inc: None, n_threads: None,
            },
            // Current production setting
            BenchConfig {
                name: "Greedy(1)".into(),
                strategy: SamplingStrategy::Greedy { best_of: 1 },
                temperature: None, temperature_inc: None, n_threads: None,
            },
            // Middle ground beam
            BenchConfig {
                name: "BeamSearch(2)".into(),
                strategy: SamplingStrategy::BeamSearch { beam_size: 2, patience: -1.0 },
                temperature: None, temperature_inc: None, n_threads: None,
            },
            // Greedy + temp=0 (no fallback retries)
            BenchConfig {
                name: "Greedy(1)+temp0".into(),
                strategy: SamplingStrategy::Greedy { best_of: 1 },
                temperature: Some(0.0), temperature_inc: Some(0.0), n_threads: None,
            },
            // Greedy + temp=0 + 4 threads (perf cores only)
            BenchConfig {
                name: "Greedy(1)+temp0+4t".into(),
                strategy: SamplingStrategy::Greedy { best_of: 1 },
                temperature: Some(0.0), temperature_inc: Some(0.0), n_threads: Some(4),
            },
            // Greedy + temp=0 + 8 threads
            BenchConfig {
                name: "Greedy(1)+temp0+8t".into(),
                strategy: SamplingStrategy::Greedy { best_of: 1 },
                temperature: Some(0.0), temperature_inc: Some(0.0), n_threads: Some(8),
            },
        ];

        eprintln!("\n=== Whisper Transcription Strategy Benchmark ===");
        eprintln!("Audio: {} samples ({:.1}s at {}Hz)",
            samples.len(),
            samples.len() as f64 / WHISPER_SAMPLE_RATE as f64,
            WHISPER_SAMPLE_RATE
        );
        eprintln!("{}\n", "-".repeat(80));

        let mut results: Vec<BenchResult> = Vec::new();
        for config in &configs {
            let result = run_bench(&ctx, &samples, &language, config);
            eprintln!("{}", result);
            eprintln!("  text: {:?}\n", result.text);
            results.push(result);
        }

        // Summary table
        let baseline_ms = results[0].total_ms;
        let baseline_ttfs = results[0].ttfs_ms;
        eprintln!("{}", "=".repeat(80));
        eprintln!("{:<35} {:>8} {:>8} {:>10} {:>10}", "Strategy", "Total", "TTFS", "vs Base", "TTFS vs");
        eprintln!("{}", "-".repeat(80));
        for r in &results {
            let vs_base = if baseline_ms > 0 {
                format!("{:.2}x", baseline_ms as f64 / r.total_ms as f64)
            } else { "n/a".into() };
            let vs_ttfs = match (baseline_ttfs, r.ttfs_ms) {
                (Some(b), Some(t)) if t > 0 => format!("{:.2}x", b as f64 / t as f64),
                _ => "n/a".into(),
            };
            eprintln!("{:<35} {:>6}ms {:>6} {:>10} {:>10}",
                r.name,
                r.total_ms,
                r.ttfs_ms.map_or("n/a".to_string(), |t| format!("{}ms", t)),
                vs_base,
                vs_ttfs,
            );
        }
        eprintln!("{}\n", "=".repeat(80));
    }
}
