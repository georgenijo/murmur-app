//! End-of-utterance tail harness (#829), after Hex's release-boundary
//! harnesses (`dictation_tail_tests.rs`, `dictation_corpus_tests.rs`).
//!
//! The deterministic tests drive the host half of the stop path
//! (`read_stopping_frames`) with helper frames that are still in flight when
//! the stop lands, and assert the clip keeps every sample the helper captured.
//! The helper half (flush the ring before `Stopped`) is covered in the capture
//! worker crate.
//!
//! The ignored sweep replays synthetic `say` speech through Murmur's stop
//! timeline at 48 kHz: 512-frame callbacks, a stop that lands `release +
//! dispatch + tail` into the audio, and a drain that keeps every callback
//! completed before the stop. Each clip then runs through the production
//! preparation (exact 48k -> 16k resample, Silero VAD at the default
//! sensitivity) and an installed backend. It never reads Murmur history or the
//! personal corpus, and prints only synthetic phrases.

use super::*;
use murmur_capture_helper_protocol::ProductionPcm;
use std::sync::mpsc;

const RATE: u32 = 48_000;
const CALLBACK_FRAMES: usize = 512;

fn pcm(sequence: u64, samples: Vec<f32>) -> HelperRead {
    HelperRead::Frame(ProductionFrame::Pcm(ProductionPcm {
        channel: CaptureChannel::Microphone,
        sequence,
        sample_rate: RATE,
        captured_at_ns: 0,
        sample_offset: 0,
        samples,
    }))
}

fn control(message: ProductionHelperMessage) -> HelperRead {
    HelperRead::Frame(ProductionFrame::Control(message))
}

#[test]
fn stop_drain_keeps_in_flight_and_flushed_audio_through_stopped() {
    let (sender, receiver) = mpsc::channel();
    // Two callbacks were in the pipe when the stop landed, then the helper
    // flushed a partial ring and confirmed the full retained count.
    sender.send(pcm(5, vec![0.1; CALLBACK_FRAMES])).unwrap();
    sender.send(pcm(6, vec![0.2; CALLBACK_FRAMES])).unwrap();
    sender
        .send(control(ProductionHelperMessage::Phase {
            phase: CapturePhase::Active,
            backend: CaptureBackend::Auhal,
        }))
        .unwrap();
    sender.send(pcm(7, vec![0.3; 100])).unwrap();
    sender
        .send(control(ProductionHelperMessage::Stopped {
            retained_samples: 1_124,
        }))
        .unwrap();
    let mut sequence = 5;
    let mut clip = Vec::new();
    let outcome = read_stopping_frames(
        &receiver,
        Instant::now() + Duration::from_secs(1),
        &mut sequence,
        Some(RATE),
        &mut |samples: &[f32]| clip.extend_from_slice(samples),
    );
    assert_eq!(
        outcome,
        StopReadOutcome::Stopped {
            retained_samples: 1_124
        }
    );
    assert_eq!(sequence, 8);
    assert_eq!(clip.len(), 1_124);
    assert_eq!(clip.last(), Some(&0.3));
}

