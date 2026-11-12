//! Incremental Silero VAD for dictation (speed workstream E1).
//!
//! The batch VAD (`vad::filter_speech`) scores the whole clip after Stop, so its
//! cost grows with the recording. With the streaming setting on, a dedicated
//! thread scores audio with Silero while it is still being recorded, and Stop
//! only scores the unfinished tail. Everything here is content-free: no audio
//! or text is logged.
//!
//! What matches the batch result exactly, and what does not:
//! - `segments_from_probs` is a line-for-line port of whisper.cpp's
//!   `whisper_vad_segments_from_probs` for the parameters Murmur uses (an
//!   unlimited maximum speech length, so that branch is absent). Given the same
//!   probabilities it returns the same segments.
//! - The probabilities come from the official Silero v5.1.2 ONNX model through
//!   ONNX Runtime, not from whisper.cpp's F16 ggml port. Per-window values can
//!   differ slightly, so a boundary can move by a window or two (measured up
//!   to 70 ms). The
//!   differential tests measure this. The batch VAD stays the default and the
//!   fallback; any doubt at Stop uses it.
//!
//! ggml reflect-pads each 512-sample window by 64 samples on both sides and
//! starts every clip from a zero LSTM state. The ONNX model takes 64 samples of
//! left context and pads the right by reflection itself, so the left context
//! fed here is the reflection of the window's own first samples, not the
//! previous audio. Probabilities then depend only on the window and the LSTM
//! state, so a prefix scored during recording equals the whole-clip prefix.
//!
//! Threading: inference runs on the `murmur-vad-stream` thread (and on the
//! Stop path's blocking worker for the tail). Nothing here runs on the
//! real-time audio callback.
//!
//! Model lifecycle: the 2.3 MB ONNX model is downloaded only when the user
//! switches the setting on or presses Download/Retry, never when a recording
//! starts; see `commands/streaming_vad.rs`.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use crate::MutexExt;

pub const SAMPLE_RATE: usize = 16_000;
/// Silero v5 scores 512-sample (32 ms) windows at 16 kHz.
pub const WINDOW_SAMPLES: usize = 512;
const CONTEXT_SAMPLES: usize = 64;
const STATE_LEN: usize = 2 * 128;
/// The capture buffer is polled at this interval while recording.
const POLL_INTERVAL: Duration = Duration::from_millis(200);
/// Every `FINGERPRINT_STRIDE`th sample feeds the buffer-identity check.
const FINGERPRINT_STRIDE: usize = 64;

pub const MODEL_FILENAME: &str = "silero_vad_v5.1.2.onnx";
/// Pinned to the immutable commit of snakers4/silero-vad tag v5.1.2 (MIT).
pub const MODEL_URL: &str = "https://raw.githubusercontent.com/snakers4/silero-vad/6478567951ae5c9979ad7b234185b5515f4be7a1/src/silero_vad/data/silero_vad.onnx";
pub const MODEL_SIZE_BYTES: u64 = 2_327_524;
pub const MODEL_SHA256: &str = "2623a2953f6ff3d2c1e61740c6cdb7168133479b267dfef114a4a3cc5bdd788f";

/// Tests point the model store at a temporary directory.
#[cfg(test)]
pub(crate) static ROOT_OVERRIDE: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Root of the hash-versioned streaming VAD model store.
pub fn models_root() -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(root) = ROOT_OVERRIDE.lock_or_recover().clone() {
        return Some(root);
    }
    dirs::data_dir().map(|dir| {
        dir.join("local-dictation")
            .join("models")
            .join("silero-vad-onnx")
    })
}

pub fn installed_model_path() -> Option<PathBuf> {
    Some(models_root()?.join(MODEL_SHA256).join(MODEL_FILENAME))
}

/// A published model of exactly the pinned size counts as installed; the full
/// SHA-256 is re-verified when the session loads.
pub fn model_installed() -> bool {
    installed_model_path()
        .and_then(|path| std::fs::metadata(path).ok())
        .is_some_and(|metadata| metadata.is_file() && metadata.len() == MODEL_SIZE_BYTES)
}

// ---------------------------------------------------------------------------
// Segmentation: a port of whisper.cpp's whisper_vad_segments_from_probs.
// ---------------------------------------------------------------------------

/// The parameters whisper.cpp's segmentation reads. Murmur sets only the
/// threshold and the padding; the others are whisper-rs's defaults.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SegmentParams {
    pub threshold: f32,
    pub min_speech_ms: i32,
    pub min_silence_ms: i32,
    pub speech_pad_ms: i32,
}

impl SegmentParams {
    pub fn murmur(threshold: f32) -> Self {
        Self {
            threshold,
            min_speech_ms: 250,
            min_silence_ms: 100,
            speech_pad_ms: crate::vad::SPEECH_PAD_MS,
        }
    }
}

fn samples_to_cs(samples: i32) -> f32 {
    // whisper.cpp: (int64_t)((samples / 16000.0) * 100.0 + 0.5), stored as float.
    (((f64::from(samples) / SAMPLE_RATE as f64) * 100.0 + 0.5) as i64) as f32
}

