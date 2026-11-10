use std::cell::RefCell;
use std::path::PathBuf;
use whisper_rs::{WhisperVadContext, WhisperVadContextParams, WhisperVadParams};

thread_local! {
    /// Whisper's VAD context is not Send/Sync, so keep one cache per blocking
    /// worker. Tokio normally reuses those workers; a different worker simply
    /// pays the small initialization cost once for its own context.
    static VAD_CONTEXT: RefCell<Option<(String, WhisperVadContext)>> = const { RefCell::new(None) };
}

pub const VAD_MODEL_FILENAME: &str = "ggml-silero-v5.1.2.bin";
pub const VAD_MODEL_URL: &str =
    "https://huggingface.co/ggml-org/whisper-vad/resolve/main/ggml-silero-v5.1.2.bin";
const SPEECH_PAD_MS: i32 = 120;

/// Expected path for the VAD model under the app's models directory.
pub fn vad_model_path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| {
        d.join("local-dictation")
            .join("models")
            .join(VAD_MODEL_FILENAME)
    })
}

/// Check whether the VAD model file exists on disk.
pub fn vad_model_exists() -> bool {
    vad_model_path().is_some_and(|path| {
        crate::model_artifact::binary_model_is_valid(
            &path,
            crate::model_artifact::MIN_VAD_MODEL_BYTES,
        )
    })
}

pub enum VadResult {
    /// No speech detected in the audio.
    NoSpeech,
    /// Speech detected — contains trimmed samples (speech segments only).
    Speech(Vec<f32>),
}

/// Zero is the explicit lowest-latency setting; non-zero values run VAD.
pub fn is_enabled(sensitivity: u32) -> bool {
    sensitivity > 0
}

/// Map the user-facing sensitivity scale to whisper.cpp's VAD threshold.
/// Higher sensitivity keeps more audio, so it uses a lower threshold.
pub fn threshold_for_sensitivity(sensitivity: u32) -> f32 {
    1.0 - (sensitivity.clamp(0, 100) as f32 / 100.0)
}

/// Sample range kept by the VAD (start inclusive, end exclusive) in the
/// original clip. Includes the [`SPEECH_PAD_MS`] padding on both sides.
pub type SpeechRange = std::ops::Range<usize>;

/// A filter result together with the clip ranges it was built from.
pub struct DetailedVad {
    pub result: VadResult,
    /// Non-empty kept ranges in segment order, clamped to the clip. These are
    /// exactly the slices concatenated into `result`.
    pub ranges: Vec<SpeechRange>,
}

/// Run Silero VAD on the given 16kHz mono samples and return only speech segments.
///
/// `model_path` must point to a valid Silero VAD GGML model file.
/// This function creates a `WhisperVadContext` which is `!Send`, so it must
/// run entirely within a single thread (use `spawn_blocking`).
pub fn filter_speech(
    model_path: &str,
    samples: &[f32],
    threshold: f32,
) -> Result<VadResult, String> {
    filter_speech_detailed(model_path, samples, threshold).map(|detailed| detailed.result)
}

/// [`filter_speech`] that also exposes the kept segment bounds. The filtered
/// output is built by the same code, so it is identical for every caller.
pub fn filter_speech_detailed(
    model_path: &str,
    samples: &[f32],
    threshold: f32,
) -> Result<DetailedVad, String> {
    VAD_CONTEXT.with(|cache| {
        let mut cached = cache.borrow_mut();
        if cached
            .as_ref()
            .is_none_or(|(cached_path, _)| cached_path != model_path)
        {
            let mut ctx_params = WhisperVadContextParams::default();
            ctx_params.set_n_threads(1);
            ctx_params.set_use_gpu(false);
            let context = WhisperVadContext::new(model_path, ctx_params)
                .map_err(|e| format!("Failed to create VAD context: {}", e))?;
            *cached = Some((model_path.to_string(), context));
        }

        let context = &mut cached
            .as_mut()
            .expect("VAD context was initialized above")
            .1;
        filter_speech_with_context(context, samples, threshold)
    })
}