#[test]
fn release_tail_accepts_live_audio_until_its_deadline() {
    let (sender, receiver) = mpsc::channel();
    let producer = std::thread::spawn(move || {
        for sequence in 0..4 {
            if sender
                .send(pcm(sequence, vec![0.5; CALLBACK_FRAMES]))
                .is_err()
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        // Keep the pipe open past the tail like a live helper.
        std::thread::sleep(Duration::from_millis(200));
    });
    let mut sequence = 0;
    let mut clip = Vec::new();
    let started = Instant::now();
    let outcome = read_stopping_frames(
        &receiver,
        started + Duration::from_millis(80),
        &mut sequence,
        Some(RATE),
        &mut |samples: &[f32]| clip.extend_from_slice(samples),
    );
    assert_eq!(outcome, StopReadOutcome::Deadline);
    assert!(started.elapsed() >= Duration::from_millis(80));
    assert_eq!(clip.len(), 4 * CALLBACK_FRAMES);
    producer.join().unwrap();
}

#[test]
fn stop_drain_ends_without_dropping_retained_audio_on_bad_frames() {
    for terminal in [
        pcm(9, vec![0.4; 8]),
        control(ProductionHelperMessage::Failure {
            code: FailureCode::StreamError,
            backend: CaptureBackend::Auhal,
            retained_samples: 8,
        }),
        HelperRead::Invalid,
    ] {
        let (sender, receiver) = mpsc::channel();
        sender.send(pcm(1, vec![0.4; 8])).unwrap();
        sender.send(terminal).unwrap();
        let mut sequence = 1;
        let mut clip = Vec::new();
        let outcome = read_stopping_frames(
            &receiver,
            Instant::now() + Duration::from_secs(1),
            &mut sequence,
            Some(RATE),
            &mut |samples: &[f32]| clip.extend_from_slice(samples),
        );
        assert_eq!(outcome, StopReadOutcome::Ended);
        assert_eq!(clip.len(), 8);
    }

    // An expired deadline stops even with a backlog still queued.
    let (sender, receiver) = mpsc::channel();
    sender.send(pcm(0, vec![0.4; 8])).unwrap();
    let mut clip = Vec::new();
    let outcome = read_stopping_frames(
        &receiver,
        Instant::now() - Duration::from_millis(1),
        &mut 0,
        Some(RATE),
        &mut |samples: &[f32]| clip.extend_from_slice(samples),
    );
    assert_eq!(outcome, StopReadOutcome::Deadline);
    assert!(clip.is_empty());

    // A helper that never confirms is bounded by the drain deadline.
    let (_sender, receiver) = mpsc::channel();
    let started = Instant::now();
    let outcome = read_stopping_frames(
        &receiver,
        started + Duration::from_millis(30),
        &mut 0,
        None,
        &mut |_: &[f32]| {},
    );
    assert_eq!(outcome, StopReadOutcome::Deadline);
    assert!(started.elapsed() < Duration::from_millis(500));
}

#[test]
fn release_clip_ends_at_the_last_callback_before_the_stop() {
    let audio = 10 * CALLBACK_FRAMES + 7;
    // Stop inside the fourth callback: the drain keeps the three completed
    // callbacks plus the one in progress when the stream stopped.
    assert_eq!(
        drained_clip_len(audio, 3 * CALLBACK_FRAMES + 1),
        4 * CALLBACK_FRAMES
    );
    assert_eq!(
        drained_clip_len(audio, 4 * CALLBACK_FRAMES),
        4 * CALLBACK_FRAMES
    );
    assert_eq!(drained_clip_len(audio, audio + 10_000), audio);
    assert_eq!(drained_clip_len(audio, 0), 0);
}

/// Samples the drained stop path delivers when the helper stops `stop`
/// samples into the stream: every callback delivered before the stop, and
/// the callback in flight when Core Audio stopped the unit.
fn drained_clip_len(audio_len: usize, stop: usize) -> usize {
    stop.div_ceil(CALLBACK_FRAMES)
        .saturating_mul(CALLBACK_FRAMES)
        .min(audio_len)
}

const PHRASES: [&str; 6] = [
    "Please send me the latest report.",
    "We should ship this on Thursday.",
    "The meeting moved to next week.",
    "Add milk and eggs to the list.",
    "Can you check the logs first?",
    "That sounds great, thanks.",
];
const VOICES: [&str; 3] = ["Samantha", "Daniel", "Karen"];
const RELEASE_OFFSETS_MS: [i64; 11] = [-300, -250, -200, -150, -100, -50, 0, 50, 100, 150, 200];
const TAILS_MS: [u64; 8] = [0, 25, 50, 75, 100, 125, 150, 200];
const LEAD_MS: usize = 400;

fn samples_for_ms(ms: i64) -> i64 {
    ms * i64::from(RATE) / 1_000
}

/// End of the last 10 ms window louder than -45 dBFS (Hex's boundary).
fn speech_end(samples: &[f32]) -> usize {
    let window = RATE as usize / 100;
    samples
        .chunks(window)
        .enumerate()
        .filter(|(_, chunk)| {
            let energy = chunk.iter().map(|s| s * s).sum::<f32>() / chunk.len() as f32;
            10.0 * energy.max(1e-12).log10() > -45.0
        })
        .map(|(index, chunk)| index * window + chunk.len())
        .next_back()
        .unwrap_or(samples.len())
}

fn synthesize(dir: &std::path::Path, voice: &str, phrase: &str, index: usize) -> Vec<f32> {
    let path = dir.join(format!("phrase-{index}.wav"));
    let status = std::process::Command::new("say")
        .args(["-v", voice, "-o"])
        .arg(&path)
        .args(["--data-format=LEF32@48000", phrase])
        .status()
        .expect("say runs");
    assert!(status.success(), "say failed for voice {voice}");
    let mut reader = hound::WavReader::open(&path).unwrap();
    let speech: Vec<f32> = reader.samples::<f32>().map(Result::unwrap).collect();
    let mut audio = vec![0.0; RATE as usize * LEAD_MS / 1_000];
    audio.extend_from_slice(&speech);
    audio.resize(audio.len() + RATE as usize, 0.0);
    audio
}

fn normalized_words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|word| {
            word.chars()
                .filter(|c| c.is_alphanumeric())
                .collect::<String>()
                .to_lowercase()
        })
        .filter(|word| !word.is_empty())
        .collect()
}