/// Segments as `(start, end)` in centiseconds, exactly as whisper.cpp reports
/// them for the same per-window probabilities.
pub fn segments_from_probs(probs: &[f32], params: SegmentParams) -> Vec<(f32, f32)> {
    #[derive(Clone, Copy)]
    struct Speech {
        start: i32,
        end: i32,
    }
    let sample_rate = SAMPLE_RATE as i32;
    let n_window = WINDOW_SAMPLES as i32;
    let min_silence_samples = sample_rate * params.min_silence_ms / 1000;
    let audio_length_samples = probs.len() as i32 * n_window;
    let min_speech_samples = sample_rate * params.min_speech_ms / 1000;
    let speech_pad_samples = sample_rate * params.speech_pad_ms / 1000;
    let threshold = params.threshold;
    let neg_threshold = (threshold - 0.15_f32).max(0.01);

    let mut speeches: Vec<Speech> = Vec::new();
    let mut is_speech_segment = false;
    let mut temp_end = 0_i32;
    let mut curr_speech_start = 0_i32;
    let mut has_curr_speech = false;

    for (i, &curr_prob) in probs.iter().enumerate() {
        let curr_sample = n_window * i as i32;

        // Reset temp_end when we get back to speech.
        if curr_prob >= threshold && temp_end != 0 {
            temp_end = 0;
        }
        // Start a new speech segment.
        if curr_prob >= threshold && !is_speech_segment {
            is_speech_segment = true;
            curr_speech_start = curr_sample;
            has_curr_speech = true;
            continue;
        }
        // Silence after speech.
        if curr_prob < neg_threshold && is_speech_segment {
            if temp_end == 0 {
                temp_end = curr_sample;
            }
            if curr_sample - temp_end < min_silence_samples {
                continue;
            }
            if temp_end - curr_speech_start > min_speech_samples {
                speeches.push(Speech {
                    start: curr_speech_start,
                    end: temp_end,
                });
            }
            temp_end = 0;
            is_speech_segment = false;
            has_curr_speech = false;
        }
    }

    // Still inside a speech segment at the end of the audio.
    if has_curr_speech && audio_length_samples - curr_speech_start > min_speech_samples {
        speeches.push(Speech {
            start: curr_speech_start,
            end: audio_length_samples,
        });
    }

    // Merge adjacent segments with gaps under 200 ms.
    if speeches.len() > 1 {
        let max_merge_gap_samples = sample_rate * 200 / 1000;
        let mut i = 0_usize;
        while i + 1 < speeches.len() {
            if speeches[i + 1].start - speeches[i].end < max_merge_gap_samples {
                speeches[i].end = speeches[i + 1].end;
                speeches.remove(i + 1);
            } else {
                i += 1;
            }
        }
    }

    // Double-check the minimum speech duration.
    speeches.retain(|speech| speech.end - speech.start >= min_speech_samples);

    // Pad, splitting the difference between close segments.
    let count = speeches.len();
    for i in 0..count {
        if i == 0 {
            speeches[i].start = if speeches[i].start > speech_pad_samples {
                speeches[i].start - speech_pad_samples
            } else {
                0
            };
        }
        if i + 1 < count {
            let silence_duration = speeches[i + 1].start - speeches[i].end;
            if silence_duration < 2 * speech_pad_samples {
                speeches[i].end += silence_duration / 2;
                speeches[i + 1].start = if speeches[i + 1].start > silence_duration / 2 {
                    speeches[i + 1].start - silence_duration / 2
                } else {
                    0
                };
            } else {
                speeches[i].end = if speeches[i].end + speech_pad_samples < audio_length_samples {
                    speeches[i].end + speech_pad_samples
                } else {
                    audio_length_samples
                };
                speeches[i + 1].start = if speeches[i + 1].start > speech_pad_samples {
                    speeches[i + 1].start - speech_pad_samples
                } else {
                    0
                };
            }
        } else {
            speeches[i].end = if speeches[i].end + speech_pad_samples < audio_length_samples {
                speeches[i].end + speech_pad_samples
            } else {
                audio_length_samples
            };
        }
    }

    speeches
        .into_iter()
        .map(|speech| (samples_to_cs(speech.start), samples_to_cs(speech.end)))
        .collect()
}

// ---------------------------------------------------------------------------
// Scoring.
// ---------------------------------------------------------------------------

/// Why one window could not be scored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScoreError {
    /// The shared session was busy (a load, an unload, or another recording's
    /// scorer) and the caller must not wait. Only the Stop path refuses to wait.
    Contended,
    Failed(String),
}

impl From<String> for ScoreError {
    fn from(message: String) -> Self {
        Self::Failed(message)
    }
}

impl From<&str> for ScoreError {
    fn from(message: &str) -> Self {
        Self::Failed(message.to_string())
    }
}

/// Scores one 512-sample window given the LSTM state, which it updates.
pub trait WindowScorer {
    fn score(
        &mut self,
        window: &[f32; WINDOW_SAMPLES],
        state: &mut [f32; STATE_LEN],
    ) -> Result<f32, ScoreError>;

    /// Called when the stream is finished at Stop: from then on the scorer must
    /// return `Contended` instead of waiting for shared resources.
    fn set_blocking(&mut self, _blocking: bool) {}
}