fn filter_speech_with_context(
    ctx: &mut WhisperVadContext,
    samples: &[f32],
    threshold: f32,
) -> Result<DetailedVad, String> {
    let mut vad_params = WhisperVadParams::default();
    vad_params.set_threshold(threshold);
    // Preserve the initial consonant around VAD boundaries. The upstream
    // default is only 30 ms, which is easy to clip when speech begins as the
    // hotkey is pressed.
    vad_params.set_speech_pad(SPEECH_PAD_MS);

    let segments = ctx
        .segments_from_samples(vad_params, samples)
        .map_err(|e| format!("VAD inference failed: {}", e))?;

    let num_segments = segments.num_segments();
    if num_segments == 0 {
        return Ok(DetailedVad {
            result: VadResult::NoSpeech,
            ranges: Vec::new(),
        });
    }

    let sample_rate = 16_000.0_f32;
    let total_samples = samples.len();
    let mut ranges = Vec::new();

    for seg in segments {
        // Timestamps are in centiseconds (cs); convert to sample indices
        let start_idx = ((seg.start / 100.0) * sample_rate) as usize;
        let end_idx = ((seg.end / 100.0) * sample_rate).ceil() as usize;

        let start_idx = start_idx.min(total_samples);
        let end_idx = end_idx.min(total_samples);

        if start_idx < end_idx {
            ranges.push(start_idx..end_idx);
        }
    }

    Ok(DetailedVad {
        result: concatenate_ranges(samples, &ranges),
        ranges,
    })
}

/// Concatenate the kept ranges in order. No ranges means no speech.
fn concatenate_ranges(samples: &[f32], ranges: &[SpeechRange]) -> VadResult {
    let mut speech_samples = Vec::new();
    for range in ranges {
        speech_samples.extend_from_slice(&samples[range.clone()]);
    }
    if speech_samples.is_empty() {
        VadResult::NoSpeech
    } else {
        VadResult::Speech(speech_samples)
    }
}

/// Samples per millisecond of the 16 kHz mono clips the VAD consumes.
pub const SAMPLES_PER_MS: usize = 16;

/// Shortest pause the early-input shadow measurement treats as a pause.
pub const SHADOW_MIN_PAUSE_MS: usize = 300;

/// Silence, in samples, between the last speech at or before `position` and
/// `position`. Zero when a segment covers `position`; `position` itself when no
/// speech precedes it. `ranges` are ordered, as returned by the VAD.
pub fn trailing_silence_samples(ranges: &[SpeechRange], position: usize) -> usize {
    ranges
        .iter()
        .filter(|range| range.start < position)
        .map(|range| range.end.min(position))
        .max()
        .map_or(position, |last_end| position - last_end)
}

/// End of the last kept range, if any speech was kept.
pub fn last_speech_end(ranges: &[SpeechRange]) -> Option<usize> {
    ranges.iter().map(|range| range.end).max()
}

/// Where a trigger waiting for a pause of at least `min_pause` samples would
/// have fired on its latest qualifying pause: `min_pause` after the pause
/// began. The pause is the gap between consecutive kept ranges, or the gap
/// between the last range and the end of the clip. `None` when no gap
/// qualifies. A gap between ranges that qualifies yields a cut that is
/// followed by more speech, so the audio up to it is a strict prefix.
pub fn last_pause_cut(ranges: &[SpeechRange], clip_len: usize, min_pause: usize) -> Option<usize> {
    let mut cut = None;
    for window in ranges.windows(2) {
        if window[1].start.saturating_sub(window[0].end) >= min_pause {
            cut = Some(window[0].end + min_pause);
        }
    }
    if let Some(last) = ranges.last() {
        if clip_len.saturating_sub(last.end) >= min_pause {
            cut = Some(last.end + min_pause);
        }
    }
    cut
}

