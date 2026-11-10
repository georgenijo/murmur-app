//! Content-free shadow measurement for the early-final decision (#1081).
//!
//! The data that decides whether an early final decode is worth building, and
//! which trigger it should use, is recorded per dictation as numbers and
//! booleans only. Nothing here reads, stores, logs or emits transcript, audio,
//! clipboard or selection content: audio is only ever compared in memory, and
//! the stored result of every comparison is a boolean.
//!
//! The measurement is computed after the paste, off the critical path, and
//! amends the delivered run. A run without a `shadow` companion was either
//! recorded before this field existed or its follow-up work never finished;
//! both mean "not measured", never "zero".
use super::types::{MeasurementV1, UnavailableReasonV1};
use crate::vad::{self, SpeechRange, VadResult, SAMPLES_PER_MS, SHADOW_MIN_PAUSE_MS};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;

pub(crate) const SHADOW_SCHEMA_VERSION: u32 = 1;

/// Unfinished or not-yet-delivered measurements kept in memory at once.
const MAX_TRACKED_RUNS: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShadowMeasurementV1 {
    #[serde(deserialize_with = "version_one")]
    pub schema_version: u32,
    /// Silence the full-clip VAD saw between its last speech and the stop-key
    /// press. `NotApplicable` without a native stop-key gesture.
    pub trailing_silence_before_press_ms: MeasurementV1<u64>,
    /// Silence between the last VAD speech and the end of the clip, which ends
    /// at the stop.
    pub trailing_silence_before_stop_ms: MeasurementV1<u64>,
    /// Whether the VAD output for the audio up to the press is byte-identical
    /// to the full-clip filtered output the final decode consumed.
    pub press_prefix_matches_final: MeasurementV1<bool>,
    /// The same comparison for the audio up to the last pause of at least
    /// 300 ms. `NotApplicable` when the clip has no such pause.
    pub pause_prefix_matches_final: MeasurementV1<bool>,
    /// True when no live preview held the transcription backend at the press.
    pub model_free_at_press: MeasurementV1<bool>,
    /// Whether a second decode of the same filtered audio produced the same
    /// text. `NotApplicable` while the diagnostics switch is off.
    pub decode_deterministic: MeasurementV1<bool>,
}

fn version_one<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
    let version = u32::deserialize(deserializer)?;
    if version == SHADOW_SCHEMA_VERSION {
        Ok(version)
    } else {
        Err(serde::de::Error::custom("unsupported shadow schema"))
    }
}

/// Environment switch for the determinism re-decode. Off unless it is exactly
/// `1`: a second decode costs real inference time and, while it runs, holds
/// the backend the next dictation needs.
pub(crate) const DECODE_DETERMINISM_ENV: &str = "MURMUR_SHADOW_DECODE_DETERMINISM";

pub(crate) fn decode_determinism_enabled() -> bool {
    decode_determinism_switch(std::env::var(DECODE_DETERMINISM_ENV).ok().as_deref())
}

fn decode_determinism_switch(value: Option<&str>) -> bool {
    value == Some("1")
}

/// SHA-256 of a decoded text. It exists only in memory, for the length of the
/// follow-up that compares it: it is never stored, logged or emitted, and the
/// only thing that leaves the comparison is a boolean.
pub(crate) type TextDigest = [u8; 32];

pub(crate) fn text_digest(text: &str) -> TextDigest {
    use sha2::{Digest, Sha256};
    let mut digest = [0u8; 32];
    digest.copy_from_slice(&Sha256::digest(text.as_bytes()));
    digest
}

/// Whether the first decode's text should be digested for the determinism
/// comparison. Only the Core ML backend is re-decoded: the authoritative
/// Whisper state is reused across decodes and carries decoder history, so a
/// second pass would neither be independent of the first nor leave the state
/// the next dictation decodes from untouched.
pub(crate) fn should_digest_first_decode(model_name: &str) -> bool {
    digest_wanted(decode_determinism_enabled(), model_name)
}