/// The 576-sample model input: ggml's left reflection of the window, then the
/// window. (The model reflect-pads its own right edge.)
fn reflected_input(window: &[f32; WINDOW_SAMPLES]) -> [f32; CONTEXT_SAMPLES + WINDOW_SAMPLES] {
    let mut input = [0.0_f32; CONTEXT_SAMPLES + WINDOW_SAMPLES];
    for (j, slot) in input[..CONTEXT_SAMPLES].iter_mut().enumerate() {
        *slot = window[CONTEXT_SAMPLES - j];
    }
    input[CONTEXT_SAMPLES..].copy_from_slice(window);
    input
}

/// The process-wide ONNX session. Loaded lazily the first time a recording
/// with the setting on starts, and dropped by [`unload`] or when the model
/// fails verification.
static SESSION: Mutex<Option<ort::session::Session>> = Mutex::new(None);

enum Verify {
    Ok,
    /// The file is readable but is not the pinned model.
    Mismatch,
    Io(String),
}

fn verify_file(path: &std::path::Path) -> Verify {
    match crate::model_digest::model_file_digest(path) {
        Ok((size, sha)) if size == MODEL_SIZE_BYTES && sha == MODEL_SHA256 => Verify::Ok,
        Ok(_) => Verify::Mismatch,
        Err(error) => Verify::Io(error.to_string()),
    }
}

fn build_session(path: &std::path::Path) -> Result<ort::session::Session, String> {
    crate::smart_turn::init_runtime()?;
    let builder_error =
        |error: ort::Error<ort::session::builder::SessionBuilder>| error.to_string();
    ort::session::Session::builder()
        .map_err(|error| error.to_string())?
        .with_optimization_level(ort::session::builder::GraphOptimizationLevel::All)
        .map_err(builder_error)?
        .with_parallel_execution(false)
        .map_err(builder_error)?
        .with_intra_threads(1)
        .map_err(builder_error)?
        .with_inter_threads(1)
        .map_err(builder_error)?
        .with_intra_op_spinning(false)
        .map_err(builder_error)?
        .commit_from_file(path)
        .map_err(|error| error.to_string())
}

/// Hash, build, then publish: the session lock is held only to publish, so an
/// unload or a scoring call never waits behind hashing or model loading.
fn load_verified(path: &std::path::Path, delete_on_mismatch: bool) -> Result<(), String> {
    let started = unload_generation();
    if SESSION.lock_or_recover().is_some() {
        return Ok(());
    }
    match verify_file(path) {
        Verify::Ok => {}
        Verify::Mismatch => {
            if delete_on_mismatch {
                // Never retry a bad file on every recording: remove it so the
                // next switch-on downloads a fresh copy, and say so in the
                // toggle's status.
                let _ = std::fs::remove_file(path);
                set_last_error("The streaming speech model failed verification and was removed. Download it again.");
            }
            return Err("model failed verification".to_string());
        }
        Verify::Io(error) => return Err(error),
    }
    let session = build_session(path)?;
    publish_session(&SESSION, started, session)
}

/// Bumped by every [`unload`]. A load that began before an unload (the model was
/// removed or the setting turned off while it hashed or built) must not publish
/// its session afterwards: that would leave a session loaded for a file that is
/// gone.
static UNLOAD_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn unload_generation() -> u64 {
    UNLOAD_GENERATION.load(std::sync::atomic::Ordering::Acquire)
}

/// Publish `value` unless an unload happened since `started`. The check and the
/// store happen under the slot lock, and [`unload`] bumps the generation under
/// the same lock, so the two cannot interleave.
fn publish_session<T>(slot: &Mutex<Option<T>>, started: u64, value: T) -> Result<(), String> {
    let mut slot = slot.lock_or_recover();
    if unload_generation() != started {
        return Err("model was unloaded while loading".to_string());
    }
    slot.get_or_insert(value);
    Ok(())
}

/// Load the installed model. On a digest mismatch the file is deleted.
pub fn load_session() -> Result<(), String> {
    if !model_installed() {
        return Err("model not installed".to_string());
    }
    load_verified(&installed_model_path().ok_or("no model directory")?, true)
}

/// [`load_session`] for an explicit model file (differential tests); never
/// deletes.
#[cfg(test)]
pub fn load_session_from(path: &std::path::Path) -> Result<(), String> {
    load_verified(path, false)
}

/// Drop the loaded session (setting turned off, model removed).
pub fn unload() {
    let mut slot = SESSION.lock_or_recover();
    UNLOAD_GENERATION.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    *slot = None;
}

/// Scores through the shared ONNX session.
pub struct OnnxScorer {
    blocking: bool,
}

impl Default for OnnxScorer {
    fn default() -> Self {
        Self { blocking: true }
    }
}

impl WindowScorer for OnnxScorer {
    fn set_blocking(&mut self, blocking: bool) {
        self.blocking = blocking;
    }

