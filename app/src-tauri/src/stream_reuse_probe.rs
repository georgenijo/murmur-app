//! Opt-in accuracy and timing probe for decode reuse (workstream E2). Not a
//! benchmark gate: it replays recorded 16 kHz WAV files through the same
//! incremental VAD, piece planner, tail decode and stitch the app uses, then
//! compares the stitched text with the whole-clip decode of the same kept
//! audio. Inputs are synthetic or repository fixtures, never user recordings.
//!
//! ```text
//! MURMUR_STREAM_PROBE_MANIFEST=/tmp/manifest.json   # [{"id": "...", "wav": "..."}]
//! MURMUR_STREAM_PROBE_OUTPUT=/tmp/out.json
//! MURMUR_STREAM_PROBE_VAD_MODEL=/tmp/silero_vad_v5.1.2.onnx   # pinned model file
//! MURMUR_STREAM_PROBE_BACKEND=parakeet|whisper
//! cargo test --lib -- --ignored --test-threads=1 stream_reuse_accuracy_probe
//! ```

use super::*;
use crate::transcriber::{self, TranscriptionBackend};
use crate::vad_stream::{IncrementalVad, OnnxScorer, SegmentParams};
use std::time::Instant;

const TICK: usize = 11_200; // The 700 ms preview tick.
const SENSITIVITY: u32 = 50;

fn words(text: &str) -> Vec<&str> {
    text.split_whitespace().collect()
}

fn normalized(word: &str) -> String {
    word.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Word-level differences between the whole-clip text and the stitched text:
/// `(position in the whole-clip words, kind, whole-clip word, stitched word)`.
/// `kind` is `punctuation_or_case` when only those differ, else `word`.
fn word_diffs(whole: &str, stitched: &str) -> Vec<serde_json::Value> {
    let a = words(whole);
    let b = words(stitched);
    let mut lcs = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut diffs = Vec::new();
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && a[i] == b[j] {
            i += 1;
            j += 1;
        } else if i < a.len()
            && j < b.len()
            && lcs[i + 1][j + 1] >= lcs[i + 1][j].max(lcs[i][j + 1])
        {
            let kind = if normalized(a[i]) == normalized(b[j]) {
                "punctuation_or_case"
            } else {
                "word"
            };
            diffs.push(serde_json::json!({"at": i, "kind": kind, "whole": a[i], "stitched": b[j]}));
            i += 1;
            j += 1;
        } else if j < b.len() && (i == a.len() || lcs[i][j + 1] >= lcs[i + 1][j]) {
            diffs.push(serde_json::json!({"at": i, "kind": "word", "whole": "", "stitched": b[j]}));
            j += 1;
        } else {
            diffs.push(serde_json::json!({"at": i, "kind": "word", "whole": a[i], "stitched": ""}));
            i += 1;
        }
    }
    diffs
}

fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