fn digest_wanted(switch_on: bool, model_name: &str) -> bool {
    switch_on && crate::transcriber::is_coreml_model(model_name)
}

/// Fold a re-decode attempt into its measurement, dropping the text with it.
/// `Ok(None)` means the backend was not immediately available, which is the
/// reason the pass is skipped rather than queued.
pub(crate) fn decode_determinism(
    first: &TextDigest,
    redecoded: Result<Option<String>, String>,
) -> MeasurementV1<bool> {
    match redecoded {
        Ok(Some(text)) => MeasurementV1::measured(text_digest(&text) == *first),
        Ok(None) => MeasurementV1::Unavailable {
            reason: UnavailableReasonV1::DependencyPending,
        },
        // The error text is dropped unread: it is not diagnostics data.
        Err(_) => MeasurementV1::Unavailable {
            reason: UnavailableReasonV1::SampleFailed,
        },
    }
}

/// Whether the transcription backend was free of live-preview work when the
/// stop key went down. Only the Core ML preview decodes on the backend the
/// final decode uses; the Whisper preview has its own recording-local context.
/// `None` is a gesture whose preview state could not be read.
pub(crate) fn model_free_at_press(
    model_name: &str,
    preview_in_flight_at_press: Option<bool>,
) -> MeasurementV1<bool> {
    match preview_in_flight_at_press {
        Some(in_flight) => {
            MeasurementV1::measured(!(in_flight && crate::transcriber::is_coreml_model(model_name)))
        }
        None => MeasurementV1::Unavailable {
            reason: UnavailableReasonV1::SampleFailed,
        },
    }
}

/// Estimated index in the clip of the sample captured at `pressed_at`, for a
/// clip that ends at the stop. The clip is cut at `stop_requested_at` plus the
/// release tail the audio supervisor applies (zero or 50 ms for a toggle
/// stop), which is not visible from here, so the estimate runs late by up to
/// that tail. A press before the clip began is clamped to the clip start.
pub(crate) fn estimate_press_sample(
    clip_len: usize,
    pressed_at: std::time::Instant,
    stop_requested_at: std::time::Instant,
) -> usize {
    let lead_micros = stop_requested_at
        .saturating_duration_since(pressed_at)
        .as_micros();
    let lead_samples = lead_micros.saturating_mul(SAMPLES_PER_MS as u128) / 1_000;
    clip_len.saturating_sub(usize::try_from(lead_samples).unwrap_or(usize::MAX))
}

/// How the final pass's VAD ran, which decides what can be compared afterwards.
pub(crate) enum FinalVad {
    /// Sensitivity zero: the final pass skipped the VAD on purpose.
    Disabled,
    /// The final pass could not run the VAD (model missing, or it failed).
    Unavailable(UnavailableReasonV1),
    /// The full-clip VAD ran. Its ranges and parameters allow running it again
    /// on a prefix of the clip.
    Ran {
        ranges: Vec<SpeechRange>,
        model_path: String,
        threshold: f32,
    },
}

/// What a delivered final pass leaves for the follow-up. It stays in memory
/// until the follow-up ends and is never stored, logged or emitted: only the
/// numbers and booleans derived from it are.
pub(crate) struct FinalPass {
    pub vad: FinalVad,
    /// The PCM the final decode consumed.
    pub filtered: Vec<f32>,
    pub vad_trimmed: bool,
    /// Digest of the first decode's text. Taken only while the determinism
    /// switch is on for a re-decodable model, so its presence is the switch's
    /// state for this run: the environment is read once, at the final pass.
    pub first_decode_digest: Option<TextDigest>,
}

/// The PCM of a filter result, empty for no speech.
pub(crate) fn speech_pcm(result: &VadResult) -> &[f32] {
    match result {
        VadResult::Speech(pcm) => pcm,
        VadResult::NoSpeech => &[],
    }
}