fn transcribe(
    backend: &mut dyn crate::transcriber::TranscriptionBackend,
    vad_path: Option<&str>,
    clip: &[f32],
) -> String {
    let samples = resample(clip, RATE, 16_000);
    let samples = match vad_path {
        Some(path) => match crate::vad::filter_speech(
            path,
            &samples,
            crate::vad::threshold_for_sensitivity(50),
        ) {
            Ok(crate::vad::VadResult::Speech(trimmed)) => trimmed,
            Ok(crate::vad::VadResult::NoSpeech) => return String::new(),
            Err(_) => samples,
        },
        None => samples,
    };
    backend
        .transcribe(&samples, "en", None, true)
        .unwrap_or_default()
}

/// Opt-in: `MURMUR_TAIL_DISPATCH_MS` is the measured release-to-helper-stop
/// latency (default 0, the conservative case) and `MURMUR_TAIL_MODELS`
/// selects installed models (default: the Core ML Parakeet model).
#[test]
#[ignore = "synthesizes `say` speech and needs an installed transcription model"]
fn release_tail_sweep() {
    use crate::transcriber::TranscriptionBackend;
    let dispatch_ms: i64 = std::env::var("MURMUR_TAIL_DISPATCH_MS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let models = std::env::var("MURMUR_TAIL_MODELS")
        .unwrap_or_else(|_| crate::transcriber::COREML_MODEL_NAME.to_string());
    let vad_path = crate::vad::vad_model_path()
        .filter(|path| path.exists())
        .map(|path| path.to_string_lossy().to_string());
    let dir = std::env::temp_dir().join(format!("murmur-tail-sweep-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut clips = Vec::new();
    for voice in VOICES {
        for (index, phrase) in PHRASES.iter().enumerate() {
            clips.push((voice, *phrase, synthesize(&dir, voice, phrase, index)));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    println!(
        "dispatch_ms={dispatch_ms} vad={} clips={}",
        vad_path.is_some(),
        clips.len()
    );
    for model in models.split(',') {
        let mut backend: Box<dyn TranscriptionBackend> =
            if crate::transcriber::is_coreml_model(model) {
                Box::new(crate::transcriber::CoreMlBackend::new())
            } else if model.starts_with("parakeet") {
                Box::new(crate::transcriber::ParakeetBackend::new())
            } else {
                Box::new(crate::transcriber::WhisperBackend::new())
            };
        backend.load_model(model).expect("model is installed");
        // hits[tail][offset] = (last word recovered, final punctuation kept)
        let mut hits = vec![vec![(0_usize, 0_usize); RELEASE_OFFSETS_MS.len()]; TAILS_MS.len()];
        let mut reference_hits = 0;
        for (voice, phrase, audio) in &clips {
            let end = speech_end(audio);
            let expected_last = normalized_words(phrase).pop().unwrap();
            let full = transcribe(backend.as_mut(), vad_path.as_deref(), audio);
            let full_ok = normalized_words(&full).last() == Some(&expected_last);
            reference_hits += usize::from(full_ok);
            println!(
                "== {model} voice={voice} speech_end_ms={} full={full:?}",
                end * 1_000 / RATE as usize
            );
            for (tail_index, tail_ms) in TAILS_MS.iter().enumerate() {
                let mut row = String::new();
                for (offset_index, offset_ms) in RELEASE_OFFSETS_MS.iter().enumerate() {
                    let stop =
                        end as i64 + samples_for_ms(offset_ms + dispatch_ms + *tail_ms as i64);
                    let clip_len = drained_clip_len(audio.len(), stop.max(0) as usize);
                    let text =
                        transcribe(backend.as_mut(), vad_path.as_deref(), &audio[..clip_len]);
                    let words = normalized_words(&text);
                    let last_ok = words.last() == Some(&expected_last);
                    let punct_ok = last_ok && text.trim_end().ends_with(['.', '?', '!']);
                    hits[tail_index][offset_index].0 += usize::from(last_ok);
                    hits[tail_index][offset_index].1 += usize::from(punct_ok);
                    row.push(if punct_ok {
                        'P'
                    } else if last_ok {
                        'w'
                    } else {
                        '.'
                    });
                }
                println!("   tail={tail_ms:>3}ms {row}");
            }
        }
        println!(
            "## {model}: full-clip last word {reference_hits}/{}",
            clips.len()
        );
        println!(
            "## offsets_ms {}",
            RELEASE_OFFSETS_MS.map(|ms| format!("{ms:>5}")).join(" ")
        );
        for (tail_index, tail_ms) in TAILS_MS.iter().enumerate() {
            let words = hits[tail_index]
                .iter()
                .map(|(last, _)| format!("{last:>5}"))
                .collect::<Vec<_>>()
                .join(" ");
            let punct = hits[tail_index]
                .iter()
                .map(|(_, punct)| format!("{punct:>5}"))
                .collect::<Vec<_>>()
                .join(" ");
            println!("## tail={tail_ms:>3}ms last_word {words}");
            println!("##              punct     {punct}");
        }
    }
}