#[test]
#[ignore = "needs local models and a WAV manifest; see the module docs"]
fn stream_reuse_accuracy_probe() {
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(std::env::var("MURMUR_STREAM_PROBE_MANIFEST").expect("manifest")).unwrap(),
    )
    .unwrap();
    let output = std::env::var("MURMUR_STREAM_PROBE_OUTPUT").expect("output path");
    let vad_model = std::env::var("MURMUR_STREAM_PROBE_VAD_MODEL").expect("VAD model path");
    crate::vad_stream::load_session_from(std::path::Path::new(&vad_model)).unwrap();
    let backend_kind = std::env::var("MURMUR_STREAM_PROBE_BACKEND").unwrap_or("parakeet".into());
    let mut backend: Box<dyn TranscriptionBackend> = match backend_kind.as_str() {
        "whisper" => {
            let mut backend = transcriber::WhisperBackend::new();
            backend.load_model("tiny.en").unwrap();
            Box::new(backend)
        }
        _ => {
            let mut backend = transcriber::ParakeetBackend::new();
            backend.load_model("parakeet-tdt-0.6b-v2-fp16").unwrap();
            Box::new(backend)
        }
    };
    let params = SegmentParams::murmur(crate::vad::threshold_for_sensitivity(SENSITIVITY));
    let mut rows = Vec::new();
    let mut warmed = false;
    for fixture in manifest.as_array().unwrap() {
        let samples = transcriber::parse_wav_to_samples(
            &std::fs::read(fixture["wav"].as_str().unwrap()).unwrap(),
        )
        .unwrap();
        let decode = |backend: &mut dyn TranscriptionBackend, audio: &[f32]| {
            let start = Instant::now();
            let text = backend.transcribe(audio, "en", None, true).unwrap();
            (text, ms(start))
        };
        if !warmed {
            decode(backend.as_mut(), &samples[..samples.len().min(32_000)]);
            warmed = true;
        }

        // Recording: the VAD stream scores audio as it arrives; at each tick
        // the planner may commit a piece, decoded on the same backend.
        let mut vad = IncrementalVad::new(OnnxScorer::default());
        let mut reuse = StreamReuse::default();
        reuse.begin(1);
        let mut consumed = 0;
        let mut piece_decode_ms = Vec::new();
        for end in (TICK..samples.len()).step_by(TICK) {
            vad.push(&samples[consumed..end]).unwrap();
            consumed = end;
            if let Plan::Commit(plan) = reuse.plan(1, &vad.snapshot(params)) {
                let piece = plan.piece(&samples[plan.raw_start..end]).unwrap();
                let (text, took) = decode(backend.as_mut(), &piece);
                piece_decode_ms.push(took);
                reuse.commit(1, plan, piece, &text);
            }
        }

        // Stop: finish the VAD (unfinished tail only), then decode.
        let stop = Instant::now();
        let finished = vad.finish(&samples, params).expect("VAD finish");
        let vad_finish_ms = ms(stop);
        let crate::vad::VadResult::Speech(filtered) = finished.result else {
            rows.push(serde_json::json!({"id": fixture["id"], "skipped": "no_speech"}));
            continue;
        };
        let whole = decode(backend.as_mut(), &filtered);
        let plan = reuse.take_final(1, Some(&finished.bounds), &filtered);
        let (stitched, outcome, tail_ms) = match plan {
            Err(reason) => (None, reason.as_str(), 0.0),
            Ok(plan) => match plan.complete_text() {
                Some(text) => (Some(text), "reused_complete", 0.0),
                None => {
                    let (tail, took) = decode(backend.as_mut(), &filtered[plan.start..]);
                    match plan.stitch(&tail) {
                        Some(text) => (Some(text), "reused", took),
                        None => (None, "stitch_mismatch", took),
                    }
                }
            },
        };
        let diffs = stitched
            .as_deref()
            .map(|text| word_diffs(&whole.0, text))
            .unwrap_or_default();
        let punctuation_only = diffs
            .iter()
            .filter(|d| d["kind"] == "punctuation_or_case")
            .count();
        rows.push(serde_json::json!({
            "id": fixture["id"],
            "backend": backend_kind,
            "audio_s": samples.len() as f64 / 16_000.0,
            "kept_s": filtered.len() as f64 / 16_000.0,
            "outcome": outcome,
            "pieces": piece_decode_ms.len(),
            "identical": stitched.as_deref().map(|t| t == whole.0),
            "word_diffs": diffs.len(),
            "punctuation_or_case_diffs": punctuation_only,
            "diffs": diffs,
            "whole_clip_ms": whole.1,
            "stop_tail_ms": tail_ms,
            "piece_decode_ms": piece_decode_ms,
            "vad_stop_ms": vad_finish_ms,
            "tail_windows": finished.tail_windows,
        }));
    }
    std::fs::write(output, serde_json::to_vec_pretty(&rows).unwrap()).unwrap();
}