/// Measure a delivered run's final pass against the captured `clip`. `full` is
/// the filtered output the final decode consumed.
pub(crate) fn measure_final_pass(
    vad: &FinalVad,
    full: &VadResult,
    clip: &[f32],
    press_sample: Option<usize>,
    model_free_at_press: MeasurementV1<bool>,
    decode_deterministic: MeasurementV1<bool>,
) -> ShadowMeasurementV1 {
    match vad {
        FinalVad::Ran {
            ranges,
            model_path,
            threshold,
        } => measure(
            &ShadowClip {
                clip_len: clip.len(),
                ranges,
                full,
                press_sample,
            },
            model_free_at_press,
            decode_deterministic,
            |len| vad::filter_speech(model_path, &clip[..len], *threshold),
        ),
        FinalVad::Disabled => without_vad(None, model_free_at_press, decode_deterministic),
        FinalVad::Unavailable(reason) => {
            without_vad(Some(*reason), model_free_at_press, decode_deterministic)
        }
    }
}

/// Without full-clip VAD ranges nothing about silence or prefixes can be said:
/// not applicable when the VAD was off, unavailable when it could not run.
fn without_vad(
    reason: Option<UnavailableReasonV1>,
    model_free_at_press: MeasurementV1<bool>,
    decode_deterministic: MeasurementV1<bool>,
) -> ShadowMeasurementV1 {
    fn absent<T>(reason: Option<UnavailableReasonV1>) -> MeasurementV1<T> {
        reason.map_or(MeasurementV1::NotApplicable, |reason| {
            MeasurementV1::Unavailable { reason }
        })
    }
    ShadowMeasurementV1 {
        schema_version: SHADOW_SCHEMA_VERSION,
        trailing_silence_before_press_ms: absent(reason),
        trailing_silence_before_stop_ms: absent(reason),
        press_prefix_matches_final: absent(reason),
        pause_prefix_matches_final: absent(reason),
        model_free_at_press,
        decode_deterministic,
    }
}

/// Everything the follow-up needs from the final pass, none of it content that
/// is stored: sample indices, booleans and the clip itself, which stays in
/// memory and is dropped when the follow-up ends.
pub(crate) struct ShadowClip<'a> {
    /// Samples in the captured clip, which ends at the stop.
    pub clip_len: usize,
    /// Ranges the full-clip VAD kept, padding included.
    pub ranges: &'a [SpeechRange],
    /// The full-clip filtered output the final decode consumed.
    pub full: &'a VadResult,
    /// Estimated sample of the stop-key press in the clip, when the stop came
    /// from a native stop-key gesture.
    pub press_sample: Option<usize>,
}

/// Build the measurement. `filter_prefix(len)` runs the VAD, with the final's
/// threshold, on the first `len` samples of the clip.
pub(crate) fn measure(
    clip: &ShadowClip<'_>,
    model_free_at_press: MeasurementV1<bool>,
    decode_deterministic: MeasurementV1<bool>,
    mut filter_prefix: impl FnMut(usize) -> Result<VadResult, String>,
) -> ShadowMeasurementV1 {
    let ms = |samples: usize| (samples / SAMPLES_PER_MS) as u64;
    let press = clip.press_sample.map(|sample| sample.min(clip.clip_len));
    let mut prefix_matches = |len: usize| {
        // An empty prefix is no audio at all, which the VAD reports as no
        // speech without being asked.
        let output = if len == 0 {
            Ok(VadResult::NoSpeech)
        } else {
            filter_prefix(len)
        };
        match output {
            Ok(output) => MeasurementV1::measured(vad::outputs_identical(&output, clip.full)),
            Err(_) => MeasurementV1::Unavailable {
                reason: UnavailableReasonV1::SampleFailed,
            },
        }
    };
    let pause_cut = vad::last_pause_cut(
        clip.ranges,
        clip.clip_len,
        SHADOW_MIN_PAUSE_MS * SAMPLES_PER_MS,
    );
    ShadowMeasurementV1 {
        schema_version: SHADOW_SCHEMA_VERSION,
        trailing_silence_before_press_ms: press.map_or(MeasurementV1::NotApplicable, |press| {
            MeasurementV1::measured(ms(vad::trailing_silence_samples(clip.ranges, press)))
        }),
        trailing_silence_before_stop_ms: vad::last_speech_end(clip.ranges)
            .map_or(MeasurementV1::NotApplicable, |end| {
                MeasurementV1::measured(ms(clip.clip_len.saturating_sub(end)))
            }),
        press_prefix_matches_final: press.map_or(MeasurementV1::NotApplicable, &mut prefix_matches),
        pause_prefix_matches_final: pause_cut.map_or(MeasurementV1::NotApplicable, |cut| {
            prefix_matches(cut.min(clip.clip_len))
        }),
        model_free_at_press,
        decode_deterministic,
    }
}

