use super::*;
use crate::vad::VadResult;

fn params() -> SegmentParams {
    SegmentParams::murmur(0.5)
}

/// `n` windows of probability `p`.
fn run(p: f32, n: usize) -> Vec<f32> {
    vec![p; n]
}

fn probs(parts: &[(f32, usize)]) -> Vec<f32> {
    parts.iter().flat_map(|(p, n)| run(*p, *n)).collect()
}

#[test]
fn no_probabilities_or_only_silence_give_no_segments() {
    assert!(segments_from_probs(&[], params()).is_empty());
    assert!(segments_from_probs(&run(0.01, 80), params()).is_empty());
}

#[test]
fn one_burst_ends_after_min_silence_and_is_padded() {
    // Speech windows 10..40, then silence. 100 ms of silence (3.1 windows) ends
    // it at the 5th silent window; the segment ends where silence began.
    let p = probs(&[(0.0, 10), (0.9, 30), (0.0, 60)]);
    // start 5120 - 1920 pad = 3200 -> 20 cs; end 20480 + 1920 = 22400 -> 140 cs.
    assert_eq!(segments_from_probs(&p, params()), vec![(20.0, 140.0)]);
}

#[test]
fn speech_shorter_than_min_speech_is_dropped() {
    // 3 windows is 96 ms, under the 250 ms minimum.
    let p = probs(&[(0.0, 10), (0.9, 3), (0.0, 40)]);
    assert!(segments_from_probs(&p, params()).is_empty());
}

#[test]
fn close_bursts_merge_when_the_gap_is_under_200ms() {
    let p = probs(&[(0.0, 10), (0.9, 16), (0.0, 5), (0.9, 16), (0.0, 14)]);
    // One merged segment: 5120..24064, padded to 3200..25984.
    assert_eq!(segments_from_probs(&p, params()), vec![(20.0, 162.0)]);
}

#[test]
fn bursts_with_a_mid_gap_split_the_padding_between_them() {
    let p = probs(&[(0.0, 10), (0.9, 16), (0.0, 7), (0.9, 16), (0.0, 21)]);
    // Gap 3584 samples is under 2 * 1920 pad, so the difference is split.
    assert_eq!(
        segments_from_probs(&p, params()),
        vec![(20.0, 94.0), (94.0, 169.0)]
    );
}

#[test]
fn bursts_with_a_wide_gap_each_get_full_padding() {
    let p = probs(&[(0.0, 10), (0.9, 16), (0.0, 20), (0.9, 16), (0.0, 20)]);
    // Gap 10240 samples: full 1920 padding on both sides of it.
    let segments = segments_from_probs(&p, params());
    assert_eq!(segments.len(), 2);
    // seg1: 5120..13312, padded 3200..15232 (end = 13312 + 1920).
    assert_eq!(segments[0], (20.0, 95.0));
}

#[test]
fn speech_running_to_the_end_is_closed_at_the_clip_end() {
    let p = probs(&[(0.0, 10), (0.9, 20)]);
    // 30 windows = 15360 samples; the end pad cannot pass the clip end.
    assert_eq!(segments_from_probs(&p, params()), vec![(20.0, 96.0)]);
}

#[test]
fn probabilities_between_the_thresholds_do_not_end_speech() {
    // 0.4 is under the 0.5 start threshold but over the 0.35 end threshold.
    let p = probs(&[(0.0, 10), (0.9, 16), (0.4, 34)]);
    assert_eq!(segments_from_probs(&p, params()), vec![(20.0, 192.0)]);
}

#[test]
fn a_low_threshold_floors_the_end_threshold() {
    // threshold 0.1 -> end threshold floors at 0.01, so 0.05 never ends speech.
    let params = SegmentParams::murmur(0.1);
    let p = probs(&[(0.0, 10), (0.9, 16), (0.05, 34)]);
    let segments = segments_from_probs(&p, params);
    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].1, 192.0);
}

/// A deterministic stand-in for the model: state dependent, so chunking that
/// reordered or skipped windows would change the result.
struct Fake;
impl WindowScorer for Fake {
    fn score(
        &mut self,
        window: &[f32; WINDOW_SAMPLES],
        state: &mut [f32; STATE_LEN],
    ) -> Result<f32, ScoreError> {
        let energy = window.iter().map(|x| x * x).sum::<f32>() / WINDOW_SAMPLES as f32;
        state[0] = state[0] * 0.5 + energy;
        Ok((state[0] * 40.0).min(1.0))
    }
}

