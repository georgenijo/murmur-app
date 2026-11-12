//! Decode while speaking (speed workstream E2).
//!
//! With `streamingPipelineEnabled` on, the incremental VAD (`vad_stream`) tells
//! us, while the user is still talking, which stretches of audio are speech and
//! where the pauses are. At each long enough pause this module has the live
//! preview tick decode the speech since the last cut and remembers the text.
//! At Stop only the audio after the last cut is decoded; the remembered text
//! and the tail text are stitched in front of the ordered transform pipeline,
//! so Stop no longer pays for the whole clip.
//!
//! Everything a piece is decoded from is **VAD-trimmed**: a piece is a run of
//! whole VAD segments (padded bounds, concatenated), the exact samples the
//! whole-clip decode would see for that stretch. The final decode is the last
//! committed piece (its overlap) plus whatever speech came after it.
//!
//! The stitched text is never trusted on its own. At Stop it must be provable
//! that it covers all kept audio, otherwise the unchanged whole-clip decode
//! runs ([`FinalPlan`] is only produced when every check below passes):
//!
//! - The segments committed during recording equal the first segments of the
//!   final VAD result, and the final kept audio is exactly the concatenation of
//!   the final segments (coverage: nothing kept is left undecoded).
//! - The last piece's samples are bit-identical to the same stretch of the
//!   final kept audio (a full hash, not a sample of it).
//! - The tail decode starts with exactly the text the overlap piece produced
//!   (same case, punctuation and word boundary). The overlap is re-decoded
//!   with the audio that follows it, so this shows the stitch point reads the
//!   same with and without the right context, the second biggest accuracy risk.
//!   Any difference means whole-clip decode.
//! - Recording generation, model and settings come from the recording-start
//!   snapshot, and every method is keyed by recording id: a stopped, cancelled
//!   or superseded recording never reads or writes another one's state.
//!
//! Nothing here holds a lock across inference, and nothing logs audio or text.

use crate::vad_stream::StreamSnapshot;

/// Padded `(start, end)` sample bounds of one VAD segment in the raw recording.
pub(crate) type Bounds = (usize, usize);

const SAMPLE_RATE: usize = 16_000;
/// A piece commits once it holds at least this much new speech.
const MIN_NEW_AUDIO: usize = 3 * SAMPLE_RATE;
/// Overlap plus new audio in one decode, and the final tail decode, never
/// exceed this (the old preview window; one Core ML window is about 15 s).
pub(crate) const MAX_WINDOW: usize = 20 * SAMPLE_RATE;
const MAX_RECORDING: usize = 10 * 60 * SAMPLE_RATE;
/// Padded silence between two segments before a cut may fall between them.
const MIN_PAUSE: usize = 4_800;
/// With no later segment yet, a cut needs this much silence after the segment.
/// This also makes the segment provably final: a segment still pending in the
/// VAD (shorter than its 250 ms minimum) starts within the last 4,000 samples,
/// and merging or splitting padding needs it within 3,840 of the segment's end.
const EDGE_MARGIN: usize = 9_600;
/// A piece whose new text has fewer words than this is not committed yet.
const MIN_SUFFIX_WORDS: usize = 3;

/// Why reuse is off for a recording, or why Stop fell back to the whole clip.
/// Content-free; becomes the `outcome` of `pipeline.stream_reuse_outcome`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reason {
    /// No session: the recording was not eligible (setting off, model is not
    /// Core ML, no incremental VAD for it).
    Inactive,
    /// Nothing was committed (short dictation or no qualifying pause).
    NoCommit,
    DecodeError,
    /// A re-decoded overlap did not read the same as the committed text.
    OverlapMismatch,
    /// Continuous speech never paused within one decode window.
    WindowCap,
    /// The retained capture buffer was reset or could not be read.
    BufferReset,
    /// Cancelled, superseded, or a worker panicked.
    Superseded,
    LongRecording,
    /// The final VAD came from the batch path, so segments cannot be compared.
    VadNotIncremental,
    /// The committed segments are not the start of the final VAD result.
    BoundsMismatch,
    /// The last piece's samples differ from the final kept audio.
    AudioMismatch,
    TailTooLong,
    /// A preview decode still held the lease when Stop took the plan.
    Busy,
    /// The tail decode did not start with the committed overlap text.
    StitchMismatch,
    TailDecodeError,
}