#[derive(Default)]
pub(crate) struct ShadowAccumulator {
    /// Measurements that finished before their run was delivered.
    pending: BTreeMap<u64, ShadowMeasurementV1>,
    /// Delivered runs that may still be amended, by recording generation.
    completed: BTreeMap<u64, String>,
}

impl ShadowAccumulator {
    /// Take a finished measurement. When the run is already delivered, returns
    /// the delivered run's ID so the caller amends exactly that row; otherwise
    /// the measurement waits for delivery.
    pub(crate) fn record(&mut self, id: u64, shadow: ShadowMeasurementV1) -> Option<String> {
        if let Some(run_id) = self.completed.remove(&id) {
            return Some(run_id);
        }
        bound(&mut self.pending, id);
        self.pending.insert(id, shadow);
        None
    }

    /// The measurement to carry into this run's delivery, if it finished first.
    pub(crate) fn pending(&self, id: u64) -> Option<ShadowMeasurementV1> {
        self.pending.get(&id).cloned()
    }

    /// The run was delivered under `run_id`. A waiting measurement was just
    /// carried into it; otherwise remember the run for a late measurement.
    pub(crate) fn completed(&mut self, id: u64, run_id: &str) {
        if self.pending.remove(&id).is_some() {
            return;
        }
        bound(&mut self.completed, id);
        self.completed.insert(id, run_id.to_string());
    }

    /// The run was dropped without being delivered (for example, diagnostics
    /// were cleared while it was in flight), so a waiting measurement has no
    /// row to join. A remembered delivered run is deliberately kept: a repeated
    /// completion of the same run finds no active row either.
    pub(crate) fn forget_pending(&mut self, id: u64) {
        self.pending.remove(&id);
    }

    pub(crate) fn clear(&mut self) {
        self.pending.clear();
        self.completed.clear();
    }
}