struct Failing;
impl WindowScorer for Failing {
    fn score(
        &mut self,
        _: &[f32; WINDOW_SAMPLES],
        _: &mut [f32; STATE_LEN],
    ) -> Result<f32, ScoreError> {
        Err("boom".into())
    }
}

/// Behaves like the shared session being busy once the stream is finishing.
#[derive(Default)]
struct BusyAtStop {
    blocking: Option<bool>,
}
impl WindowScorer for BusyAtStop {
    fn set_blocking(&mut self, blocking: bool) {
        self.blocking = Some(blocking);
    }
    fn score(
        &mut self,
        window: &[f32; WINDOW_SAMPLES],
        state: &mut [f32; STATE_LEN],
    ) -> Result<f32, ScoreError> {
        if self.blocking == Some(false) {
            return Err(ScoreError::Contended);
        }
        Fake.score(window, state)
    }
}

/// Scores slowly, so a push is still running when Stop takes the stream.
struct Slow;
impl WindowScorer for Slow {
    fn score(
        &mut self,
        window: &[f32; WINDOW_SAMPLES],
        state: &mut [f32; STATE_LEN],
    ) -> Result<f32, ScoreError> {
        std::thread::sleep(Duration::from_millis(2));
        Fake.score(window, state)
    }
}

/// Speech-like bursts of a tone separated by silence.
fn clip(len: usize) -> Vec<f32> {
    (0..len)
        .map(|n| {
            let t = n as f32 / SAMPLE_RATE as f32;
            let gate = if (n / 8_000) % 3 == 1 { 0.0 } else { 1.0 };
            gate * 0.3 * (2.0 * std::f32::consts::PI * 220.0 * t).sin()
        })
        .collect()
}

fn whole_probs(samples: &[f32]) -> Vec<f32> {
    let mut vad = IncrementalVad::new(Fake);
    vad.push(samples).unwrap();
    vad.flush().unwrap();
    vad.probs
}

#[test]
fn chunk_size_does_not_change_the_probabilities() {
    let samples = clip(48_123);
    let reference = whole_probs(&samples);
    assert_eq!(reference.len(), samples.len().div_ceil(WINDOW_SAMPLES));
    for chunk in [1, 7, 511, 512, 513, 3_200, 9_999] {
        let mut vad = IncrementalVad::new(Fake);
        for piece in samples.chunks(chunk) {
            vad.push(piece).unwrap();
        }
        vad.flush().unwrap();
        assert_eq!(vad.probs, reference, "chunk size {chunk}");
    }
}

#[test]
fn finish_scores_only_the_tail_and_matches_the_whole_clip() {
    let samples = clip(48_123);
    let reference = whole_probs(&samples);
    let mut vad = IncrementalVad::new(Fake);
    // A recording-time poll scored the first 30_000 samples.
    vad.push(&samples[..30_000]).unwrap();
    let before = vad.probs.len();
    let finished = vad.finish(&samples, params()).unwrap();
    assert_eq!(finished.total_windows, reference.len());
    assert_eq!(finished.tail_windows, reference.len() - before);
    assert!(finished.tail_windows < reference.len());
    let expected = segments_from_probs(&reference, params());
    let (expected_result, expected_bounds) =
        crate::vad::speech_from_segments::<true>(expected, &samples);
    assert_eq!(finished.bounds, expected_bounds);
    match (finished.result, expected_result) {
        (VadResult::Speech(a), VadResult::Speech(b)) => assert_eq!(a, b),
        (VadResult::NoSpeech, VadResult::NoSpeech) => {}
        _ => panic!("results differ"),
    }
}

#[test]
fn finish_on_an_empty_clip_is_no_speech() {
    let vad = IncrementalVad::new(Fake);
    let finished = vad.finish(&[], params()).unwrap();
    assert!(matches!(finished.result, VadResult::NoSpeech));
    assert_eq!(finished.total_windows, 0);
}

#[test]
fn finish_refuses_a_buffer_that_does_not_extend_what_was_scored() {
    let samples = clip(20_000);
    let mut shorter = IncrementalVad::new(Fake);
    shorter.push(&samples).unwrap();
    assert_eq!(
        shorter.finish(&samples[..10_000], params()).err(),
        Some(FallbackReason::LengthMismatch)
    );

    let mut changed = IncrementalVad::new(Fake);
    changed.push(&samples[..16_000]).unwrap();
    let mut other = samples.clone();
    other[640] += 0.5; // a sampled index (multiple of the fingerprint stride)
    assert_eq!(
        changed.finish(&other, params()).err(),
        Some(FallbackReason::PrefixMismatch)
    );
}

