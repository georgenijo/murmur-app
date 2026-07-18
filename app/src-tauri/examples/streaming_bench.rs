//! Reproducible stop-latency comparison for Whisper's batch and incremental
//! paths. The fixture is already 16 kHz mono; this isolates Whisper inference
//! and deterministic overlap reconciliation from capture/VAD overhead.

use serde::Serialize;
use std::path::PathBuf;
use std::time::Instant;
use ui_lib::transcriber::chunking::{
    merge_overlapping_text, OVERLAP_SAMPLES, STEP_SAMPLES, WINDOW_SAMPLES,
};
use ui_lib::transcriber::{
    parse_wav_to_samples, TranscriptionBackend, WhisperBackend, WHISPER_SAMPLE_RATE,
};

#[derive(Serialize)]
struct Report {
    fixture: String,
    audio_seconds: f64,
    model: String,
    during_recording_chunks: usize,
    during_recording_inference_ms: f64,
    batch_post_stop_ms: f64,
    incremental_post_stop_ms: f64,
    post_stop_speedup: f64,
    batch_wer: Option<f64>,
    incremental_wer: Option<f64>,
    incremental_vs_batch_wer: Option<f64>,
    batch_text: String,
    incremental_text: String,
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

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let fixture = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("../../bench/audio/xlong.wav"));
    let model = args.next().unwrap_or_else(|| "base.en".to_string());
    let samples = parse_wav_to_samples(
        &std::fs::read(&fixture)
            .map_err(|error| format!("could not read {}: {error}", fixture.display()))?,
    )?;
    if samples.len() < WINDOW_SAMPLES {
        return Err("fixture must be at least 10 seconds".to_string());
    }
    let reference = std::fs::read_to_string(fixture.with_extension("txt")).ok();

    let mut backend = WhisperBackend::new();
    backend.load_model(&model)?;
    // Warm Metal allocations before either measured path.
    backend.transcribe(&samples[..WINDOW_SAMPLES], "en", None, true)?;

    let batch_started = Instant::now();
    let batch_text = backend.transcribe(&samples, "en", None, true)?;
    let batch_post_stop_ms = batch_started.elapsed().as_secs_f64() * 1000.0;

    let mut incremental_text = String::new();
    let mut during_recording_inference_ms = 0.0;
    let mut processed_end = 0;
    let mut chunk_count = 0;
    let mut next_end = WINDOW_SAMPLES;
    while next_end <= samples.len() {
        let window = &samples[next_end - WINDOW_SAMPLES..next_end];
        let started = Instant::now();
        let chunk = backend.transcribe(window, "en", None, true)?;
        println!("during-recording chunk {}: {}", chunk_count + 1, chunk);
        during_recording_inference_ms += started.elapsed().as_secs_f64() * 1000.0;
        incremental_text = merge_overlapping_text(&incremental_text, &chunk);
        processed_end = next_end;
        chunk_count += 1;
        next_end = next_end.saturating_add(STEP_SAMPLES);
    }

    let tail_start = processed_end.saturating_sub(OVERLAP_SAMPLES);
    let final_started = Instant::now();
    let final_text = backend.transcribe(&samples[tail_start..], "en", None, true)?;
    println!("post-stop final tail: {final_text}");
    let incremental_post_stop_ms = final_started.elapsed().as_secs_f64() * 1000.0;
    incremental_text = merge_overlapping_text(&incremental_text, &final_text);

    let report = Report {
        fixture: fixture.display().to_string(),
        audio_seconds: samples.len() as f64 / WHISPER_SAMPLE_RATE as f64,
        model,
        during_recording_chunks: chunk_count,
        during_recording_inference_ms,
        batch_post_stop_ms,
        incremental_post_stop_ms,
        post_stop_speedup: batch_post_stop_ms / incremental_post_stop_ms,
        batch_wer: reference
            .as_deref()
            .and_then(|text| word_error_rate(text, &batch_text)),
        incremental_wer: reference
            .as_deref()
            .and_then(|text| word_error_rate(text, &incremental_text)),
        incremental_vs_batch_wer: word_error_rate(&batch_text, &incremental_text),
        batch_text,
        incremental_text,
    };
    println!(
        "batch post-stop={:.1}ms; incremental final-tail={:.1}ms; speedup={:.2}x; chunks during recording={}",
        report.batch_post_stop_ms,
        report.incremental_post_stop_ms,
        report.post_stop_speedup,
        report.during_recording_chunks,
    );
    println!(
        "batch WER={}; incremental WER={}; incremental-vs-batch WER={}",
        report
            .batch_wer
            .map(|value| format!("{:.1}%", value * 100.0))
            .unwrap_or_else(|| "n/a".to_string()),
        report
            .incremental_wer
            .map(|value| format!("{:.1}%", value * 100.0))
            .unwrap_or_else(|| "n/a".to_string()),
        report
            .incremental_vs_batch_wer
            .map(|value| format!("{:.1}%", value * 100.0))
            .unwrap_or_else(|| "n/a".to_string()),
    );
    println!(
        "STREAMING_BENCH_JSON:{}",
        serde_json::to_string(&report).map_err(|error| error.to_string())?
    );
    Ok(())
}
