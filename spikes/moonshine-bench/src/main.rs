use std::path::{Path, PathBuf};
use std::time::Instant;

use sherpa_rs::moonshine::{MoonshineConfig, MoonshineRecognizer};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

const RUNS: usize = 3;

struct BenchResult {
    model: String,
    clip: String,
    first_token_ms: f64,
    total_ms: f64,
    peak_rss_mb: f64,
    output: String,
}

/// Get current process RSS in MB via ps (simple, accurate on macOS)
fn current_rss_mb() -> f64 {
    std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse::<f64>().ok())
        .map(|kb| kb / 1024.0)
        .unwrap_or(0.0)
}

/// Load 16kHz mono WAV as f32 samples normalized to [-1, 1]
fn load_wav(path: &Path) -> Vec<f32> {
    let reader = hound::WavReader::open(path)
        .unwrap_or_else(|e| panic!("Failed to open {}: {}", path.display(), e));
    let spec = reader.spec();
    assert_eq!(spec.sample_rate, 16000, "Expected 16kHz, got {}", spec.sample_rate);

    reader
        .into_samples::<i16>()
        .map(|s| s.unwrap() as f32 / 32768.0)
        .collect()
}

/// Search known macOS directories for a whisper ggml model file
fn find_whisper_model(name: &str) -> PathBuf {
    let home = std::env::var("HOME").expect("HOME not set");
    let filename = format!("ggml-{}.bin", name);

    let search_dirs = [
        format!("{}/Library/Application Support/local-dictation/models", home),
        format!(
            "{}/Library/Application Support/pywhispercpp/models",
            home
        ),
        format!("{}/.cache/whisper.cpp", home),
        format!("{}/.cache/whisper", home),
        format!("{}/.whisper/models", home),
    ];

    for dir in &search_dirs {
        let path = PathBuf::from(dir).join(&filename);
        if path.exists() {
            return path;
        }
    }

    panic!(
        "Whisper model '{}' ({}) not found. Searched: {:?}",
        name, filename, search_dirs
    );
}

fn bench_whisper(
    model_name: &str,
    model_path: &Path,
    clips: &[(&str, &[f32])],
) -> Vec<BenchResult> {
    let rss_before = current_rss_mb();

    let ctx = WhisperContext::new_with_params(
        model_path.to_str().unwrap(),
        WhisperContextParameters::default(),
    )
    .expect("Failed to load whisper model");

    let rss_after = current_rss_mb();
    eprintln!(
        "  Loaded {} ({:.0} MB -> {:.0} MB, +{:.0} MB)",
        model_name,
        rss_before,
        rss_after,
        rss_after - rss_before
    );

    let mut results = Vec::new();

    for (clip_name, samples) in clips {
        let mut timings: Vec<f64> = Vec::new();
        let mut last_text = String::new();
        let mut max_rss = 0.0f64;

        for run_idx in 0..RUNS {
            let mut state = ctx.create_state().expect("Failed to create whisper state");

            let start = Instant::now();

            let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
            params.set_language(Some("en"));
            params.set_print_special(false);
            params.set_print_progress(false);
            params.set_print_realtime(false);
            params.set_print_timestamps(false);
            params.set_suppress_blank(true);
            params.set_single_segment(true);
            params.set_debug_mode(false);

            state.full(params, samples).expect("Transcription failed");
            let total = start.elapsed();

            let n_segments = state.full_n_segments().unwrap_or(0);
            let mut text = String::new();
            for i in 0..n_segments {
                if let Ok(seg) = state.full_get_segment_text(i) {
                    text.push_str(&seg);
                }
            }
            last_text = text.trim().to_string();

            let rss = current_rss_mb();
            max_rss = max_rss.max(rss);
            timings.push(total.as_secs_f64() * 1000.0);

            if run_idx == 0 {
                eprintln!(
                    "    {} run 1: {:.0}ms \"{}\"",
                    clip_name,
                    total.as_millis(),
                    last_text.chars().take(60).collect::<String>()
                );
            }
        }

        let avg_total = timings.iter().sum::<f64>() / RUNS as f64;

        results.push(BenchResult {
            model: model_name.to_string(),
            clip: clip_name.to_string(),
            first_token_ms: avg_total, // offline batch: first-token = total
            total_ms: avg_total,
            peak_rss_mb: max_rss,
            output: last_text,
        });
    }

    results
}