#[test]
fn a_failing_scorer_falls_back_instead_of_guessing() {
    let samples = clip(4_096);
    let mut vad = IncrementalVad::new(Failing);
    assert!(vad.push(&samples[..100]).is_ok()); // under one window: nothing scored
    assert_eq!(
        vad.finish(&samples, params()).err(),
        Some(FallbackReason::Inference)
    );
}

#[test]
fn fingerprint_is_the_same_incrementally_and_over_the_whole_prefix() {
    let samples = clip(10_001);
    let whole = fold_fingerprint(FINGERPRINT_SEED, 0, &samples);
    let mut hash = FINGERPRINT_SEED;
    let mut base = 0;
    for piece in samples.chunks(333) {
        hash = fold_fingerprint(hash, base, piece);
        base += piece.len();
    }
    assert_eq!(hash, whole);
}

#[test]
fn model_input_is_the_window_with_ggmls_left_reflection() {
    let mut window = [0.0_f32; WINDOW_SAMPLES];
    for (i, slot) in window.iter_mut().enumerate() {
        *slot = i as f32;
    }
    let input = reflected_input(&window);
    assert_eq!(input[0], 64.0);
    assert_eq!(input[63], 1.0);
    assert_eq!(&input[CONTEXT_SAMPLES..], &window[..]);
}

#[test]
fn slot_state_belongs_to_exactly_one_recording() {
    let slot = StreamSlot::<Fake>::default();
    assert!(slot.consumed(1).is_none());
    assert!(slot.begin_with(7, Fake));
    assert_eq!(slot.consumed(7), Some(0));
    assert!(slot.consumed(8).is_none(), "another id never sees it");
    assert!(slot.push(7, 0, &[0.0; 100]));
    assert_eq!(slot.consumed(7), Some(100));
    assert!(!slot.push(8, 100, &[0.0; 10]), "a stale id cannot push");
    assert!(
        !slot.push(7, 5, &[0.0; 10]),
        "a gap or overlap fails the stream"
    );
    assert!(slot.consumed(7).is_none(), "a failed stream stops polling");
    assert!(slot.take(8).is_none());
    // The failed stream hands back its failure, which means batch fallback.
    let handoff = slot.take(7).expect("owner takes it");
    assert_eq!(
        handoff.finish(&[0.0; 100], params()).err(),
        Some(FallbackReason::BufferReset)
    );
    assert!(slot.take(7).is_none(), "consumed exactly once");
}

#[test]
fn a_new_recording_resets_the_slot() {
    let slot = StreamSlot::<Fake>::default();
    assert!(slot.begin_with(1, Fake));
    assert!(slot.push(1, 0, &[0.0; 100]));
    assert!(slot.begin_with(2, Fake));
    assert!(slot.consumed(1).is_none());
    assert_eq!(slot.consumed(2), Some(0));
    slot.clear(1);
    assert_eq!(slot.consumed(2), Some(0), "clearing a stale id is a no-op");
    slot.clear(2);
    assert!(slot.consumed(2).is_none());
}

#[test]
fn a_recording_can_begin_only_once_so_two_workers_cannot_share_it() {
    let slot = StreamSlot::<Fake>::default();
    assert!(slot.begin_with(5, Fake));
    assert!(slot.push(5, 0, &[0.0; 100]));
    assert!(
        !slot.begin_with(5, Fake),
        "a second start for the same id is refused"
    );
    assert_eq!(
        slot.consumed(5),
        Some(100),
        "and it does not reset the stream"
    );
    assert!(
        !slot.begin_with(4, Fake),
        "an older id cannot replace a newer one"
    );
    // Even after the stream was taken at Stop, a late duplicate start cannot
    // revive the id.
    assert!(slot.take(5).is_some());
    assert!(!slot.begin_with(5, Fake));
    assert!(slot.consumed(5).is_none());
    assert!(slot.begin_with(6, Fake));
}

#[test]
fn stop_takes_the_stream_between_chunks_instead_of_waiting_for_the_backlog() {
    let slot = std::sync::Arc::new(StreamSlot::<Slow>::default());
    assert!(slot.begin_with(9, Slow));
    // 16 chunks of about 16 windows at 2 ms each: about half a second of work.
    let backlog = vec![0.1_f32; PUSH_CHUNK_SAMPLES * 16];
    let worker = {
        let slot = slot.clone();
        std::thread::spawn(move || slot.push(9, 0, &backlog))
    };
    std::thread::sleep(Duration::from_millis(60));
    let started = std::time::Instant::now();
    let handoff = slot.take(9).expect("stop takes the stream");
    let waited = started.elapsed();
    assert!(!worker.join().unwrap(), "the worker lets go once stop asks");
    let scored = handoff.vad.consumed();
    assert!(
        scored > 0 && scored < PUSH_CHUNK_SAMPLES * 16,
        "scored {scored}"
    );
    assert_eq!(
        scored % PUSH_CHUNK_SAMPLES,
        0,
        "only whole chunks were scored"
    );
    assert!(waited < Duration::from_millis(200), "waited {waited:?}");
}