    fn score(
        &mut self,
        window: &[f32; WINDOW_SAMPLES],
        state: &mut [f32; STATE_LEN],
    ) -> Result<f32, ScoreError> {
        // The recording-time worker is a dedicated thread and may wait for a
        // window's worth of someone else's inference. The Stop path never
        // waits: contention there means "use the batch VAD".
        let mut guard = if self.blocking {
            SESSION.lock_or_recover()
        } else {
            SESSION.try_lock_or_recover().ok_or(ScoreError::Contended)?
        };
        let session = guard.as_mut().ok_or("session not loaded")?;
        let input = ort::value::Tensor::from_array((
            [1_usize, CONTEXT_SAMPLES + WINDOW_SAMPLES],
            reflected_input(window).to_vec().into_boxed_slice(),
        ))
        .map_err(|error| error.to_string())?;
        let state_in =
            ort::value::Tensor::from_array(([2_usize, 1, 128], state.to_vec().into_boxed_slice()))
                .map_err(|error| error.to_string())?;
        let sample_rate = ort::value::Tensor::from_array((
            [0_usize; 0],
            vec![SAMPLE_RATE as i64].into_boxed_slice(),
        ))
        .map_err(|error| error.to_string())?;
        let outputs = session
            .run(ort::inputs!["input" => input, "state" => state_in, "sr" => sample_rate])
            .map_err(|error| error.to_string())?;
        let (_, probability) = outputs["output"]
            .try_extract_tensor::<f32>()
            .map_err(|error| error.to_string())?;
        let probability = probability
            .first()
            .copied()
            .filter(|probability| probability.is_finite())
            .ok_or("model returned no finite probability")?;
        let (_, next_state) = outputs["stateN"]
            .try_extract_tensor::<f32>()
            .map_err(|error| error.to_string())?;
        if next_state.len() != STATE_LEN {
            return Err("unexpected LSTM state size".into());
        }
        state.copy_from_slice(next_state);
        Ok(probability)
    }
}

// ---------------------------------------------------------------------------
// Incremental scoring state.
// ---------------------------------------------------------------------------

/// Why the incremental result was not used. Every reason falls back to the
/// batch VAD.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackReason {
    /// No stream was started for this recording (setting off, VAD disabled,
    /// model not installed, or a different recording owns the slot).
    NotStarted,
    /// The ONNX session could not be loaded or verified.
    ModelLoad,
    /// Scoring a window failed.
    Inference,
    /// The session was busy and Stop does not wait for it.
    Contended,
    /// The capture buffer was reset or shrank while recording.
    BufferReset,
    /// The final PCM is shorter than what was already scored.
    LengthMismatch,
    /// The already-scored prefix is not the prefix of the final PCM.
    PrefixMismatch,
    /// Too much audio was left unscored for Stop to score it cheaply (the
    /// recording-time worker fell behind or stopped early).
    Backlog,
}

impl FallbackReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotStarted => "not_started",
            Self::ModelLoad => "model_load",
            Self::Inference => "inference",
            Self::Contended => "contended",
            Self::BufferReset => "buffer_reset",
            Self::LengthMismatch => "length_mismatch",
            Self::PrefixMismatch => "prefix_mismatch",
            Self::Backlog => "backlog",
        }
    }
}

impl From<&ScoreError> for FallbackReason {
    fn from(error: &ScoreError) -> Self {
        match error {
            ScoreError::Contended => Self::Contended,
            ScoreError::Failed(_) => Self::Inference,
        }
    }
}

/// More unscored audio than this at Stop means streaming did not keep up; the
/// batch VAD is no slower than scoring it here single-threaded.
pub const MAX_STOP_BACKLOG_SAMPLES: usize = 2 * SAMPLE_RATE;