fn bench_moonshine(
    model_name: &str,
    model_dir: &Path,
    clips: &[(&str, &[f32])],
) -> Vec<BenchResult> {
    let rss_before = current_rss_mb();

    let config = MoonshineConfig {
        preprocessor: model_dir
            .join("preprocess.onnx")
            .to_str()
            .unwrap()
            .to_string(),
        encoder: model_dir
            .join("encode.int8.onnx")
            .to_str()
            .unwrap()
            .to_string(),
        uncached_decoder: model_dir
            .join("uncached_decode.int8.onnx")
            .to_str()
            .unwrap()
            .to_string(),
        cached_decoder: model_dir
            .join("cached_decode.int8.onnx")
            .to_str()
            .unwrap()
            .to_string(),
        tokens: model_dir
            .join("tokens.txt")
            .to_str()
            .unwrap()
            .to_string(),
        provider: Some("cpu".to_string()),
        num_threads: None,
        ..Default::default()
    };

    let mut recognizer =
        MoonshineRecognizer::new(config).expect("Failed to load Moonshine model");

    let rss_after = current_rss_mb();
    eprintln!(
        "  Loaded {} ({:.0} MB -> {:.0} MB, +{:.0} MB)",
        model_name,
        rss_before,
        rss_after,
        rss_after - rss_before
    );

    let mut results = Vec::new();

    for (clip_name, samples) in clips {
        let mut timings: Vec<(f64, f64)> = Vec::new();
        let mut last_text = String::new();
        let mut max_rss = 0.0f64;

        for run_idx in 0..RUNS {
            let start = Instant::now();
            let result = recognizer.transcribe(16000, samples);
            let total = start.elapsed();

            last_text = result.text.trim().to_string();
            let rss = current_rss_mb();
            max_rss = max_rss.max(rss);

            // Moonshine offline: first-token = total
            timings.push((
                total.as_secs_f64() * 1000.0,
                total.as_secs_f64() * 1000.0,
            ));

            if run_idx == 0 {
                eprintln!(
                    "    {} run 1: {:.0}ms \"{}\"",
                    clip_name,
                    total.as_millis(),
                    last_text.chars().take(60).collect::<String>()
                );
            }
        }

        let avg_first = timings.iter().map(|t| t.0).sum::<f64>() / RUNS as f64;
        let avg_total = timings.iter().map(|t| t.1).sum::<f64>() / RUNS as f64;

        results.push(BenchResult {
            model: model_name.to_string(),
            clip: clip_name.to_string(),
            first_token_ms: avg_first,
            total_ms: avg_total,
            peak_rss_mb: max_rss,
            output: last_text,
        });
    }

    results
}

fn main() {
    eprintln!("=== Moonshine v2 vs whisper.cpp Benchmark ===\n");

    let bench_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixtures_dir = bench_dir.join("fixtures");
    let models_dir = bench_dir.join("models");

    // Load audio clips
    eprintln!("Loading audio clips...");
    let clips_data: Vec<(&str, Vec<f32>)> = [
        ("3s", fixtures_dir.join("test-3s.wav")),
        ("10s", fixtures_dir.join("test-10s.wav")),
        ("30s", fixtures_dir.join("test-30s.wav")),
    ]
    .iter()
    .map(|(name, path)| {
        let samples = load_wav(path);
        let dur = samples.len() as f64 / 16000.0;
        eprintln!("  {}: {} samples ({:.1}s)", name, samples.len(), dur);
        (*name, samples)
    })
    .collect();

    let clips: Vec<(&str, &[f32])> = clips_data
        .iter()
        .map(|(name, data)| (*name, data.as_slice()))
        .collect();

    let mut all_results: Vec<BenchResult> = Vec::new();

    // --- Whisper benchmarks (Metal GPU) ---
    eprintln!("\n--- whisper.cpp (Metal) ---");

    let whisper_base_path = find_whisper_model("base.en");
    eprintln!("  Path: {}", whisper_base_path.display());
    all_results.extend(bench_whisper("whisper base.en", &whisper_base_path, &clips));

    eprintln!();
    let whisper_turbo_path = find_whisper_model("large-v3-turbo");
    eprintln!("  Path: {}", whisper_turbo_path.display());
    all_results.extend(bench_whisper(
        "whisper large-v3-turbo",
        &whisper_turbo_path,
        &clips,
    ));

    // --- Moonshine benchmarks (CPU) ---
    eprintln!("\n--- Moonshine v2 (CPU, int8) ---");

    let moonshine_tiny_dir = models_dir.join("sherpa-onnx-moonshine-tiny-en-int8");
    all_results.extend(bench_moonshine("moonshine tiny", &moonshine_tiny_dir, &clips));

    eprintln!();
    let moonshine_base_dir = models_dir.join("sherpa-onnx-moonshine-base-en-int8");
    all_results.extend(bench_moonshine("moonshine base", &moonshine_base_dir, &clips));

    // --- Print markdown results table ---
    eprintln!("\n=== Results ===\n");

    println!("| Model | Clip | First Token (ms) | Total (ms) | Peak RSS (MB) | Output |");
    println!("|-------|------|-------------------|------------|---------------|--------|");

    for r in &all_results {
        let truncated = if r.output.len() > 80 {
            format!("{}...", &r.output[..77])
        } else {
            r.output.clone()
        };
        println!(
            "| {} | {} | {:.0} | {:.0} | {:.0} | {} |",
            r.model, r.clip, r.first_token_ms, r.total_ms, r.peak_rss_mb, truncated
        );
    }

    println!();
    println!("*Averaged over {} runs per configuration. Whisper uses Metal GPU; Moonshine uses CPU (int8 quantized).*", RUNS);
    println!("*First Token = total inference time (both models operate in offline batch mode).*");
    println!("*Peak RSS = max process resident set size observed during that model's benchmark runs.*");
}