#[test]
fn stop_scores_only_a_small_tail_and_falls_back_on_a_large_backlog() {
    let samples = clip(5 * SAMPLE_RATE);
    // A worker that kept up: the tail is under the limit.
    let mut kept_up = IncrementalVad::new(Fake);
    kept_up
        .push(&samples[..samples.len() - SAMPLE_RATE])
        .unwrap();
    assert!(kept_up.finish(&samples, params()).is_ok());
    // A worker that stopped early (a slow supervisor reply): over 2 s remain.
    let mut early = IncrementalVad::new(Fake);
    early.push(&samples[..SAMPLE_RATE]).unwrap();
    assert_eq!(
        early.finish(&samples, params()).err(),
        Some(FallbackReason::Backlog)
    );
    assert_eq!(
        IncrementalVad::new(Fake).finish(&samples, params()).err(),
        Some(FallbackReason::Backlog),
        "a worker that never scored anything"
    );
}

#[test]
fn stop_never_waits_for_a_busy_session_and_reports_contended() {
    let samples = clip(20_000);
    let mut vad = IncrementalVad::new(BusyAtStop::default());
    vad.push(&samples[..10_000]).unwrap(); // recording-time scoring waits and works
    assert_eq!(
        vad.finish(&samples, params()).err(),
        Some(FallbackReason::Contended)
    );
}

/// Run `stop_vad` with a batch closure that counts its calls and returns a
/// recognisable marker.
fn run_stop<S: WindowScorer>(
    enabled: bool,
    handoff: Option<StreamHandoff<S>>,
    samples: &[f32],
) -> (Result<VadResult, String>, &'static str, usize) {
    let calls = std::cell::Cell::new(0);
    let (result, outcome, _bounds) = stop_vad(enabled, handoff, 3, samples, params(), || {
        calls.set(calls.get() + 1);
        Ok(VadResult::Speech(vec![0.5; 3]))
    });
    (result, outcome, calls.get())
}

fn is_batch_marker(result: &Result<VadResult, String>) -> bool {
    matches!(result, Ok(VadResult::Speech(kept)) if kept == &vec![0.5; 3])
}

#[test]
fn setting_off_runs_the_batch_vad_without_touching_the_slot() {
    let slot = StreamSlot::<Fake>::default();
    assert!(slot.begin_with(3, Fake));
    let (result, outcome, calls) = run_stop::<Fake>(false, None, &clip(8_000));
    assert!(is_batch_marker(&result));
    assert_eq!((outcome, calls), ("disabled", 1));
    assert_eq!(slot.consumed(3), Some(0), "the stream was not taken");
}

#[test]
fn no_stream_for_the_recording_falls_back_to_batch() {
    let (result, outcome, calls) = run_stop::<Fake>(true, None, &clip(8_000));
    assert!(is_batch_marker(&result));
    assert_eq!((outcome, calls), ("not_started", 1));
    // A different recording's stream is not this recording's either.
    let slot = StreamSlot::<Fake>::default();
    assert!(slot.begin_with(2, Fake));
    let (result, outcome, calls) = run_stop(true, slot.take(3), &clip(8_000));
    assert!(is_batch_marker(&result));
    assert_eq!((outcome, calls), ("not_started", 1));
    assert_eq!(slot.consumed(2), Some(0));
}

#[test]
fn a_stream_whose_model_failed_to_load_falls_back_to_batch() {
    let slot = StreamSlot::<Fake>::default();
    assert!(slot.begin_with(3, Fake));
    slot.mark_failed(3, FallbackReason::ModelLoad);
    let (result, outcome, calls) = run_stop(true, slot.take(3), &clip(8_000));
    assert!(is_batch_marker(&result));
    assert_eq!((outcome, calls), ("model_load", 1));
}

#[test]
fn a_busy_session_at_stop_falls_back_to_batch() {
    let samples = clip(20_000);
    let slot = StreamSlot::<BusyAtStop>::default();
    assert!(slot.begin_with(3, BusyAtStop::default()));
    assert!(slot.push(3, 0, &samples[..10_000]));
    let (result, outcome, calls) = run_stop(true, slot.take(3), &samples);
    assert!(is_batch_marker(&result));
    assert_eq!((outcome, calls), ("contended", 1));
}