/// Whether two filter outputs are byte-identical: the same kind of result and,
/// for speech, the same samples bit for bit. Two "no speech" results match.
pub fn outputs_identical(a: &VadResult, b: &VadResult) -> bool {
    match (a, b) {
        (VadResult::NoSpeech, VadResult::NoSpeech) => true,
        (VadResult::Speech(a), VadResult::Speech(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.to_bits() == b.to_bits())
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Public synthetic speech only; never accepts a private corpus path.
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    #[test]
    #[ignore = "loads installed Core ML and VAD models; synthetic fixture diagnostic"]
    fn synthetic_attenuated_speech_and_silence_diagnostic() {
        use crate::transcriber::{
            parse_wav_to_samples, CoreMlBackend, TranscriptionBackend, COREML_MODEL_NAME,
        };
        let path = vad_model_path()
            .filter(|path| path.exists())
            .expect("installed VAD required");
        let fixture =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bench/audio/short.wav");
        let clean = parse_wav_to_samples(&std::fs::read(fixture).unwrap()).unwrap();
        assert!(!clean.is_empty());
        let mut backend = CoreMlBackend::new();
        backend
            .load_model(COREML_MODEL_NAME)
            .expect("installed Core ML required");
        for gain_db in [0, -12, -24, -30, -36, -48, -96] {
            let gain = if gain_db == -96 {
                0.0
            } else {
                10.0f32.powf(gain_db as f32 / 20.0)
            };
            let mut samples: Vec<f32> = clean.iter().map(|sample| sample * gain).collect();
            samples.resize(samples.len() + 24_000, 0.0); // 1.5 seconds trailing silence.
            let mut nonempty_previews = 0;
            for end in (12_800..samples.len()).step_by(11_200) {
                let window = &samples[end.saturating_sub(320_000)..end];
                let preview = backend
                    .transcribe_preview(
                        window,
                        "auto",
                        None,
                        true,
                        crate::transcriber::PreviewPass::SyntheticDiagnostic,
                    )
                    .unwrap();
                nonempty_previews += usize::from(!preview.trim().is_empty());
            }
            let (vad_speech, kept_samples, final_nonempty) = match filter_speech(
                path.to_str().unwrap(),
                &samples,
                threshold_for_sensitivity(50),
            )
            .unwrap()
            {
                VadResult::NoSpeech => (false, 0, false),
                VadResult::Speech(kept) => {
                    let final_text = backend.transcribe(&kept, "auto", None, true).unwrap();
                    (true, kept.len(), !final_text.trim().is_empty())
                }
            };
            println!(
                "{}",
                serde_json::json!({
                    "fixture": if gain == 0.0 { "synthetic_silence" } else { "synthetic_short" },
                    "gain_db": gain_db, "input_samples": samples.len(), "kept_samples": kept_samples,
                    "rms": crate::audio::compute_rms(&samples), "peak": crate::audio::compute_peak(&samples),
                    "nonempty_previews": nonempty_previews, "vad_speech": vad_speech,
                    "final_nonempty": final_nonempty, "disagreement": nonempty_previews > 0 && !vad_speech,
                })
            );
            if gain_db == 0 {
                assert!(nonempty_previews > 0 && vad_speech && final_nonempty);
            }
            if gain == 0.0 {
                assert!(!vad_speech);
            }
        }
    }

    #[test]
    fn zero_sensitivity_disables_vad() {
        assert!(!is_enabled(0));
        assert!(is_enabled(1));
        assert!(is_enabled(100));
    }

    #[test]
    fn sensitivity_threshold_mapping_is_clamped_and_shared() {
        assert_eq!(threshold_for_sensitivity(0), 1.0);
        assert_eq!(threshold_for_sensitivity(50), 0.5);
        assert_eq!(threshold_for_sensitivity(100), 0.0);
        assert_eq!(threshold_for_sensitivity(150), 0.0);
    }

    #[test]
    fn concatenated_ranges_are_the_filtered_output() {
        let samples: Vec<f32> = (0..20).map(|i| i as f32).collect();
        let VadResult::Speech(kept) = concatenate_ranges(&samples, &[2..4, 10..13]) else {
            panic!("ranges keep speech");
        };
        assert_eq!(kept, vec![2.0, 3.0, 10.0, 11.0, 12.0]);
        assert!(matches!(
            concatenate_ranges(&samples, &[]),
            VadResult::NoSpeech
        ));
    }

    #[test]
    fn trailing_silence_is_measured_from_the_last_speech_before_a_position() {
        let ranges = [100..400, 1_000..1_600];
        // Between segments: 600 samples after the first one.
        assert_eq!(trailing_silence_samples(&ranges, 1_000), 600);
        // A position inside a segment has no trailing silence.
        assert_eq!(trailing_silence_samples(&ranges, 1_200), 0);
        // After the final segment, up to the end of a clip.
        assert_eq!(trailing_silence_samples(&ranges, 2_000), 400);
        // Before any speech the whole prefix is silence.
        assert_eq!(trailing_silence_samples(&ranges, 50), 50);
        assert_eq!(trailing_silence_samples(&[], 800), 800);
        assert_eq!(last_speech_end(&ranges), Some(1_600));
        assert_eq!(last_speech_end(&[]), None);
    }

    // A single speech segment is the input here, not a mistyped `[0; 16_000]`.
    #[allow(clippy::single_range_in_vec_init)]
    #[test]
    fn trailing_silence_in_milliseconds_uses_the_sixteen_khz_clock() {
        let ranges = [0..16_000];
        let silence = trailing_silence_samples(&ranges, 16_000 + 4_800);
        assert_eq!(silence / SAMPLES_PER_MS, 300);
    }

    // A single speech segment is the input here, not a mistyped `[0; 1_000]`.
    #[allow(clippy::single_range_in_vec_init)]
    #[test]
    fn last_pause_cut_fires_a_minimum_pause_after_the_pause_began() {
        let pause = SHADOW_MIN_PAUSE_MS * SAMPLES_PER_MS;
        // Trailing silence of exactly the minimum pause qualifies and the cut
        // is the end of the clip.
        let ranges = [0..1_000];
        assert_eq!(
            last_pause_cut(&ranges, 1_000 + pause, pause),
            Some(1_000 + pause)
        );
        // Longer trailing silence cuts earlier than the clip end.
        assert_eq!(
            last_pause_cut(&ranges, 1_000 + 3 * pause, pause),
            Some(1_000 + pause)
        );
        // Below the minimum there is no pause.
        assert_eq!(last_pause_cut(&ranges, 1_000 + pause - 1, pause), None);
        assert_eq!(last_pause_cut(&[], 10 * pause, pause), None);
    }

    #[test]
    fn last_pause_cut_prefers_the_latest_qualifying_pause() {
        let pause = 300 * SAMPLES_PER_MS;
        let ranges = [
            0..1_000,
            1_000 + 2 * pause..2_000 + 2 * pause,
            10_000 + 2 * pause..11_000 + 2 * pause,
        ];
        // Both interior gaps qualify; the later one wins, and speech follows it.
        let clip_len = 11_000 + 2 * pause + 100;
        assert_eq!(
            last_pause_cut(&ranges, clip_len, pause),
            Some(2_000 + 2 * pause + pause)
        );
        // A qualifying trailing pause is later than any interior one.
        assert_eq!(
            last_pause_cut(&ranges, 11_000 + 2 * pause + pause, pause),
            Some(11_000 + 2 * pause + pause)
        );
    }

    #[test]
    fn identical_outputs_compare_bit_for_bit() {
        let speech = |samples: &[f32]| VadResult::Speech(samples.to_vec());
        assert!(outputs_identical(
            &VadResult::NoSpeech,
            &VadResult::NoSpeech
        ));
        assert!(outputs_identical(
            &speech(&[0.0, 0.5, -0.25]),
            &speech(&[0.0, 0.5, -0.25])
        ));
        // Different length, different value, and sign of zero all differ.
        assert!(!outputs_identical(
            &speech(&[0.0, 0.5]),
            &speech(&[0.0, 0.5, 0.5])
        ));
        assert!(!outputs_identical(
            &speech(&[0.0, 0.5]),
            &speech(&[0.0, 0.25])
        ));
        assert!(!outputs_identical(&speech(&[0.0]), &speech(&[-0.0])));
        assert!(!outputs_identical(&VadResult::NoSpeech, &speech(&[0.0])));
        assert!(!outputs_identical(&speech(&[0.0]), &VadResult::NoSpeech));
    }

    #[test]
    fn missing_model_returns_an_error_without_populating_cache() {
        let missing = std::env::temp_dir().join("murmur-missing-vad-model.bin");
        let missing = missing.to_string_lossy().into_owned();
        let result = filter_speech(&missing, &[0.0; 16_000], 0.5);
        assert!(result.is_err());
        VAD_CONTEXT.with(|cache| {
            assert!(
                cache
                    .borrow()
                    .as_ref()
                    .is_none_or(|(path, _)| path != &missing),
                "a failed context must not be cached"
            );
        });
    }

    #[test]
    fn installed_model_context_is_reused_on_the_same_worker() {
        let Some(path) = vad_model_path().filter(|path| path.exists()) else {
            return;
        };
        let path = path.to_string_lossy().into_owned();
        let samples = vec![0.0; 16_000];
        filter_speech(&path, &samples, 0.5).expect("first VAD pass");
        let first_context = VAD_CONTEXT.with(|cache| {
            let cache = cache.borrow();
            let (_, context) = cache.as_ref().expect("context should be cached");
            context as *const WhisperVadContext as usize
        });
        filter_speech(&path, &samples, 0.5).expect("second VAD pass");
        let second_context = VAD_CONTEXT.with(|cache| {
            let cache = cache.borrow();
            let (_, context) = cache.as_ref().expect("context should be cached");
            context as *const WhisperVadContext as usize
        });
        assert_eq!(first_context, second_context);
    }
}