impl Reason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Inactive => "inactive",
            Self::NoCommit => "no_commit",
            Self::DecodeError => "decode_error",
            Self::OverlapMismatch => "overlap_mismatch",
            Self::WindowCap => "window_cap",
            Self::BufferReset => "buffer_reset",
            Self::Superseded => "superseded",
            Self::LongRecording => "long_recording",
            Self::VadNotIncremental => "vad_not_incremental",
            Self::BoundsMismatch => "bounds_mismatch",
            Self::AudioMismatch => "audio_mismatch",
            Self::TailTooLong => "tail_too_long",
            Self::Busy => "busy",
            Self::StitchMismatch => "stitch_mismatch",
            Self::TailDecodeError => "tail_decode_error",
        }
    }
}

/// FNV-1a over the exact bits of every sample.
fn hash_samples(samples: &[f32]) -> u64 {
    samples.iter().fold(0xcbf2_9ce4_8422_2325, |hash, sample| {
        (hash ^ u64::from(sample.to_bits())).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn segment_len((start, end): Bounds) -> usize {
    end.saturating_sub(start)
}

fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

fn join(a: &str, b: &str) -> String {
    if a.is_empty() {
        b.to_string()
    } else if b.is_empty() {
        a.to_string()
    } else {
        format!("{a} {b}")
    }
}

/// `decoded` minus a leading `overlap`, only if the overlap is exactly there
/// (case, punctuation) and ends on a word boundary.
fn split_overlap<'a>(decoded: &'a str, overlap: &str) -> Option<&'a str> {
    let rest = decoded.strip_prefix(overlap)?;
    if rest.is_empty() || rest.starts_with(char::is_whitespace) {
        Some(rest.trim_start())
    } else {
        None
    }
}

#[derive(Default)]
pub(crate) struct StreamReuse {
    id: u64,
    failed: Option<Reason>,
    /// Segments committed so far (raw padded bounds); a prefix of the final VAD.
    committed_bounds: Vec<Bounds>,
    /// Trimmed samples the committed pieces cover.
    trimmed_len: usize,
    /// Trimmed index where the last committed piece's new audio starts.
    last_start: usize,
    /// That piece's new audio: the overlap of the next decode. At most one
    /// window, so memory does not grow with the recording.
    last_piece: Vec<f32>,
    last_hash: u64,
    /// Text of every piece before the last one.
    prefix: String,
    /// Text of the last committed piece, the exact overlap of the next decode.
    last_text: String,
    pieces: u32,
    /// Segment index below which no cut is tried (after a too-short piece).
    floor: usize,
}

/// A piece to decode: overlap audio followed by the new segments' audio.
#[derive(Clone)]
pub(crate) struct CommitPlan {
    id: u64,
    first: usize,
    last: usize,
    /// Where the new segments start in the raw recording (the peek offset).
    pub raw_start: usize,
    segments: Vec<Bounds>,
    overlap: Vec<f32>,
    new_len: usize,
}

impl CommitPlan {
    /// The decoder input, given the raw samples from `raw_start` onward.
    /// `None` when `raw` does not reach the end of the last new segment.
    pub fn piece(&self, raw: &[f32]) -> Option<Vec<f32>> {
        let mut piece = Vec::with_capacity(self.overlap.len() + self.new_len);
        piece.extend_from_slice(&self.overlap);
        for &(start, end) in &self.segments {
            piece.extend_from_slice(raw.get(start - self.raw_start..end - self.raw_start)?);
        }
        Some(piece)
    }
}

pub(crate) enum Plan {
    /// This recording has no (or no longer a) reuse session.
    Off,
    /// Nothing to commit yet.
    Idle,
    Commit(CommitPlan),
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Committed {
    Stored,
    /// Too few new words: wait for a later cut instead of committing.
    Deferred,
    /// The session ended (see [`Reason`]) or was not this recording's.
    Off,
}

/// What Stop may reuse, once every proof has passed.
#[derive(Debug)]
pub(crate) struct FinalPlan {
    /// Index into the final kept audio where the tail decode starts.
    pub start: usize,
    prefix: String,
    overlap: String,
    complete: bool,
    pub pieces: u32,
    pub overlap_words: usize,
}

impl FinalPlan {
    /// The committed pieces already cover all kept audio: nothing to decode.
    pub fn complete_text(&self) -> Option<String> {
        self.complete.then(|| join(&self.prefix, &self.overlap))
    }

    /// Stitch the decoded tail onto the committed text. Exact case and
    /// punctuation and a word boundary are deliberate: any contextual rewrite
    /// of the overlap requires whole-clip ASR instead.
    pub fn stitch(&self, decoded: &str) -> Option<String> {
        if self.complete {
            return None;
        }
        if split_overlap(decoded.trim(), &self.overlap)?.is_empty() {
            return None;
        }
        Some(join(&self.prefix, decoded.trim()))
    }
}

impl StreamReuse {
    pub fn begin(&mut self, id: u64) {
        if id > self.id {
            *self = Self {
                id,
                ..Self::default()
            };
        }
    }

    /// The recording ended or was cancelled; its state is never reused.
    pub fn clear(&mut self, id: u64) {
        if self.id == id {
            *self = Self {
                id,
                failed: Some(Reason::Superseded),
                ..Self::default()
            };
        }
    }

    pub fn fail(&mut self, id: u64, reason: Reason) {
        if self.id == id && self.failed.is_none() {
            *self = Self {
                id,
                failed: Some(reason),
                ..Self::default()
            };
        }
    }

    pub fn is_active(&self, id: u64) -> bool {
        id != 0 && self.id == id && self.failed.is_none()
    }

    /// Decide whether a piece can be committed now, from the VAD's current
    /// segments. A cut falls after a segment that is final (a later segment
    /// exists, or enough silence has passed) followed by a real pause, once the
    /// piece holds at least [`MIN_NEW_AUDIO`] of new speech.
    pub fn plan(&mut self, id: u64, snapshot: &StreamSnapshot) -> Plan {
        if !self.is_active(id) {
            return Plan::Off;
        }
        let segments = &snapshot.segments;
        if snapshot.scored_samples > MAX_RECORDING {
            self.fail(id, Reason::LongRecording);
            return Plan::Off;
        }
        let committed = self.committed_bounds.len();
        if segments.len() < committed || segments[..committed] != self.committed_bounds[..] {
            // Committed segments changed under us: the VAD disagrees with what
            // was decoded, so nothing here can be trusted.
            self.fail(id, Reason::BoundsMismatch);
            return Plan::Off;
        }
        let mut new_len = 0;
        for index in committed..segments.len() {
            new_len += segment_len(segments[index]);
            if index < self.floor || new_len < MIN_NEW_AUDIO {
                continue;
            }
            let gap = match segments.get(index + 1) {
                Some(next) => next.0.saturating_sub(segments[index].1),
                None => snapshot.scored_samples.saturating_sub(segments[index].1),
            };
            let is_final = index + 1 < segments.len() || gap >= EDGE_MARGIN;
            if !is_final || gap < MIN_PAUSE {
                continue;
            }
            if self.last_piece.len() + new_len > MAX_WINDOW {
                self.fail(id, Reason::WindowCap);
                return Plan::Off;
            }
            return Plan::Commit(CommitPlan {
                id,
                first: committed,
                last: index,
                raw_start: segments[committed].0,
                segments: segments[committed..=index].to_vec(),
                overlap: self.last_piece.clone(),
                new_len,
            });
        }
        // Speech that has not paused within one window can never be cut.
        if self.last_piece.len() + new_len > MAX_WINDOW {
            self.fail(id, Reason::WindowCap);
            return Plan::Off;
        }
        Plan::Idle
    }

    /// Record the decode of `plan`'s piece. `piece` is what `CommitPlan::piece`
    /// built and `decoded` its text. The re-decoded overlap must read exactly
    /// as the text committed before; anything else ends reuse for this
    /// recording (Stop then decodes the whole clip).
    pub fn commit(
        &mut self,
        id: u64,
        plan: CommitPlan,
        mut piece: Vec<f32>,
        decoded: &str,
    ) -> Committed {
        if !self.is_active(id) || plan.id != id {
            return Committed::Off;
        }
        if plan.first != self.committed_bounds.len()
            || piece.len() != plan.overlap.len() + plan.new_len
            || plan.overlap != self.last_piece
        {
            self.fail(id, Reason::BoundsMismatch);
            return Committed::Off;
        }
        let decoded = decoded.trim();
        let suffix = if self.last_text.is_empty() {
            Some(decoded)
        } else {
            split_overlap(decoded, &self.last_text)
        };
        let Some(suffix) = suffix else {
            self.fail(id, Reason::OverlapMismatch);
            return Committed::Off;
        };
        if word_count(suffix) < MIN_SUFFIX_WORDS {
            self.floor = plan.last + 1;
            return Committed::Deferred;
        }
        let new_audio = piece.split_off(plan.overlap.len());
        self.prefix = join(&self.prefix, &self.last_text);
        self.last_text = suffix.to_string();
        self.last_start = self.trimmed_len;
        self.trimmed_len += plan.new_len;
        self.last_hash = hash_samples(&new_audio);
        self.last_piece = new_audio;
        self.committed_bounds.extend_from_slice(&plan.segments);
        self.floor = plan.last + 1;
        self.pieces += 1;
        Committed::Stored
    }

    /// Consume this recording's session once, at Stop. `final_bounds` are the
    /// final VAD's padded segment bounds (`None`: the batch VAD produced the
    /// kept audio, so nothing planned from the incremental segments can be
    /// proven) and `filtered` the kept audio built from them. Returns the
    /// proof-carrying plan, or why the whole clip must be decoded. A stale
    /// recording id changes nothing.
    pub fn take_final(
        &mut self,
        id: u64,
        final_bounds: Option<&[Bounds]>,
        filtered: &[f32],
    ) -> Result<FinalPlan, Reason> {
        if self.id != id || id == 0 {
            return Err(Reason::Inactive);
        }
        let session = std::mem::replace(
            self,
            Self {
                id,
                failed: Some(Reason::Superseded),
                ..Self::default()
            },
        );
        if let Some(reason) = session.failed {
            return Err(reason);
        }
        let committed = session.committed_bounds.len();
        if committed == 0 || session.last_text.is_empty() {
            return Err(Reason::NoCommit);
        }
        if filtered.len() > MAX_RECORDING {
            return Err(Reason::LongRecording);
        }
        let final_bounds = final_bounds.ok_or(Reason::VadNotIncremental)?;
        // Coverage: the kept audio is exactly the final segments, and the
        // segments decoded during recording are the first of them.
        if final_bounds.len() < committed
            || final_bounds[..committed] != session.committed_bounds[..]
            || final_bounds.iter().map(|b| segment_len(*b)).sum::<usize>() != filtered.len()
            || session.trimmed_len > filtered.len()
        {
            return Err(Reason::BoundsMismatch);
        }
        let last_piece = filtered
            .get(session.last_start..session.trimmed_len)
            .ok_or(Reason::BoundsMismatch)?;
        if hash_samples(last_piece) != session.last_hash {
            return Err(Reason::AudioMismatch);
        }
        let complete = filtered.len() == session.trimmed_len;
        if !complete && filtered.len() - session.last_start > MAX_WINDOW {
            return Err(Reason::TailTooLong);
        }
        let overlap_words = word_count(&session.last_text);
        Ok(FinalPlan {
            start: session.last_start,
            prefix: session.prefix,
            overlap: session.last_text,
            complete,
            pieces: session.pieces,
            overlap_words,
        })
    }
}

/// Content-free outcome of the Stop-time decision: the reason or `reused`.
/// Emitted once per recording whose setting was on.
pub(crate) struct Outcome {
    pub code: &'static str,
    pub pieces: u32,
    /// Audio decoded at Stop, in milliseconds (the whole clip's length on a
    /// fallback, zero when the committed pieces covered everything).
    pub decoded_ms: u64,
    pub overlap_words: u64,
}

pub(crate) fn emit_outcome(recording_id: u64, outcome: &Outcome) {
    tracing::info!(
        target: "pipeline",
        event_code = "pipeline.stream_reuse_outcome",
        recording_id,
        outcome = outcome.code,
        piece_count = u64::from(outcome.pieces),
        decoded_ms = outcome.decoded_ms,
        overlap_words = outcome.overlap_words,
        "streaming decode outcome"
    );
}

#[cfg(test)]
#[path = "stream_reuse_tests.rs"]
mod tests;