#[test]
fn a_no_speech_verdict_is_confirmed_by_the_batch_vad() {
    let silence = vec![0.0_f32; 24_000];
    let slot = StreamSlot::<Fake>::default();
    assert!(slot.begin_with(3, Fake));
    assert!(slot.push(3, 0, &silence[..16_000]));
    let (result, outcome, calls) = run_stop(true, slot.take(3), &silence);
    assert!(is_batch_marker(&result), "the batch answer wins");
    assert_eq!((outcome, calls), ("no_speech_recheck", 1));
}

#[test]
fn a_trusted_stream_is_used_and_the_batch_vad_never_runs() {
    let samples = clip(40_000);
    let slot = StreamSlot::<Fake>::default();
    assert!(slot.begin_with(3, Fake));
    assert!(slot.push(3, 0, &samples[..32_000]));
    let (result, outcome, calls) = run_stop(true, slot.take(3), &samples);
    assert!(matches!(result, Ok(VadResult::Speech(_))));
    assert!(!is_batch_marker(&result));
    assert_eq!((outcome, calls), ("used", 0));
}

/// Serializes the tests that point the model store at a temporary directory.
static STORE_LOCK: Mutex<()> = Mutex::new(());

struct TempStore {
    _guard: std::sync::MutexGuard<'static, ()>,
    dir: tempfile::TempDir,
}

impl TempStore {
    fn new() -> Self {
        let guard = STORE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let dir = tempfile::tempdir().unwrap();
        *ROOT_OVERRIDE.lock_or_recover() = Some(dir.path().to_path_buf());
        *LAST_ERROR.lock_or_recover() = None;
        unload();
        Self { _guard: guard, dir }
    }

    /// A file of the pinned size whose content is not the model.
    fn install_corrupt_model(&self) -> PathBuf {
        let path = installed_model_path().unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, vec![7_u8; MODEL_SIZE_BYTES as usize]).unwrap();
        path
    }
}

impl Drop for TempStore {
    fn drop(&mut self) {
        *ROOT_OVERRIDE.lock_or_recover() = None;
        *LAST_ERROR.lock_or_recover() = None;
        let _ = &self.dir;
    }
}

#[test]
fn a_right_size_wrong_hash_model_is_deleted_at_load_and_reported() {
    let store = TempStore::new();
    let path = store.install_corrupt_model();
    assert!(model_installed(), "size alone looks installed");
    assert!(load_session().is_err());
    assert!(
        !path.exists(),
        "the bad file is deleted so it can be fetched again"
    );
    assert!(!model_installed());
    let status = model_status();
    assert_eq!(status.state, ModelState::Failed);
    assert!(status.error.is_some());
}

#[test]
fn model_status_reports_not_downloaded_downloading_ready_and_failed() {
    let _store = TempStore::new();
    assert_eq!(model_status().state, ModelState::NotDownloaded);
    set_last_error("boom");
    let failed = model_status();
    assert_eq!(
        (failed.state, failed.error.as_deref()),
        (ModelState::Failed, Some("boom"))
    );
    {
        let _flag = DownloadingFlag::set();
        assert_eq!(model_status().state, ModelState::Downloading);
    }
    let path = installed_model_path().unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, vec![0_u8; MODEL_SIZE_BYTES as usize]).unwrap();
    let ready = model_status();
    assert_eq!((ready.state, ready.error), (ModelState::Ready, None));
}

fn model_less_app() -> (tauri::App<tauri::test::MockRuntime>, impl Send) {
    crate::commands::recording::tests::partial_lease_fixture()
}

#[test]
fn starting_a_recording_never_makes_a_network_request_or_a_stream_without_the_model() {
    let store = TempStore::new();
    let (app, _lease) = model_less_app();
    let before = DOWNLOAD_ATTEMPTS.load(std::sync::atomic::Ordering::SeqCst);
    for id in 41..44 {
        assert!(!start(app.handle().clone(), id, |_, _| false));
    }
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        DOWNLOAD_ATTEMPTS.load(std::sync::atomic::Ordering::SeqCst),
        before,
        "recording start must not fetch the model"
    );
    assert_eq!(
        model_status().state,
        ModelState::NotDownloaded,
        "and it records no failure either"
    );
    assert!(!installed_model_path().unwrap().exists());
    let _ = &store;
}