fn bound<T>(entries: &mut BTreeMap<u64, T>, incoming: u64) {
    while entries.len() >= MAX_TRACKED_RUNS && !entries.contains_key(&incoming) {
        let Some(oldest) = entries.keys().next().copied() else {
            break;
        };
        entries.remove(&oldest);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measured<T>(value: T) -> MeasurementV1<T> {
        MeasurementV1::measured(value)
    }

    /// A VAD stand-in: keeps the fixed ranges that fall inside the prefix and
    /// clips the one that crosses it, like the real one on a shorter clip.
    fn fake_filter(clip: &[f32], ranges: &[SpeechRange], len: usize) -> VadResult {
        let mut kept = Vec::new();
        for range in ranges {
            let end = range.end.min(len);
            if range.start < end {
                kept.extend_from_slice(&clip[range.start..end]);
            }
        }
        if kept.is_empty() {
            VadResult::NoSpeech
        } else {
            VadResult::Speech(kept)
        }
    }

    fn fixture() -> (Vec<f32>, Vec<SpeechRange>) {
        // 100 ms (1_600 samples) of speech, a 600 ms gap, 200 ms of speech and
        // 800 ms of trailing silence: 1.7 s at 16 kHz.
        let ranges = vec![0..1_600, 11_200..14_400];
        let clip: Vec<f32> = (0..27_200).map(|i| (i % 97) as f32 / 97.0).collect();
        (clip, ranges)
    }

    fn run(press_sample: Option<usize>) -> ShadowMeasurementV1 {
        let (clip, ranges) = fixture();
        let full = fake_filter(&clip, &ranges, clip.len());
        measure(
            &ShadowClip {
                clip_len: clip.len(),
                ranges: &ranges,
                full: &full,
                press_sample,
            },
            measured(true),
            MeasurementV1::NotApplicable,
            |len| Ok(fake_filter(&clip, &ranges, len)),
        )
    }

    #[test]
    fn press_inside_the_trailing_pause_matches_and_reports_the_silence() {
        // Press 400 ms after the last speech, 400 ms before the stop.
        let shadow = run(Some(14_400 + 6_400));
        assert_eq!(shadow.schema_version, SHADOW_SCHEMA_VERSION);
        assert_eq!(shadow.trailing_silence_before_press_ms, measured(400));
        assert_eq!(shadow.trailing_silence_before_stop_ms, measured(800));
        assert_eq!(shadow.press_prefix_matches_final, measured(true));
        // The trailing pause is 800 ms: a 300 ms pause trigger fires inside it.
        assert_eq!(shadow.pause_prefix_matches_final, measured(true));
        assert_eq!(shadow.model_free_at_press, measured(true));
        assert_eq!(shadow.decode_deterministic, MeasurementV1::NotApplicable);
    }

    #[test]
    fn press_before_later_speech_does_not_match() {
        // Press during the 600 ms gap, before the second utterance.
        let shadow = run(Some(1_600 + 4_000));
        assert_eq!(shadow.trailing_silence_before_press_ms, measured(250));
        assert_eq!(shadow.press_prefix_matches_final, measured(false));
        // The last 300 ms pause is still the trailing one.
        assert_eq!(shadow.pause_prefix_matches_final, measured(true));
    }

    #[test]
    fn a_pause_followed_by_more_speech_does_not_match() {
        let (clip, mut ranges) = fixture();
        // Speech resumes 200 ms into what was the trailing silence, so the last
        // 300 ms pause is the interior gap and later speech follows it.
        ranges.push(17_600..20_800);
        let full = fake_filter(&clip, &ranges, clip.len());
        let shadow = measure(
            &ShadowClip {
                clip_len: clip.len(),
                ranges: &ranges,
                full: &full,
                press_sample: None,
            },
            MeasurementV1::NotApplicable,
            MeasurementV1::NotApplicable,
            |len| Ok(fake_filter(&clip, &ranges, len)),
        );
        // Gap 14_400..17_600 is 200 ms; the trailing gap 20_800..27_200 is 400 ms.
        assert_eq!(shadow.pause_prefix_matches_final, measured(true));
        // Without a native gesture nothing is measured about the press.
        assert_eq!(
            shadow.trailing_silence_before_press_ms,
            MeasurementV1::NotApplicable
        );
        assert_eq!(
            shadow.press_prefix_matches_final,
            MeasurementV1::NotApplicable
        );

        // Drop the trailing silence below 300 ms: the interior gap is now the
        // last pause and a whole utterance follows it.
        let short = &clip[..20_800 + 3_200];
        let full = fake_filter(short, &ranges, short.len());
        let shadow = measure(
            &ShadowClip {
                clip_len: short.len(),
                ranges: &ranges,
                full: &full,
                press_sample: None,
            },
            MeasurementV1::NotApplicable,
            MeasurementV1::NotApplicable,
            |len| Ok(fake_filter(short, &ranges, len)),
        );
        assert_eq!(shadow.trailing_silence_before_stop_ms, measured(200));
        assert_eq!(shadow.pause_prefix_matches_final, measured(false));
    }

    // A single speech segment is the input here, not a mistyped `[0; n]`.
    #[allow(clippy::single_range_in_vec_init)]
    #[test]
    fn a_clip_without_a_long_pause_has_no_pause_prefix() {
        let (clip, ranges) = fixture();
        let short = &clip[..14_400 + 2_000];
        let full = fake_filter(short, &ranges, short.len());
        let shadow = measure(
            &ShadowClip {
                clip_len: short.len(),
                ranges: &[0..16_000.min(short.len())],
                full: &full,
                press_sample: None,
            },
            MeasurementV1::NotApplicable,
            MeasurementV1::NotApplicable,
            |_| panic!("no pause means no prefix pass"),
        );
        assert_eq!(
            shadow.pause_prefix_matches_final,
            MeasurementV1::NotApplicable
        );
    }

    #[test]
    fn a_press_at_the_clip_start_is_an_empty_prefix() {
        let shadow = run(Some(0));
        assert_eq!(shadow.press_prefix_matches_final, measured(false));
        assert_eq!(shadow.trailing_silence_before_press_ms, measured(0));
    }

    #[test]
    fn a_failed_prefix_pass_is_unavailable_rather_than_a_guess() {
        let (clip, ranges) = fixture();
        let full = fake_filter(&clip, &ranges, clip.len());
        let shadow = measure(
            &ShadowClip {
                clip_len: clip.len(),
                ranges: &ranges,
                full: &full,
                press_sample: Some(20_000),
            },
            MeasurementV1::NotApplicable,
            MeasurementV1::NotApplicable,
            |_| Err("vad failed".to_string()),
        );
        let unavailable = MeasurementV1::Unavailable {
            reason: UnavailableReasonV1::SampleFailed,
        };
        assert_eq!(shadow.press_prefix_matches_final, unavailable);
        assert_eq!(shadow.pause_prefix_matches_final, unavailable);
    }

    #[test]
    fn decode_determinism_is_off_unless_explicitly_enabled() {
        assert!(!decode_determinism_switch(None));
        assert!(!decode_determinism_switch(Some("")));
        assert!(!decode_determinism_switch(Some("0")));
        assert!(!decode_determinism_switch(Some("true")));
        assert!(decode_determinism_switch(Some("1")));
        // The shipped default: nothing sets the switch for the app.
        if std::env::var_os(DECODE_DETERMINISM_ENV).is_none() {
            assert!(!decode_determinism_enabled());
        }
    }

    #[test]
    fn text_digests_are_equal_only_for_identical_text() {
        assert_eq!(text_digest("same words"), text_digest("same words"));
        assert_ne!(text_digest("same words"), text_digest("same words."));
        assert_eq!(text_digest(""), text_digest(""));
    }

    #[test]
    fn redecode_outcomes_fold_into_a_boolean_or_a_reason() {
        let first = text_digest("hello world");
        assert_eq!(
            decode_determinism(&first, Ok(Some("hello world".to_string()))),
            measured(true)
        );
        assert_eq!(
            decode_determinism(&first, Ok(Some("hello, world".to_string()))),
            measured(false)
        );
        // The backend was busy, not loaded, or no longer current: skipped.
        assert_eq!(
            decode_determinism(&first, Ok(None)),
            MeasurementV1::Unavailable {
                reason: UnavailableReasonV1::DependencyPending
            }
        );
        assert_eq!(
            decode_determinism(&first, Err("backend failed".to_string())),
            MeasurementV1::Unavailable {
                reason: UnavailableReasonV1::SampleFailed
            }
        );
    }

    #[test]
    fn only_the_core_ml_model_is_redecoded_and_only_with_the_switch() {
        let coreml = crate::transcriber::COREML_MODEL_NAME;
        assert!(digest_wanted(true, coreml));
        assert!(!digest_wanted(false, coreml));
        // Whisper keeps decoder history in its reused state: never re-decoded.
        assert!(!digest_wanted(true, "base.en"));
        assert!(!digest_wanted(true, "small.en"));
        if std::env::var_os(DECODE_DETERMINISM_ENV).is_none() {
            assert!(!should_digest_first_decode(coreml));
        }
    }

    #[test]
    fn the_model_is_free_unless_a_preview_held_the_shared_backend() {
        let coreml = crate::transcriber::COREML_MODEL_NAME;
        assert_eq!(model_free_at_press(coreml, Some(false)), measured(true));
        // The Core ML preview decodes on the shared backend.
        assert_eq!(model_free_at_press(coreml, Some(true)), measured(false));
        // The Whisper preview has its own context: the shared one stays free.
        assert_eq!(model_free_at_press("base.en", Some(true)), measured(true));
        assert_eq!(
            model_free_at_press(coreml, None),
            MeasurementV1::Unavailable {
                reason: UnavailableReasonV1::SampleFailed
            }
        );
    }

    #[test]
    fn press_sample_is_the_clip_end_minus_the_press_to_stop_lead() {
        let stop = std::time::Instant::now();
        let press = stop - std::time::Duration::from_millis(250);
        // 250 ms at 16 samples per millisecond.
        assert_eq!(estimate_press_sample(48_000, press, stop), 48_000 - 4_000);
        // A press that was not before the stop sits at the clip end.
        assert_eq!(estimate_press_sample(48_000, stop, stop), 48_000);
        assert_eq!(
            estimate_press_sample(48_000, stop + std::time::Duration::from_millis(5), stop),
            48_000
        );
        // A press before the clip began clamps to the clip start.
        assert_eq!(estimate_press_sample(1_600, press, stop), 0);
    }

    #[test]
    fn a_final_pass_without_a_vad_says_why_nothing_was_compared() {
        let clip = vec![0.1_f32; 3_200];
        let full = VadResult::Speech(clip.clone());
        let off = measure_final_pass(
            &FinalVad::Disabled,
            &full,
            &clip,
            Some(1_600),
            measured(true),
            measured(false),
        );
        assert_eq!(
            off.trailing_silence_before_press_ms,
            MeasurementV1::NotApplicable
        );
        assert_eq!(
            off.trailing_silence_before_stop_ms,
            MeasurementV1::NotApplicable
        );
        assert_eq!(off.press_prefix_matches_final, MeasurementV1::NotApplicable);
        assert_eq!(off.pause_prefix_matches_final, MeasurementV1::NotApplicable);
        // What does not depend on the VAD is still reported.
        assert_eq!(off.model_free_at_press, measured(true));
        assert_eq!(off.decode_deterministic, measured(false));

        let pending = UnavailableReasonV1::DependencyPending;
        let missing = measure_final_pass(
            &FinalVad::Unavailable(pending),
            &full,
            &clip,
            None,
            MeasurementV1::NotApplicable,
            MeasurementV1::NotApplicable,
        );
        assert_eq!(
            missing.trailing_silence_before_press_ms,
            MeasurementV1::Unavailable { reason: pending }
        );
        assert_eq!(
            missing.trailing_silence_before_stop_ms,
            MeasurementV1::Unavailable { reason: pending }
        );
        assert_eq!(
            missing.press_prefix_matches_final,
            MeasurementV1::Unavailable { reason: pending }
        );
        assert_eq!(
            missing.pause_prefix_matches_final,
            MeasurementV1::Unavailable { reason: pending }
        );
    }

    #[test]
    fn a_prefix_pass_that_cannot_run_the_vad_is_unavailable_not_false() {
        // A model path that does not exist fails the filter without a model.
        let missing = std::env::temp_dir().join("murmur-shadow-missing-vad-model.bin");
        let (clip, ranges) = fixture();
        let full = fake_filter(&clip, &ranges, clip.len());
        let shadow = measure_final_pass(
            &FinalVad::Ran {
                ranges,
                model_path: missing.to_string_lossy().to_string(),
                threshold: 0.5,
            },
            &full,
            &clip,
            Some(20_000),
            MeasurementV1::NotApplicable,
            MeasurementV1::NotApplicable,
        );
        // Silence comes from the ranges alone and needs no second VAD pass.
        assert_eq!(shadow.trailing_silence_before_press_ms, measured(350));
        assert_eq!(shadow.trailing_silence_before_stop_ms, measured(800));
        let unavailable = MeasurementV1::Unavailable {
            reason: UnavailableReasonV1::SampleFailed,
        };
        assert_eq!(shadow.press_prefix_matches_final, unavailable);
        assert_eq!(shadow.pause_prefix_matches_final, unavailable);
    }

    #[test]
    fn serialized_measurement_is_numbers_booleans_and_status_only() {
        let shadow = run(Some(20_000));
        let json = serde_json::to_value(&shadow).unwrap();
        assert_eq!(json["schemaVersion"], 1);
        assert_eq!(
            json["trailingSilenceBeforeStopMs"],
            serde_json::json!({ "status": "measured", "value": 800 })
        );
        assert_eq!(
            json["decodeDeterministic"],
            serde_json::json!({ "status": "notApplicable" })
        );
        let back: ShadowMeasurementV1 = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(back, shadow);
        // Future schemas and unknown content-bearing keys are refused.
        let mut future = json.clone();
        future["schemaVersion"] = serde_json::json!(2);
        assert!(serde_json::from_value::<ShadowMeasurementV1>(future).is_err());
        let mut extra = json;
        extra["transcript"] = serde_json::json!("words");
        assert!(serde_json::from_value::<ShadowMeasurementV1>(extra).is_err());
    }

    #[test]
    fn accumulator_pairs_a_measurement_with_its_delivered_run_either_way() {
        let (clip, ranges) = fixture();
        let full = fake_filter(&clip, &ranges, clip.len());
        let shadow = measure(
            &ShadowClip {
                clip_len: clip.len(),
                ranges: &ranges,
                full: &full,
                press_sample: None,
            },
            MeasurementV1::NotApplicable,
            MeasurementV1::NotApplicable,
            |len| Ok(fake_filter(&clip, &ranges, len)),
        );

        // Finished before delivery: it rides along with the run being delivered.
        let mut early = ShadowAccumulator::default();
        assert_eq!(early.record(7, shadow.clone()), None);
        assert_eq!(early.pending(7), Some(shadow.clone()));
        early.completed(7, "run-7");
        assert_eq!(early.pending(7), None);
        // Nothing is left to amend later.
        assert_eq!(early.record(7, shadow.clone()), None);

        // Finished after delivery: the delivered run's ID comes back, once.
        let mut late = ShadowAccumulator::default();
        late.completed(8, "run-8");
        assert_eq!(late.record(8, shadow.clone()), Some("run-8".to_string()));
        assert_eq!(late.record(8, shadow.clone()), None);

        // Clearing diagnostics forgets both.
        let mut cleared = ShadowAccumulator::default();
        cleared.completed(9, "run-9");
        cleared.clear();
        assert_eq!(cleared.record(9, shadow), None);
    }

    #[test]
    fn accumulator_is_bounded() {
        let mut accumulator = ShadowAccumulator::default();
        for id in 0..(MAX_TRACKED_RUNS as u64 * 3) {
            accumulator.completed(id, "run");
        }
        assert!(accumulator.completed.len() <= MAX_TRACKED_RUNS);
        assert!(accumulator
            .completed
            .contains_key(&(MAX_TRACKED_RUNS as u64 * 3 - 1)));
        assert!(!accumulator.completed.contains_key(&0));
    }
}