/// Fold every `FINGERPRINT_STRIDE`th sample (by absolute index) into `hash`.
fn fold_fingerprint(mut hash: u64, base: usize, samples: &[f32]) -> u64 {
    let first = (FINGERPRINT_STRIDE - base % FINGERPRINT_STRIDE) % FINGERPRINT_STRIDE;
    for index in (first..samples.len()).step_by(FINGERPRINT_STRIDE) {
        hash = (hash ^ u64::from(samples[index].to_bits())).wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

const FINGERPRINT_SEED: u64 = 0xcbf2_9ce4_8422_2325;

/// The segments implied by the audio scored so far (see
/// [`IncrementalVad::snapshot`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamSnapshot {
    /// Padded `(start, end)` sample bounds, in order.
    pub segments: Vec<(usize, usize)>,
    /// Samples covered by whole scored windows. Never above what was pushed.
    pub scored_samples: usize,
}

/// The result of finishing a stream at Stop.
pub struct Finished {
    pub result: crate::vad::VadResult,
    /// Padded segment bounds in samples (as `vad::filter_speech_with_segments`).
    /// Decode reuse (`stream_reuse`) compares them with the segments it planned
    /// from during recording.
    pub bounds: Vec<(usize, usize)>,
    /// Windows scored at Stop (the unfinished tail, including the padded last).
    pub tail_windows: usize,
    pub total_windows: usize,
}

pub struct IncrementalVad<S: WindowScorer> {
    scorer: S,
    state: [f32; STATE_LEN],
    pending: Vec<f32>,
    probs: Vec<f32>,
    consumed: usize,
    fingerprint: u64,
}

impl<S: WindowScorer> IncrementalVad<S> {
    pub fn new(scorer: S) -> Self {
        Self {
            scorer,
            state: [0.0; STATE_LEN],
            pending: Vec::with_capacity(WINDOW_SAMPLES),
            probs: Vec::new(),
            consumed: 0,
            fingerprint: FINGERPRINT_SEED,
        }
    }

    /// 16 kHz samples handed to `push` so far.
    pub fn consumed(&self) -> usize {
        self.consumed
    }

    #[cfg(test)]
    pub fn probabilities(&self) -> &[f32] {
        &self.probs
    }

    fn score_window(&mut self, window: &[f32; WINDOW_SAMPLES]) -> Result<(), ScoreError> {
        let probability = self.scorer.score(window, &mut self.state)?;
        self.probs.push(probability);
        Ok(())
    }

    /// Score every whole window the new samples complete. Chunk size does not
    /// change the result.
    pub fn push(&mut self, samples: &[f32]) -> Result<(), ScoreError> {
        self.fingerprint = fold_fingerprint(self.fingerprint, self.consumed, samples);
        self.consumed += samples.len();
        let mut rest = samples;
        if !self.pending.is_empty() {
            let take = (WINDOW_SAMPLES - self.pending.len()).min(rest.len());
            self.pending.extend_from_slice(&rest[..take]);
            rest = &rest[take..];
            if self.pending.len() == WINDOW_SAMPLES {
                let mut window = [0.0_f32; WINDOW_SAMPLES];
                window.copy_from_slice(&self.pending);
                self.pending.clear();
                self.score_window(&window)?;
            }
        }
        while rest.len() >= WINDOW_SAMPLES {
            let mut window = [0.0_f32; WINDOW_SAMPLES];
            window.copy_from_slice(&rest[..WINDOW_SAMPLES]);
            rest = &rest[WINDOW_SAMPLES..];
            self.score_window(&window)?;
        }
        self.pending.extend_from_slice(rest);
        Ok(())
    }

    /// The segments the probabilities scored so far imply, as padded sample
    /// bounds, plus the length those probabilities cover. The last segment may
    /// still be growing: `stream_reuse` decides which are final.
    pub fn snapshot(&self, params: SegmentParams) -> StreamSnapshot {
        let scored_samples = self.probs.len() * WINDOW_SAMPLES;
        let segments = segments_from_probs(&self.probs, params)
            .into_iter()
            .map(|(start, end)| crate::vad::segment_sample_bounds(start, end, scored_samples))
            .filter(|(start, end)| start < end)
            .collect();
        StreamSnapshot {
            segments,
            scored_samples,
        }
    }

    /// Score the final partial window (zero-padded, as ggml does).
    fn flush(&mut self) -> Result<(), ScoreError> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let mut window = [0.0_f32; WINDOW_SAMPLES];
        window[..self.pending.len()].copy_from_slice(&self.pending);
        self.pending.clear();
        self.score_window(&window)
    }

    /// Finish against the authoritative final PCM: verify it extends what was
    /// already scored, score the remaining tail without waiting for shared
    /// resources, and segment.
    pub fn finish(
        mut self,
        samples: &[f32],
        params: SegmentParams,
    ) -> Result<Finished, FallbackReason> {
        if samples.len() < self.consumed {
            return Err(FallbackReason::LengthMismatch);
        }
        if samples.len() - self.consumed > MAX_STOP_BACKLOG_SAMPLES {
            return Err(FallbackReason::Backlog);
        }
        if fold_fingerprint(FINGERPRINT_SEED, 0, &samples[..self.consumed]) != self.fingerprint {
            return Err(FallbackReason::PrefixMismatch);
        }
        self.scorer.set_blocking(false);
        let windows_before = self.probs.len();
        self.push(&samples[self.consumed..])
            .and_then(|()| self.flush())
            .map_err(|error| FallbackReason::from(&error))?;
        if self.probs.len() != samples.len().div_ceil(WINDOW_SAMPLES) {
            return Err(FallbackReason::LengthMismatch);
        }
        let segments = segments_from_probs(&self.probs, params);
        let (result, bounds) = if segments.is_empty() {
            (crate::vad::VadResult::NoSpeech, Vec::new())
        } else {
            crate::vad::speech_from_segments::<true>(segments, samples)
        };
        Ok(Finished {
            result,
            bounds,
            tail_windows: self.probs.len() - windows_before,
            total_windows: self.probs.len(),
        })
    }
}

// ---------------------------------------------------------------------------
// The per-recording slot.
// ---------------------------------------------------------------------------

/// Samples scored per slot-lock hold (about 16 windows, a few milliseconds), so
/// Stop's `take` waits for one chunk rather than a whole poll's backlog.
const PUSH_CHUNK_SAMPLES: usize = SAMPLE_RATE / 2;

struct ActiveStream<S: WindowScorer> {
    recording_id: u64,
    vad: IncrementalVad<S>,
    failed: Option<FallbackReason>,
}

struct SlotInner<S: WindowScorer> {
    active: Option<ActiveStream<S>>,
    /// Recording ids only grow, so a duplicate or stale `begin` is refused.
    highest_begun: u64,
}

/// Holds the one incremental VAD that belongs to the current recording.
/// Every method is keyed by recording id; another recording's id never reads
/// or changes the state. `S` is the scorer; production uses [`OnnxScorer`].
pub struct StreamSlot<S: WindowScorer = OnnxScorer> {
    inner: Mutex<SlotInner<S>>,
    /// Recording id whose `take` is waiting for the lock; its worker yields.
    taking_id: std::sync::atomic::AtomicU64,
}

impl<S: WindowScorer> Default for StreamSlot<S> {
    fn default() -> Self {
        Self {
            inner: Mutex::new(SlotInner {
                active: None,
                highest_begun: 0,
            }),
            taking_id: std::sync::atomic::AtomicU64::new(0),
        }
    }
}

