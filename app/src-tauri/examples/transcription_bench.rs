//! Comparable local benchmark for Murmur transcription backends.
//!
//! Usage:
//!   cargo run --release --example transcription_bench -- \
//!     --engine coreml|parakeet|whisper [--iterations 5] [--audio-dir PATH]

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::Instant;
use ui_lib::transcriber::{
    parse_wav_to_samples, ParakeetBackend, TranscriptionBackend, WhisperBackend,
    WHISPER_SAMPLE_RATE,
};
#[cfg(target_os = "macos")]
use ui_lib::transcriber::{CoreMlBackend, COREML_MODEL_NAME};

const DEFAULT_AUDIO_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../bench/audio");
const PARAKEET_MODEL: &str = "parakeet-tdt-0.6b-v2-fp16";
const WHISPER_MODEL: &str = "base.en";

struct Args {
    engine: String,
    iterations: usize,
    audio_dir: PathBuf,
    model: Option<String>,
}

#[derive(Serialize)]
struct ResultRow {
    fixture: String,
    audio_seconds: f64,
    model_load_ms: f64,
    first_inference_ms: f64,
    warm_median_ms: f64,
    warm_min_ms: f64,
    warm_max_ms: f64,
    realtime_factor: f64,
    word_error_rate: Option<f64>,
    transcript: String,
}

#[derive(Serialize)]
struct Report {
    engine: String,
    model: String,
    iterations: usize,
    results: Vec<ResultRow>,
}

fn parse_args() -> Result<Args, String> {
    let mut engine = None;
    let mut iterations = 5;
    let mut audio_dir = PathBuf::from(DEFAULT_AUDIO_DIR);
    let mut model = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--engine" => engine = args.next(),
            "--iterations" => {
                iterations = args
                    .next()
                    .ok_or("--iterations requires a value")?
                    .parse()
                    .map_err(|_| "--iterations must be a positive integer")?;
            }
            "--audio-dir" => {
                audio_dir = PathBuf::from(args.next().ok_or("--audio-dir requires a path")?)
            }
            "--model" => model = args.next(),
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    if iterations == 0 {
        return Err("--iterations must be greater than zero".to_string());
    }
    Ok(Args {
        engine: engine.ok_or("--engine is required (coreml|parakeet|whisper)")?,
        iterations,
        audio_dir,
        model,
    })
}

fn backend(engine: &str) -> Result<(Box<dyn TranscriptionBackend>, &'static str), String> {
    match engine {
        #[cfg(target_os = "macos")]
        "coreml" => Ok((Box::new(CoreMlBackend::new()), COREML_MODEL_NAME)),
        "parakeet" => Ok((Box::new(ParakeetBackend::new()), PARAKEET_MODEL)),
        "whisper" => Ok((Box::new(WhisperBackend::new()), WHISPER_MODEL)),
        #[cfg(not(target_os = "macos"))]
        "coreml" => Err("Core ML is available only on macOS".to_string()),
        _ => Err(format!("unknown engine: {engine}")),
    }
}

fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|character: char| !character.is_alphanumeric() && character != '\'')
        .filter(|word| !word.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn word_error_rate(reference: &str, hypothesis: &str) -> Option<f64> {
    let reference = words(reference);
    if reference.is_empty() {
        return None;
    }
    let hypothesis = words(hypothesis);
    let mut previous: Vec<usize> = (0..=hypothesis.len()).collect();
    for (row, reference_word) in reference.iter().enumerate() {
        let mut current = vec![row + 1; hypothesis.len() + 1];
        for (column, hypothesis_word) in hypothesis.iter().enumerate() {
            current[column + 1] = (previous[column + 1] + 1)
                .min(current[column] + 1)
                .min(previous[column] + usize::from(reference_word != hypothesis_word));
        }
        previous = current;
    }
    Some(previous[hypothesis.len()] as f64 / reference.len() as f64)
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    if values.len() % 2 == 0 {
        (values[middle - 1] + values[middle]) / 2.0
    } else {
        values[middle]
    }
}

fn fixtures(directory: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = std::fs::read_dir(directory)
        .map_err(|error| format!("could not read {}: {error}", directory.display()))?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("wav"))
        .collect::<Vec<_>>();
    files.sort();
    if files.is_empty() {
        return Err(format!(
            "no WAV fixtures in {}; run bench/make_audio.sh first",
            directory.display()
        ));
    }
    Ok(files)
}

fn main() -> Result<(), String> {
    let args = parse_args()?;
    let (mut backend, default_model) = backend(&args.engine)?;
    let model = args.model.as_deref().unwrap_or(default_model).to_string();
    let mut results = Vec::new();

    for wav_path in fixtures(&args.audio_dir)? {
        let bytes = std::fs::read(&wav_path)
            .map_err(|error| format!("could not read {}: {error}", wav_path.display()))?;
        let samples = parse_wav_to_samples(&bytes)?;
        let audio_seconds = samples.len() as f64 / WHISPER_SAMPLE_RATE as f64;
        let fixture = wav_path
            .file_stem()
            .and_then(|value| value.to_str())
            .ok_or("fixture name is not UTF-8")?
            .to_string();
        let reference = std::fs::read_to_string(wav_path.with_extension("txt")).ok();

        backend.reset();
        let load_started = Instant::now();
        backend.load_model(&model)?;
        let model_load_ms = load_started.elapsed().as_secs_f64() * 1000.0;

        let first_started = Instant::now();
        let transcript = backend.transcribe(&samples, "en", None, true)?;
        let first_inference_ms = first_started.elapsed().as_secs_f64() * 1000.0;

        let mut warm = Vec::with_capacity(args.iterations);
        for _ in 0..args.iterations {
            let started = Instant::now();
            backend.transcribe(&samples, "en", None, true)?;
            warm.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        let warm_min_ms = warm.iter().copied().fold(f64::INFINITY, f64::min);
        let warm_max_ms = warm.iter().copied().fold(0.0, f64::max);
        let warm_median_ms = median(&mut warm);
        let wer = reference
            .as_deref()
            .and_then(|reference| word_error_rate(reference, &transcript));

        println!(
            "{fixture}: audio={audio_seconds:.2}s load={model_load_ms:.1}ms first={first_inference_ms:.1}ms warm_median={warm_median_ms:.1}ms WER={}",
            wer.map(|value| format!("{:.1}%", value * 100.0))
                .unwrap_or_else(|| "n/a".to_string())
        );
        println!("  {transcript}");
        results.push(ResultRow {
            fixture,
            audio_seconds,
            model_load_ms,
            first_inference_ms,
            warm_median_ms,
            warm_min_ms,
            warm_max_ms,
            realtime_factor: warm_median_ms / 1000.0 / audio_seconds,
            word_error_rate: wer,
            transcript,
        });
    }

    println!(
        "BENCH_JSON:{}",
        serde_json::to_string(&Report {
            engine: args.engine,
            model,
            iterations: args.iterations,
            results,
        })
        .map_err(|error| error.to_string())?
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_error_rate_is_order_sensitive() {
        assert_eq!(word_error_rate("one two three", "one two three"), Some(0.0));
        assert_eq!(
            word_error_rate("one two three", "one three two"),
            Some(2.0 / 3.0)
        );
    }

    #[test]
    fn median_handles_even_and_odd_inputs() {
        assert_eq!(median(&mut [3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&mut [4.0, 1.0, 3.0, 2.0]), 2.5);
    }
}
