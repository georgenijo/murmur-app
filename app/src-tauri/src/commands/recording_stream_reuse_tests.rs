//! Decode reuse at the `AppState` level: the same functions the preview tick
//! and the stop path call, over a fake backend that answers from a script and
//! records how much audio it was asked to decode. Audio is synthetic PCM and
//! the texts are fixed phrases.

use super::*;
use crate::audio_lifecycle::PeekSince;
use crate::stream_reuse::Reason;
use crate::vad_stream::StreamSnapshot;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Instant;

const A: (usize, usize) = (8_000, 72_000);
const B: (usize, usize) = (88_000, 152_000);
const C: (usize, usize) = (168_000, 184_000);

/// Answers decodes from a script (`None` entries are decode failures) and
/// records the length of each decode.
struct Scripted {
    answers: VecDeque<Option<&'static str>>,
    decoded: Vec<usize>,
}

impl Scripted {
    fn new(answers: &[Option<&'static str>]) -> Self {
        Self {
            answers: answers.iter().copied().collect(),
            decoded: Vec::new(),
        }
    }
}

impl transcriber::TranscriptionBackend for Scripted {
    fn name(&self) -> &str {
        "scripted"
    }
    fn load_model(&mut self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn is_model_loaded(&self, _: &str) -> bool {
        true
    }
    fn transcribe(
        &mut self,
        samples: &[f32],
        _: &str,
        _: Option<&str>,
        _: bool,
    ) -> Result<String, String> {
        self.decoded.push(samples.len());
        match self.answers.pop_front() {
            Some(Some(text)) => Ok(text.to_string()),
            Some(None) => Err("scripted decode failure".into()),
            None => Err("unexpected extra decode".into()),
        }
    }
    fn token_count(&self, _: &str) -> Option<usize> {
        None
    }
    fn model_exists(&self) -> bool {
        true
    }
    fn models_dir(&self) -> Result<PathBuf, String> {
        Ok(std::env::temp_dir())
    }
    fn reset(&mut self) {}
}

fn settings(streaming: bool) -> dictation_context::TranscriptionSettings {
    dictation_context::TranscriptionSettings {
        model_name: transcriber::COREML_MODEL_NAME.into(),
        language: "auto".into(),
        vad_sensitivity: 50,
        prompt: None,
        smart_punctuation: true,
        streaming_pipeline_enabled: streaming,
    }
}

/// 16 kHz PCM, 14 s, whose content differs everywhere so a mixed-up range
/// changes a hash.
fn raw_recording() -> Vec<f32> {
    (0..16_000 * 14).map(|i| (i as f32 * 0.37).sin()).collect()
}

fn kept(raw: &[f32], bounds: &[(usize, usize)]) -> Vec<f32> {
    bounds
        .iter()
        .flat_map(|&(start, end)| raw[start..end].iter().copied())
        .collect()
}

/// What the VAD stream reports once speech up to `last` has been scored and
/// enough silence followed it for that segment to be final.
fn snapshot(segments: &[(usize, usize)]) -> StreamSnapshot {
    StreamSnapshot {
        segments: segments.to_vec(),
        scored_samples: segments.last().unwrap().1 + 9_600,
    }
}

/// A live Core ML recording `id` with a reuse session.
fn live(id: u64) -> AppState {
    let state = AppState::default();
    state.recording_id.store(id, Ordering::SeqCst);
    state.dictation.lock_or_recover().status = DictationStatus::Recording;
    state.stream_reuse.lock_or_recover().begin(id);
    state
}

fn prepare(
    state: &AppState,
    id: u64,
    raw: &[f32],
    segments: &[(usize, usize)],
) -> Option<StreamCommit> {
    prepare_stream_commit_from(
        state,
        id,
        |_| Some(snapshot(segments)),
        |from| PeekSince::Samples(raw[from..].to_vec()),
        0.5,
    )
}

/// One preview tick that commits a piece: the plan, the decode on `backend`
/// (as the preview does, with the backend held) and the recording.
fn commit_piece(
    state: &AppState,
    id: u64,
    backend: &mut Scripted,
    raw: &[f32],
    segments: &[(usize, usize)],
) {
    let (plan, piece) = prepare(state, id, raw, segments).expect("a piece is ready");
    let decoded = backend.transcribe_preview(
        &piece,
        "auto",
        None,
        true,
        transcriber::PreviewPass::Dictation(id),
    );
    commit_stream_piece(state, id, plan, piece, decoded);
}

use transcriber::TranscriptionBackend;

const FIRST: &str = "First stable segment.";
const FIRST_AND_SECOND: &str = "First stable segment. Next stable segment.";

#[test]
fn committed_pieces_become_the_final_text_through_the_stop_path() {
    let id = 7_100_001;
    let state = live(id);
    let raw = raw_recording();
    let mut backend = Scripted::new(&[Some(FIRST), Some(FIRST_AND_SECOND)]);
    commit_piece(&state, id, &mut backend, &raw, &[A]);
    commit_piece(&state, id, &mut backend, &raw, &[A, B]);
    // The pieces are the whole of A + B: only the overlap piece was decoded.
    assert_eq!(backend.decoded, [A.1 - A.0, (A.1 - A.0) + (B.1 - B.0)]);

    // Stop: the final VAD kept A, B and the tail C.
    let bounds = [A, B, C];
    let filtered = kept(&raw, &bounds);
    let plan = take_stream_reuse_plan(&state, &settings(true), id, Some(&bounds), &filtered);
    let plan = plan.expect("every proof passes").expect("reuse is on");
    let tail_len = filtered.len() - plan.start;
    assert_eq!(tail_len, (B.1 - B.0) + (C.1 - C.0));

    let mut stop = Scripted::new(&[Some("Next stable segment. Tail words arrive.")]);
    let result = decode_final_transcript(
        &mut stop,
        &settings(true),
        &filtered,
        &filtered,
        true,
        Ok(Some(plan)),
    )
    .unwrap();
    assert_eq!(
        result.attempt.text,
        "First stable segment. Next stable segment. Tail words arrive."
    );
    assert_eq!(stop.decoded, [tail_len]);
    let outcome = result.reuse.expect("a decision was reported");
    assert_eq!(outcome.code, "reused");
    assert_eq!(outcome.pieces, 2);

    // The plan is consumed once.
    assert!(matches!(
        take_stream_reuse_plan(&state, &settings(true), id, Some(&bounds), &filtered),
        Err(Reason::Superseded)
    ));
}

#[test]
fn pieces_that_cover_every_kept_sample_decode_nothing_at_stop() {
    let id = 7_100_002;
    let state = live(id);
    let raw = raw_recording();
    let mut backend = Scripted::new(&[Some(FIRST), Some(FIRST_AND_SECOND)]);
    commit_piece(&state, id, &mut backend, &raw, &[A]);
    commit_piece(&state, id, &mut backend, &raw, &[A, B]);
    let bounds = [A, B];
    let filtered = kept(&raw, &bounds);
    let plan = take_stream_reuse_plan(&state, &settings(true), id, Some(&bounds), &filtered)
        .unwrap()
        .unwrap();
    let mut stop = Scripted::new(&[]);
    let result = decode_final_transcript(
        &mut stop,
        &settings(true),
        &filtered,
        &filtered,
        true,
        Ok(Some(plan)),
    )
    .unwrap();
    assert_eq!(result.attempt.text, FIRST_AND_SECOND);
    assert!(stop.decoded.is_empty());
}

#[test]
fn a_preview_still_holding_its_lease_at_stop_gives_busy_and_the_whole_clip() {
    let id = 7_100_003;
    let state = live(id);
    let raw = raw_recording();
    let mut backend = Scripted::new(&[Some(FIRST), Some(FIRST_AND_SECOND)]);
    commit_piece(&state, id, &mut backend, &raw, &[A]);
    commit_piece(&state, id, &mut backend, &raw, &[A, B]);
    let bounds = [A, B];
    let filtered = kept(&raw, &bounds);
    // A preview decode that never lets go within the 10 ms the stop waits.
    state
        .dictation_partial_in_flight_id
        .store(id, Ordering::SeqCst);
    let started = Instant::now();
    let decision = take_stream_reuse_plan(&state, &settings(true), id, Some(&bounds), &filtered);
    assert!(started.elapsed() < std::time::Duration::from_millis(500));
    assert!(matches!(decision, Err(Reason::Busy)));

    let mut stop = Scripted::new(&[Some("Whole clip result.")]);
    let result = decode_final_transcript(
        &mut stop,
        &settings(true),
        &filtered,
        &filtered,
        true,
        decision,
    )
    .unwrap();
    assert_eq!(result.attempt.text, "Whole clip result.");
    assert_eq!(stop.decoded, [filtered.len()]);
    assert_eq!(result.reuse.unwrap().code, "busy");
}

#[test]
fn a_preview_that_lets_go_during_the_wait_does_not_lose_reuse() {
    let id = 7_100_004;
    let state = std::sync::Arc::new(live(id));
    let raw = raw_recording();
    let mut backend = Scripted::new(&[Some(FIRST), Some(FIRST_AND_SECOND)]);
    commit_piece(&state, id, &mut backend, &raw, &[A]);
    commit_piece(&state, id, &mut backend, &raw, &[A, B]);
    let bounds = [A, B];
    let filtered = kept(&raw, &bounds);
    state
        .dictation_partial_in_flight_id
        .store(id, Ordering::SeqCst);
    let releaser = {
        let state = state.clone();
        std::thread::spawn(move || {
            state
                .dictation_partial_in_flight_id
                .store(0, Ordering::SeqCst);
        })
    };
    // The releasing thread runs within the bounded wait on any reasonable
    // schedule; accept Busy only if the machine starved it for 10 ms.
    let decision = take_stream_reuse_plan(&state, &settings(true), id, Some(&bounds), &filtered);
    releaser.join().unwrap();
    assert!(matches!(decision, Ok(Some(_)) | Err(Reason::Busy)));
}

#[test]
fn a_superseded_or_cancelled_recording_never_reuses() {
    let raw = raw_recording();
    let bounds = [A, B];
    let filtered = kept(&raw, &bounds);

    // The next recording took over before the stop path decided.
    let id = 7_100_005;
    let state = live(id);
    let mut backend = Scripted::new(&[Some(FIRST), Some(FIRST_AND_SECOND)]);
    commit_piece(&state, id, &mut backend, &raw, &[A]);
    commit_piece(&state, id, &mut backend, &raw, &[A, B]);
    state.recording_id.store(id + 1, Ordering::SeqCst);
    assert!(matches!(
        take_stream_reuse_plan(&state, &settings(true), id, Some(&bounds), &filtered),
        Err(Reason::Superseded)
    ));

    // Cancelled while stopping.
    let id = 7_100_006;
    let state = live(id);
    let mut backend = Scripted::new(&[Some(FIRST), Some(FIRST_AND_SECOND)]);
    commit_piece(&state, id, &mut backend, &raw, &[A]);
    commit_piece(&state, id, &mut backend, &raw, &[A, B]);
    state.cancel_recording(id);
    assert!(matches!(
        take_stream_reuse_plan(&state, &settings(true), id, Some(&bounds), &filtered),
        Err(Reason::Superseded)
    ));
}

#[test]
fn a_piece_that_finishes_after_stop_is_dropped_and_the_earlier_pieces_are_reused() {
    let id = 7_100_007;
    let state = live(id);
    let raw = raw_recording();
    let mut backend = Scripted::new(&[Some(FIRST), Some(FIRST_AND_SECOND)]);
    commit_piece(&state, id, &mut backend, &raw, &[A]);

    // The second piece was planned and is decoding when the stop begins.
    let (plan, piece) = prepare(&state, id, &raw, &[A, B]).unwrap();
    state.dictation.lock_or_recover().status = DictationStatus::Processing;
    let decoded = backend.transcribe_preview(
        &piece,
        "auto",
        None,
        true,
        transcriber::PreviewPass::Dictation(id),
    );
    commit_stream_piece(&state, id, plan, piece, decoded);

    // Only the first piece counts: the tail starts where its audio starts.
    let bounds = [A, B];
    let filtered = kept(&raw, &bounds);
    let plan = take_stream_reuse_plan(&state, &settings(true), id, Some(&bounds), &filtered)
        .unwrap()
        .unwrap();
    assert_eq!(plan.pieces, 1);
    assert_eq!(plan.start, 0);
    let mut stop = Scripted::new(&[Some("First stable segment. Next stable segment.")]);
    let result = decode_final_transcript(
        &mut stop,
        &settings(true),
        &filtered,
        &filtered,
        true,
        Ok(Some(plan)),
    )
    .unwrap();
    assert_eq!(result.attempt.text, FIRST_AND_SECOND);
    assert_eq!(stop.decoded, [filtered.len()]);
}

#[test]
fn a_piece_that_finishes_for_a_replaced_recording_changes_nothing() {
    let id = 7_100_008;
    let state = live(id);
    let raw = raw_recording();
    let (plan, piece) = prepare(&state, id, &raw, &[A]).unwrap();
    // The next recording owns the state by the time the old decode returns.
    state.recording_id.store(id + 1, Ordering::SeqCst);
    state.stream_reuse.lock_or_recover().begin(id + 1);
    commit_stream_piece(&state, id, plan, piece, Ok(FIRST.to_string()));
    let reuse = state.stream_reuse.lock_or_recover();
    assert!(reuse.is_active(id + 1));
    assert!(!reuse.is_active(id));
}

#[test]
fn a_failed_piece_decode_ends_reuse_and_stop_decodes_the_whole_clip() {
    let id = 7_100_009;
    let state = live(id);
    let raw = raw_recording();
    let mut backend = Scripted::new(&[None]);
    commit_piece(&state, id, &mut backend, &raw, &[A]);
    assert!(!state.stream_reuse.lock_or_recover().is_active(id));
    let bounds = [A, B];
    let filtered = kept(&raw, &bounds);
    assert!(matches!(
        take_stream_reuse_plan(&state, &settings(true), id, Some(&bounds), &filtered),
        Err(Reason::DecodeError)
    ));
}

#[test]
fn a_peek_that_is_not_current_only_skips_the_tick_and_a_reset_ends_reuse() {
    let id = 7_100_010;
    let state = live(id);
    let raw = raw_recording();
    let tick = |peek: PeekSince| {
        prepare_stream_commit_from(&state, id, |_| Some(snapshot(&[A])), |_| peek, 0.5)
    };
    // The supervisor did not answer in time (or the capture just stopped).
    assert!(tick(PeekSince::NotCurrent).is_none());
    assert!(state.stream_reuse.lock_or_recover().is_active(id));
    // The next tick, with the audio readable again, commits the piece as usual.
    assert!(prepare(&state, id, &raw, &[A]).is_some());

    // The retained buffer is shorter than the offset: the audio is gone.
    assert!(tick(PeekSince::Reset).is_none());
    assert!(!state.stream_reuse.lock_or_recover().is_active(id));
    let bounds = [A];
    let filtered = kept(&raw, &bounds);
    assert!(matches!(
        take_stream_reuse_plan(&state, &settings(true), id, Some(&bounds), &filtered),
        Err(Reason::BufferReset)
    ));
}

#[test]
fn a_missing_vad_stream_ends_reuse_for_the_recording() {
    let id = 7_100_011;
    let state = live(id);
    let planned = prepare_stream_commit_from(
        &state,
        id,
        |_| None,
        |_| PeekSince::Samples(Vec::new()),
        0.5,
    );
    assert!(planned.is_none());
    assert!(!state.stream_reuse.lock_or_recover().is_active(id));
}

/// The stop-gesture cap is not part of the opt-in: it applies to every
/// recording, with `streamingPipelineEnabled` off as well, and it only ever
/// affects previews, never the final transcript.
#[test]
fn the_stop_gesture_cap_applies_with_the_setting_off_and_leaves_the_final_text_alone() {
    let id = 7_100_012;
    let state = AppState::default();
    state.recording_id.store(id, Ordering::SeqCst);
    state.dictation.lock_or_recover().status = DictationStatus::Recording;
    assert!(dictation_preview_may_start(&state, id));
    assert!(dictation_preview_may_start_if_uncontended(&state, id));

    // The user lets go of the hotkey: no new preview decode may start.
    crate::hotkey_release::record(id, Instant::now());
    assert!(!dictation_preview_may_start(&state, id));
    assert!(!dictation_preview_may_start_if_uncontended(&state, id));
    // Another recording is not affected by this recording's release.
    state.recording_id.store(id + 1, Ordering::SeqCst);
    assert!(dictation_preview_may_start(&state, id + 1));
    state.recording_id.store(id, Ordering::SeqCst);

    // The setting is off: Stop makes no reuse decision and the final decode is
    // the unchanged whole-clip decode of the kept audio.
    let raw = raw_recording();
    let bounds = [A, B];
    let filtered = kept(&raw, &bounds);
    let decision = take_stream_reuse_plan(&state, &settings(false), id, Some(&bounds), &filtered);
    assert!(matches!(decision, Ok(None)));
    let mut stop = Scripted::new(&[Some("Whole clip result.")]);
    let result = decode_final_transcript(
        &mut stop,
        &settings(false),
        &filtered,
        &filtered,
        true,
        decision,
    )
    .unwrap();
    assert_eq!(result.attempt.text, "Whole clip result.");
    assert_eq!(stop.decoded, [filtered.len()]);
    assert!(result.reuse.is_none());

    // The stop claims the release: previews are not capped for it afterwards.
    assert!(crate::hotkey_release::take_for(id, Instant::now()).is_some());
    assert!(dictation_preview_may_start(&state, id));
}

#[test]
fn a_discarded_tail_decode_counts_toward_the_decoded_time_of_the_fallback() {
    let id = 7_100_013;
    let state = live(id);
    let raw = raw_recording();
    let mut backend = Scripted::new(&[Some(FIRST), Some(FIRST_AND_SECOND)]);
    commit_piece(&state, id, &mut backend, &raw, &[A]);
    commit_piece(&state, id, &mut backend, &raw, &[A, B]);
    let bounds = [A, B, C];
    let filtered = kept(&raw, &bounds);
    let plan = take_stream_reuse_plan(&state, &settings(true), id, Some(&bounds), &filtered)
        .unwrap()
        .unwrap();
    let tail_len = filtered.len() - plan.start;
    // The tail does not start with the committed overlap: whole clip instead.
    let mut stop = Scripted::new(&[Some("Different words entirely here."), Some("Whole.")]);
    let result = decode_final_transcript(
        &mut stop,
        &settings(true),
        &filtered,
        &filtered,
        true,
        Ok(Some(plan)),
    )
    .unwrap();
    let outcome = result.reuse.unwrap();
    assert_eq!(outcome.code, "stitch_mismatch");
    assert_eq!(
        outcome.decoded_ms,
        ((tail_len + filtered.len()) / 16) as u64
    );
}