#[test]
fn a_corrupt_installed_model_is_removed_by_the_first_recording_and_not_retried() {
    let store = TempStore::new();
    let (app, _lease) = model_less_app();
    let path = store.install_corrupt_model();
    let before = DOWNLOAD_ATTEMPTS.load(std::sync::atomic::Ordering::SeqCst);
    assert!(
        start(app.handle().clone(), 41, |_, _| false),
        "a stream starts for an installed file"
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while path.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!path.exists(), "the worker deleted the corrupt file");
    // The next recording does not retry the bad file and does not download.
    assert!(!start(app.handle().clone(), 42, |_, _| false));
    assert_eq!(
        DOWNLOAD_ATTEMPTS.load(std::sync::atomic::Ordering::SeqCst),
        before
    );
    assert_eq!(model_status().state, ModelState::Failed);
}

#[test]
fn incremental_resample_equals_the_whole_clip_resample() {
    use crate::audio_lifecycle::{snapshot_since, PeekSince};
    let raw: Vec<f32> = (0..100_003)
        .map(|n| ((n * 2_654_435_761_u64 as usize) % 1000) as f32 / 1000.0 - 0.5)
        .collect();
    for rate in [48_000_u32, 44_100, 32_000, 24_000, 22_050, 16_000] {
        let whole = crate::audio::resample(&raw, rate, 16_000);
        let mut produced: Vec<f32> = Vec::new();
        let mut len = 0;
        while len < raw.len() {
            len = (len + 4_801).min(raw.len());
            match snapshot_since(&raw[..len], rate, produced.len()) {
                PeekSince::Samples(new) => produced.extend(new),
                other => panic!("rate {rate}: {other:?}"),
            }
        }
        assert!(produced.len() <= whole.len(), "rate {rate}");
        assert!(
            whole.len() - produced.len() <= 2,
            "rate {rate}: only the last interpolation edge is left to Stop"
        );
        assert_eq!(&whole[..produced.len()], &produced[..], "rate {rate}");
        assert!(matches!(
            snapshot_since(&raw[..1_000], rate, 10_000_000),
            PeekSince::Reset
        ));
    }
}

// ---------------------------------------------------------------------------
// Differential run against the batch whisper.cpp VAD. Ignored by default: it
// needs the installed ggml model, the pinned ONNX model (MURMUR_E1_ONNX or the
// installed path) and fixture WAVs. Synthetic/public audio only. Output is
// content-free (clip names, numbers).
// ---------------------------------------------------------------------------

fn onnx_model_path() -> std::path::PathBuf {
    std::env::var_os("MURMUR_E1_ONNX")
        .map(std::path::PathBuf::from)
        .or_else(installed_model_path)
        .expect("pinned ONNX model path")
}

fn fixture_wavs() -> Vec<std::path::PathBuf> {
    let mut dirs = vec![std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bench/audio")];
    if let Some(extra) = std::env::var_os("MURMUR_E1_EXTRA_WAV_DIR") {
        dirs.extend(std::env::split_paths(&extra));
    }
    let mut wavs: Vec<_> = dirs
        .into_iter()
        .flat_map(|dir| std::fs::read_dir(dir).into_iter().flatten().flatten())
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "wav"))
        .collect();
    wavs.sort();
    wavs
}

fn kept_mask(len: usize, bounds: &[(usize, usize)]) -> Vec<bool> {
    let mut mask = vec![false; len];
    for &(start, end) in bounds {
        for slot in &mut mask[start.min(len)..end.min(len)] {
            *slot = true;
        }
    }
    mask
}

/// Convert a whole-clip WAV into 16 kHz mono f32, resampling other rates.
fn load_clip(path: &std::path::Path) -> Option<Vec<f32>> {
    let mut reader = hound::WavReader::open(path).ok()?;
    let spec = reader.spec();
    let channels = spec.channels as usize;
    let interleaved: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => {
            let scale = (1_i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|sample| sample.map(|value| value as f32 / scale))
                .collect::<Result<_, _>>()
                .ok()?
        }
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>().ok()?,
    };
    let mono: Vec<f32> = interleaved
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect();
    Some(crate::audio::resample(&mono, spec.sample_rate, 16_000))
}