impl StreamSlot<OnnxScorer> {
    /// Start a fresh stream for `recording_id`. Returns false, changing
    /// nothing, when this id (or a newer one) has already begun.
    pub fn begin(&self, recording_id: u64) -> bool {
        self.begin_with(recording_id, OnnxScorer::default())
    }
}

impl<S: WindowScorer> StreamSlot<S> {
    /// [`StreamSlot::begin`] with an explicit scorer.
    pub fn begin_with(&self, recording_id: u64, scorer: S) -> bool {
        let mut inner = self.inner.lock_or_recover();
        if recording_id <= inner.highest_begun {
            return false;
        }
        inner.highest_begun = recording_id;
        inner.active = Some(ActiveStream {
            recording_id,
            vad: IncrementalVad::new(scorer),
            failed: None,
        });
        true
    }

    /// Samples scored so far, or `None` when `recording_id` does not own the
    /// slot or its stream has failed.
    pub fn consumed(&self, recording_id: u64) -> Option<usize> {
        self.inner
            .lock_or_recover()
            .active
            .as_ref()
            .filter(|stream| stream.recording_id == recording_id && stream.failed.is_none())
            .map(|stream| stream.vad.consumed())
    }

    /// Score `samples`, which must start at `expected_offset`, in bounded
    /// chunks, releasing the slot lock between them. Returns false when the
    /// stream no longer belongs to `recording_id`, failed, or Stop is taking it.
    pub fn push(&self, recording_id: u64, expected_offset: usize, samples: &[f32]) -> bool {
        use std::sync::atomic::Ordering;
        let mut offset = expected_offset;
        for chunk in samples.chunks(PUSH_CHUNK_SAMPLES) {
            if self.taking_id.load(Ordering::Acquire) == recording_id {
                return false;
            }
            let mut inner = self.inner.lock_or_recover();
            let Some(stream) = inner
                .active
                .as_mut()
                .filter(|stream| stream.recording_id == recording_id && stream.failed.is_none())
            else {
                return false;
            };
            if stream.vad.consumed() != offset {
                stream.failed = Some(FallbackReason::BufferReset);
                return false;
            }
            if let Err(error) = stream.vad.push(chunk) {
                stream.failed = Some(FallbackReason::from(&error));
                return false;
            }
            offset += chunk.len();
        }
        true
    }

    /// The segments scored so far, or `None` when `recording_id` does not own
    /// the slot or its stream has failed.
    pub fn snapshot(&self, recording_id: u64, params: SegmentParams) -> Option<StreamSnapshot> {
        self.inner
            .lock_or_recover()
            .active
            .as_ref()
            .filter(|stream| stream.recording_id == recording_id && stream.failed.is_none())
            .map(|stream| stream.vad.snapshot(params))
    }

    pub fn mark_failed(&self, recording_id: u64, reason: FallbackReason) {
        if let Some(stream) = self
            .inner
            .lock_or_recover()
            .active
            .as_mut()
            .filter(|stream| stream.recording_id == recording_id)
        {
            stream.failed.get_or_insert(reason);
        }
    }

    /// Remove and return the stream for exactly `recording_id`.
    pub fn take(&self, recording_id: u64) -> Option<StreamHandoff<S>> {
        use std::sync::atomic::Ordering;
        // Ask this recording's worker to let go between chunks.
        self.taking_id.store(recording_id, Ordering::Release);
        let mut inner = self.inner.lock_or_recover();
        let handoff = if inner
            .active
            .as_ref()
            .is_some_and(|stream| stream.recording_id == recording_id)
        {
            inner.active.take().map(|stream| StreamHandoff {
                vad: stream.vad,
                failed: stream.failed,
            })
        } else {
            None
        };
        drop(inner);
        self.taking_id.store(0, Ordering::Release);
        handoff
    }

    /// Drop the stream owned by `recording_id` (a cancelled or ended recording).
    pub fn clear(&self, recording_id: u64) {
        let mut inner = self.inner.lock_or_recover();
        if inner
            .active
            .as_ref()
            .is_some_and(|stream| stream.recording_id == recording_id)
        {
            inner.active = None;
        }
    }
}

/// A stream taken out of the slot at Stop, ready to finish on any thread.
pub struct StreamHandoff<S: WindowScorer = OnnxScorer> {
    vad: IncrementalVad<S>,
    failed: Option<FallbackReason>,
}

impl<S: WindowScorer> StreamHandoff<S> {
    pub fn finish(
        self,
        samples: &[f32],
        params: SegmentParams,
    ) -> Result<Finished, FallbackReason> {
        if let Some(reason) = self.failed {
            return Err(reason);
        }
        self.vad.finish(samples, params)
    }
}