#[test]
#[ignore = "differential run: needs the installed ggml VAD model and the pinned ONNX model"]
fn differential_incremental_vs_batch_vad() {
    use whisper_rs::{WhisperVadContext, WhisperVadContextParams, WhisperVadParams};

    let ggml = crate::vad::vad_model_path()
        .filter(|path| path.exists())
        .expect("installed ggml VAD model");
    load_session_from(&onnx_model_path()).expect("ONNX session");
    let mut context_params = WhisperVadContextParams::default();
    context_params.set_n_threads(1);
    context_params.set_use_gpu(false);
    let mut ggml_context =
        WhisperVadContext::new(ggml.to_str().unwrap(), context_params).expect("ggml context");

    // MURMUR_E1_DECODE=1 also decodes the kept audio of both paths whenever it
    // differs, with the installed Whisper tiny.en (Core ML is not required).
    // Only whether the text matches and a word-level edit count are reported.
    let mut decoder = std::env::var_os("MURMUR_E1_DECODE").and_then(|_| {
        use crate::transcriber::TranscriptionBackend;
        let mut backend = crate::transcriber::whisper::WhisperBackend::new();
        backend.load_model("tiny.en").ok().map(|()| backend)
    });
    let mut clips = 0;
    let mut identical = 0;
    let mut worst_shift_samples = 0_usize;
    let mut lost_any = false;
    let mut max_prob_diff = 0.0_f32;
    for path in fixture_wavs() {
        let Some(samples) = load_clip(&path) else {
            continue;
        };
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        // Trailing silence like a real recording stop, so tails exist.
        let mut samples = samples;
        samples.resize(samples.len() + 12_000, 0.0);
        for sensitivity in [30_u32, 50, 70] {
            let threshold = crate::vad::threshold_for_sensitivity(sensitivity);

            // Batch: probabilities and Murmur's exact segments.
            let mut batch_params = WhisperVadParams::default();
            batch_params.set_threshold(threshold);
            batch_params.set_speech_pad(crate::vad::SPEECH_PAD_MS);
            ggml_context.detect_speech(&samples).expect("ggml detect");
            let batch_probs = ggml_context.probabilities().to_vec();
            let (batch_result, batch_bounds) = crate::vad::filter_speech_with_segments(
                ggml.to_str().unwrap(),
                &samples,
                threshold,
            )
            .expect("batch VAD");

            // The segmentation port is exact on the batch's own probabilities.
            let ported: Vec<(usize, usize)> = {
                let segs = segments_from_probs(&batch_probs, SegmentParams::murmur(threshold));
                crate::vad::speech_from_segments::<true>(segs, &samples).1
            };
            assert_eq!(
                ported, batch_bounds,
                "{name} s{sensitivity}: port != whisper.cpp"
            );

            // Incremental: polled in 200 ms chunks, tail finished at "Stop".
            let mut vad = IncrementalVad::new(OnnxScorer::default());
            let polled = samples.len() - samples.len() % 3_200 - 3_200;
            for piece in samples[..polled].chunks(3_200) {
                vad.push(piece).expect("incremental push");
            }
            let probs_at_stop = vad.probs.len();
            let finished = vad
                .finish(&samples, SegmentParams::murmur(threshold))
                .expect("finish");

            // Re-run to read probabilities for the comparison.
            let mut probe = IncrementalVad::new(OnnxScorer::default());
            probe.push(&samples).unwrap();
            probe.flush().unwrap();
            let diffs: Vec<f32> = batch_probs
                .iter()
                .zip(probe.probabilities())
                .map(|(a, b)| (a - b).abs())
                .collect();
            let prob_diff = diffs.iter().copied().fold(0.0_f32, f32::max);
            let flipped = batch_probs
                .iter()
                .zip(probe.probabilities())
                .filter(|(a, b)| (**a >= threshold) != (**b >= threshold))
                .count();
            max_prob_diff = max_prob_diff.max(prob_diff);

            let stream_bounds = finished.bounds.clone();
            let shift = if stream_bounds.len() == batch_bounds.len() {
                stream_bounds
                    .iter()
                    .zip(&batch_bounds)
                    .map(|(a, b)| a.0.abs_diff(b.0).max(a.1.abs_diff(b.1)))
                    .max()
                    .unwrap_or(0)
            } else {
                usize::MAX
            };
            let batch_mask = kept_mask(samples.len(), &batch_bounds);
            let stream_mask = kept_mask(samples.len(), &stream_bounds);
            let lost = batch_mask
                .iter()
                .zip(&stream_mask)
                .filter(|(kept_by_batch, kept_by_stream)| **kept_by_batch && !**kept_by_stream)
                .count();
            let extra = batch_mask
                .iter()
                .zip(&stream_mask)
                .filter(|(kept_by_batch, kept_by_stream)| !**kept_by_batch && **kept_by_stream)
                .count();
            let same_audio = match (&batch_result, &finished.result) {
                (VadResult::Speech(a), VadResult::Speech(b)) => a == b,
                (VadResult::NoSpeech, VadResult::NoSpeech) => true,
                _ => false,
            };
            let text_check = if same_audio {
                serde_json::json!("identical_input")
            } else if let Some(backend) = decoder.as_mut() {
                use crate::transcriber::TranscriptionBackend;
                let mut decode = |result: &VadResult| -> String {
                    match result {
                        VadResult::Speech(kept) => backend
                            .transcribe(kept, "en", None, true)
                            .unwrap_or_default(),
                        VadResult::NoSpeech => String::new(),
                    }
                };
                let batch_text = decode(&batch_result);
                let stream_text = decode(&finished.result);
                // Control: the batch audio with 30 ms of silence prepended. It
                // perturbs the decoder input by the size of the largest VAD
                // difference while keeping every kept sample, so it shows how
                // much the decoder alone moves for a 30 ms change.
                let control_text = match &batch_result {
                    VadResult::Speech(kept) => {
                        let mut padded = vec![0.0_f32; 480];
                        padded.extend_from_slice(kept);
                        decode(&VadResult::Speech(padded))
                    }
                    VadResult::NoSpeech => String::new(),
                };
                // Word-level Levenshtein distance; no text leaves this block.
                let distance = |a: &str, b: &str| -> usize {
                    let a: Vec<&str> = a.split_whitespace().collect();
                    let b: Vec<&str> = b.split_whitespace().collect();
                    let mut row: Vec<usize> = (0..=b.len()).collect();
                    for (i, aw) in a.iter().enumerate() {
                        let mut prev = row[0];
                        row[0] = i + 1;
                        for (j, bw) in b.iter().enumerate() {
                            let cur = row[j + 1];
                            row[j + 1] = (prev + usize::from(aw != bw))
                                .min(row[j] + 1)
                                .min(row[j + 1] + 1);
                            prev = cur;
                        }
                    }
                    row[b.len()]
                };
                serde_json::json!({
                    "text_equal": batch_text == stream_text,
                    "word_edit_distance": distance(&batch_text, &stream_text),
                    "control_30ms_silence_edit_distance": distance(&batch_text, &control_text),
                    "batch_words": batch_text.split_whitespace().count(),
                    "stream_words": stream_text.split_whitespace().count(),
                })
            } else {
                serde_json::json!("decode_unavailable")
            };
            clips += 1;
            identical += usize::from(same_audio);
            if shift != usize::MAX {
                worst_shift_samples = worst_shift_samples.max(shift);
            }
            lost_any |= lost > 0;
            println!(
                "{}",
                serde_json::json!({
                    "clip": name, "sensitivity": sensitivity, "samples": samples.len(),
                    "windows": batch_probs.len(), "windows_scored_before_stop": probs_at_stop,
                    "max_prob_diff": prob_diff, "flipped_windows": flipped,
                    "batch_segments": batch_bounds.len(), "stream_segments": stream_bounds.len(),
                    "batch_bounds": batch_bounds, "stream_bounds": stream_bounds,
                    "max_boundary_shift_samples": if shift == usize::MAX { serde_json::Value::Null } else { shift.into() },
                    "max_boundary_shift_ms": if shift == usize::MAX { serde_json::Value::Null } else { (shift as f64 / 16.0).into() },
                    "batch_kept_samples_lost": lost, "stream_extra_kept_samples": extra,
                    "kept_audio_identical": same_audio,
                    "text": text_check,
                })
            );
        }
    }
    println!(
        "{}",
        serde_json::json!({
            "summary": true, "runs": clips, "identical_kept_audio": identical,
            "worst_boundary_shift_ms": worst_shift_samples as f64 / 16.0,
            "any_batch_speech_lost": lost_any, "max_prob_diff": max_prob_diff,
        })
    );
    assert!(clips > 0, "no fixture WAVs found");
}

#[test]
#[ignore = "memory report: needs the pinned ONNX model"]
fn session_memory_report() {
    let before = crate::resource_monitor::get_process_rss_bytes().unwrap_or(0);
    load_session_from(&onnx_model_path()).expect("ONNX session");
    let loaded = crate::resource_monitor::get_process_rss_bytes().unwrap_or(0);
    let samples = clip(16_000 * 120);
    let mut vad = IncrementalVad::new(OnnxScorer::default());
    vad.push(&samples).unwrap();
    let scored = crate::resource_monitor::get_process_rss_bytes().unwrap_or(0);
    println!(
        "{}",
        serde_json::json!({
            "rss_bytes_before_load": before, "rss_bytes_after_load": loaded,
            "rss_bytes_after_scoring_120s": scored, "windows": vad.probs.len(),
            "probability_vector_bytes": vad.probs.len() * 4,
        })
    );
}