/// Start incremental scoring for `recording_id`: reset the slot synchronously,
/// then poll the capture buffer on a dedicated thread until the recording is no
/// longer current. `is_current` is the caller's recording-generation check.
///
/// Returns false and starts nothing when the model is not installed, or when
/// this recording already started a worker. It NEVER downloads: fetching the
/// model is only ever the user's action (see [`download_model`]), so starting a
/// recording makes no network request.
pub fn start<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    recording_id: u64,
    is_current: impl Fn(&crate::state::AppState, u64) -> bool + Send + 'static,
) -> bool {
    use tauri::Manager;
    if !model_installed() {
        return false;
    }
    if !app
        .state::<crate::State>()
        .app_state
        .vad_stream
        .begin(recording_id)
    {
        return false;
    }
    let cleanup_app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("murmur-vad-stream".to_string())
        .spawn(move || {
            let state = app.state::<crate::State>();
            let slot = &state.app_state.vad_stream;
            if load_session().is_err() {
                slot.mark_failed(recording_id, FallbackReason::ModelLoad);
                return;
            }
            loop {
                std::thread::sleep(POLL_INTERVAL);
                if !is_current(&state.app_state, recording_id) {
                    break;
                }
                let Some(offset) = slot.consumed(recording_id) else {
                    break;
                };
                match crate::audio_lifecycle::peek_dictation_samples_since(recording_id, offset) {
                    crate::audio_lifecycle::PeekSince::Samples(samples) => {
                        if !samples.is_empty() && !slot.push(recording_id, offset, &samples) {
                            break;
                        }
                    }
                    // A slow or missed supervisor reply is retried; the
                    // currency check above ends the loop once the recording
                    // is really over. Stop falls back if too much is left.
                    crate::audio_lifecycle::PeekSince::NotCurrent => {}
                    crate::audio_lifecycle::PeekSince::Reset => {
                        slot.mark_failed(recording_id, FallbackReason::BufferReset);
                        break;
                    }
                }
            }
        });
    if spawned.is_err() {
        cleanup_app
            .state::<crate::State>()
            .app_state
            .vad_stream
            .clear(recording_id);
        return false;
    }
    true
}

fn emit_outcome(
    recording_id: u64,
    outcome: &'static str,
    tail_windows: usize,
    total_windows: usize,
    finish_ms: u64,
) {
    tracing::info!(
        target: "pipeline",
        event_code = "pipeline.vad_stream_outcome",
        recording_id,
        outcome,
        tail_windows = tail_windows as u64,
        total_windows = total_windows as u64,
        finish_ms,
        "streaming VAD outcome"
    );
}

/// What [`stop_vad`] decided: the VAD result, the outcome label, and the padded
/// segment bounds when the incremental result produced the kept audio.
pub type StopVad = (
    Result<crate::vad::VadResult, String>,
    &'static str,
    Option<Vec<(usize, usize)>>,
);

/// The Stop-time VAD decision. `batch` is the unchanged batch VAD.
///
/// - Setting off (`enabled == false`): run `batch` and emit nothing.
/// - Streaming on: finish the incremental result when it can be trusted;
///   otherwise run `batch` (no stream, failed stream, mismatch, backlog,
///   contention) and report why. A "no speech" verdict is also confirmed by
///   `batch`, because a silent verdict ends the dictation.
///
/// Returns the VAD result, the outcome label, and the padded segment bounds
/// when the incremental result produced the kept audio. `None` means the batch
/// VAD did, so anything `stream_reuse` planned from the incremental segments
/// cannot be proven to match it (decode reuse then falls back to the
/// whole-clip decode). Blocking; run it on a blocking worker.
pub fn stop_vad<S: WindowScorer>(
    enabled: bool,
    handoff: Option<StreamHandoff<S>>,
    recording_id: u64,
    samples: &[f32],
    params: SegmentParams,
    batch: impl FnOnce() -> Result<crate::vad::VadResult, String>,
) -> StopVad {
    if !enabled {
        return (batch(), "disabled", None);
    }
    let started = std::time::Instant::now();
    let Some(handoff) = handoff else {
        let outcome = FallbackReason::NotStarted.as_str();
        emit_outcome(recording_id, outcome, 0, 0, 0);
        return (batch(), outcome, None);
    };
    match handoff.finish(samples, params) {
        Ok(finished) => {
            let finish_ms = started.elapsed().as_millis() as u64;
            if matches!(finished.result, crate::vad::VadResult::NoSpeech) {
                emit_outcome(
                    recording_id,
                    "no_speech_recheck",
                    finished.tail_windows,
                    finished.total_windows,
                    finish_ms,
                );
                return (batch(), "no_speech_recheck", None);
            }
            emit_outcome(
                recording_id,
                "used",
                finished.tail_windows,
                finished.total_windows,
                finish_ms,
            );
            (Ok(finished.result), "used", Some(finished.bounds))
        }
        Err(reason) => {
            emit_outcome(recording_id, reason.as_str(), 0, 0, 0);
            (batch(), reason.as_str(), None)
        }
    }
}

// ---------------------------------------------------------------------------
// Model lifecycle (2.3 MB, pinned, verified). The download is ONLY ever started
// by the user: switching the setting on, or pressing Download/Retry.
// ---------------------------------------------------------------------------

static DOWNLOAD_LOCK: std::sync::LazyLock<tokio::sync::Mutex<()>> =
    std::sync::LazyLock::new(|| tokio::sync::Mutex::new(()));
static DOWNLOADING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static LAST_ERROR: Mutex<Option<String>> = Mutex::new(None);
/// Network fetches started in this process (tests assert recording start makes
/// none).
#[cfg(test)]
pub(crate) static DOWNLOAD_ATTEMPTS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

fn set_last_error(message: &str) {
    *LAST_ERROR.lock_or_recover() = Some(message.to_string());
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ModelState {
    NotDownloaded,
    Downloading,
    Ready,
    /// The last download or verification failed; retry downloads again.
    Failed,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    pub state: ModelState,
    pub size_bytes: u64,
    pub error: Option<String>,
}

pub fn model_status() -> ModelStatus {
    use std::sync::atomic::Ordering;
    let error = LAST_ERROR.lock_or_recover().clone();
    let state = if DOWNLOADING.load(Ordering::Acquire) {
        ModelState::Downloading
    } else if model_installed() {
        ModelState::Ready
    } else if error.is_some() {
        ModelState::Failed
    } else {
        ModelState::NotDownloaded
    };
    ModelStatus {
        state,
        size_bytes: MODEL_SIZE_BYTES,
        error: (state == ModelState::Failed).then_some(error).flatten(),
    }
}

struct DownloadingFlag;
impl DownloadingFlag {
    fn set() -> Self {
        DOWNLOADING.store(true, std::sync::atomic::Ordering::Release);
        Self
    }
}
impl Drop for DownloadingFlag {
    fn drop(&mut self) {
        DOWNLOADING.store(false, std::sync::atomic::Ordering::Release);
    }
}

/// Download, verify and publish the pinned model (user-initiated). Uses the
/// shared resumable `model_download::transfer` (idle timeout, free-space
/// preflight). A right-size file is re-hashed first, and a wrong one is
/// replaced. Records the failure for the toggle's status.
pub async fn download_model() -> Result<(), String> {
    let _single_flight = DOWNLOAD_LOCK.lock().await;
    let _flag = DownloadingFlag::set();
    let result = download_model_locked().await;
    match &result {
        Ok(()) => *LAST_ERROR.lock_or_recover() = None,
        Err(error) => set_last_error(error),
    }
    result
}

async fn download_model_locked() -> Result<(), String> {
    let root = models_root().ok_or("Could not determine the model directory")?;
    if model_installed() {
        let path = installed_model_path().ok_or("Could not determine the model path")?;
        let verified = tokio::task::spawn_blocking({
            let path = path.clone();
            move || verify_file(&path)
        })
        .await
        .map_err(|_| "Verification task failed".to_string())?;
        match verified {
            Verify::Ok => return Ok(()),
            Verify::Mismatch | Verify::Io(_) => {
                let _ = tokio::fs::remove_file(&path).await;
            }
        }
    }
    #[cfg(test)]
    DOWNLOAD_ATTEMPTS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    tokio::fs::create_dir_all(&root)
        .await
        .map_err(|error| format!("Could not create the model directory: {error}"))?;
    let partial = root.join(format!("{MODEL_FILENAME}.partial"));
    crate::model_download::transfer(
        MODEL_URL,
        &partial,
        &crate::model_download::Cancellation::default(),
        crate::model_download::IDLE_TIMEOUT,
        MODEL_SIZE_BYTES,
        |_, _| {},
    )
    .await?;
    // Keep an interrupted partial for resume, but never a verified-bad one.
    let verified = tokio::task::spawn_blocking({
        let partial = partial.clone();
        move || verify_file(&partial)
    })
    .await
    .map_err(|_| "Verification task failed".to_string())?;
    let resume = crate::model_download::resume_path(&partial);
    match verified {
        Verify::Ok => {}
        Verify::Mismatch | Verify::Io(_) => {
            let _ = tokio::fs::remove_file(&partial).await;
            let _ = tokio::fs::remove_file(&resume).await;
            return Err(
                "The downloaded model failed verification. Retry the download.".to_string(),
            );
        }
    }
    let final_dir = root.join(MODEL_SHA256);
    tokio::fs::create_dir_all(&final_dir)
        .await
        .map_err(|error| format!("Could not create the model version directory: {error}"))?;
    tokio::fs::rename(&partial, final_dir.join(MODEL_FILENAME))
        .await
        .map_err(|error| format!("Could not publish the model: {error}"))?;
    let _ = tokio::fs::remove_file(&resume).await;
    tracing::info!(target: "system", size_bytes = MODEL_SIZE_BYTES, "streaming_vad_model_installed");
    Ok(())
}

/// Release the session and delete the model and any partial download.
pub async fn remove_model() -> Result<(), String> {
    let _single_flight = DOWNLOAD_LOCK.lock().await;
    tokio::task::spawn_blocking(unload)
        .await
        .map_err(|_| "Failed to unload the streaming speech model".to_string())?;
    let root = models_root().ok_or("Could not determine the model directory")?;
    let final_dir = root.join(MODEL_SHA256);
    if final_dir.exists() {
        tokio::fs::remove_dir_all(&final_dir)
            .await
            .map_err(|error| format!("Failed to remove the streaming speech model: {error}"))?;
    }
    let partial = root.join(format!("{MODEL_FILENAME}.partial"));
    let _ = tokio::fs::remove_file(crate::model_download::resume_path(&partial)).await;
    let _ = tokio::fs::remove_file(&partial).await;
    *LAST_ERROR.lock_or_recover() = None;
    tracing::info!(target: "system", "streaming_vad_model_removed");
    Ok(())
}

#[cfg(test)]
#[path = "vad_stream_tests.rs"]
mod tests;
