//! Experimental, recording-owned reuse. No text from here is delivered until
//! the ordinary final pipeline validates the overlap and runs transformations.
use crate::preview_pause::PauseSegmenter;
use std::ops::Range;

const SAFETY_SAMPLES: usize = 16_000; // A full second beyond a detected pause.
const MAX_WINDOW: usize = 20 * 16_000;
const MAX_RECORDING: usize = 10 * 60 * 16_000;
pub(crate) const SNAPSHOT_LIMIT: usize = MAX_RECORDING + 16_000;

#[derive(Default)]
pub(crate) struct PreviewReuse {
    id: u64,
    failed: bool,
    segmenter: PauseSegmenter,
    seen: usize,
    cuts: Vec<usize>,
    committed_audio: Vec<f32>,
    prefix: String,
    last_text: String,
    last_start: usize,
}

pub(crate) struct FinalPlan {
    pub start: usize,
    prefix: String,
    overlap: String,
}

impl FinalPlan {
    pub fn stitch(&self, decoded: &str) -> Option<String> {
        // Exact case/punctuation and a word boundary are deliberate: any
        // contextual rewrite of the overlap requires whole-clip ASR instead.
        if split_overlap(decoded.trim(), &self.overlap)?.is_empty() {
            return None;
        }
        Some(join(&self.prefix, decoded.trim()))
    }
}

impl PreviewReuse {
    pub fn begin(&mut self, id: u64) {
        if id > self.id {
            *self = Self {
                id,
                ..Self::default()
            };
        }
    }

    pub fn clear(&mut self, id: u64) {
        if self.id == id {
            *self = Self {
                id,
                failed: true,
                ..Self::default()
            };
        }
    }

    pub fn fail(&mut self, id: u64) {
        if self.id == id {
            self.failed = true;
            self.committed_audio.clear();
            self.prefix.clear();
            self.last_text.clear();
        }
    }

    fn matches_audio(&self, samples: &[f32]) -> bool {
        samples.len() >= self.committed_audio.len()
            && self
                .committed_audio
                .iter()
                .zip(samples)
                .all(|(a, b)| a.to_bits() == b.to_bits())
    }

    pub fn plan(&mut self, id: u64, samples: &[f32]) -> Option<Range<usize>> {
        if self.id != id || id == 0 || self.failed {
            return None;
        }
        if samples.len() < self.seen
            || samples.len() > MAX_RECORDING
            || !self.matches_audio(samples)
        {
            self.fail(id);
            return None;
        }
        // Feed frames individually so multiple pauses within a skipped tick
        // cannot disappear (PauseSegmenter::push returns only its last cut).
        for frame in samples[self.seen..].chunks(crate::preview_pause::FRAME_SAMPLES) {
            if let Some(cut) = self.segmenter.push(frame) {
                self.cuts.push(cut);
            }
        }
        self.seen = samples.len();
        let end = self.cuts.iter().copied().find(|cut| {
            *cut > self.committed_audio.len() && *cut + SAFETY_SAMPLES <= samples.len()
        })?;
        if end - self.last_start > MAX_WINDOW {
            self.fail(id);
            return None;
        }
        Some(self.last_start..end)
    }

    pub fn commit(&mut self, id: u64, range: Range<usize>, samples: &[f32], decoded: &str) {
        if self.id != id || self.failed {
            return;
        }
        let decoded = decoded.trim();
        let suffix = if self.last_text.is_empty() {
            Some(decoded)
        } else {
            split_overlap(decoded, &self.last_text)
        };
        let Some(suffix) = suffix.filter(|text| text.split_whitespace().count() >= 3) else {
            self.fail(id);
            return;
        };
        if range.start != self.last_start
            || range.end > samples.len()
            || !self.matches_audio(samples)
        {
            self.fail(id);
            return;
        }
        self.prefix = join(&self.prefix, &self.last_text);
        self.last_text = suffix.to_string();
        self.last_start = self.committed_audio.len();
        self.committed_audio = samples[..range.end].to_vec();
        self.cuts.retain(|cut| *cut > range.end);
    }

    /// Consume the exact recording's cache once. Stale workers cannot retrieve
    /// or erase a newer recording's state. VAD remains authoritative upstream.
    pub fn take_final(&mut self, id: u64, samples: &[f32]) -> Option<FinalPlan> {
        if self.id != id || id == 0 {
            return None;
        }
        let session = std::mem::replace(
            self,
            Self {
                id,
                failed: true,
                ..Self::default()
            },
        );
        if session.failed
            || samples.len() > MAX_RECORDING
            || session.last_text.is_empty()
            || !session.matches_audio(samples)
            || samples.len().saturating_sub(session.last_start) > MAX_WINDOW
        {
            return None;
        }
        Some(FinalPlan {
            start: session.last_start,
            prefix: session.prefix,
            overlap: session.last_text,
        })
    }
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

fn split_overlap<'a>(decoded: &'a str, overlap: &str) -> Option<&'a str> {
    let rest = decoded.strip_prefix(overlap)?;
    if rest.is_empty() || rest.starts_with(char::is_whitespace) {
        Some(rest.trim_start())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preview_pause::tests::utterance;

    #[test]
    fn pauses_are_old_enough_and_overlap_is_decoded_again() {
        let samples = utterance(&[(4_000, 2_000), (4_000, 2_000), (2_000, 0)], -70.0);
        let mut reuse = PreviewReuse::default();
        reuse.begin(7);
        let first = reuse.plan(7, &samples[..6 * 16000]).unwrap();
        assert_eq!(first, 0..68_800);
        reuse.commit(7, first, &samples, "First complete sentence.");
        let second = reuse.plan(7, &samples[..12 * 16000]).unwrap();
        assert_eq!(second.start, 0); // Last committed segment is the overlap.
        reuse.commit(
            7,
            second,
            &samples,
            "First complete sentence. Another complete sentence.",
        );
        let final_plan = reuse.take_final(7, &samples).unwrap();
        assert_eq!(final_plan.start, 68_800);
        assert_eq!(
            final_plan.stitch("Another complete sentence. Final words here."),
            Some("First complete sentence. Another complete sentence. Final words here.".into())
        );
        assert!(reuse.take_final(7, &samples).is_none());
        assert!(final_plan
            .stitch("Another complete sentence, Final words here.")
            .is_none());
        assert!(final_plan
            .stitch("Another complete sentence.extra")
            .is_none());
    }

    #[test]
    fn stale_error_cancel_and_pcm_mismatch_cannot_reuse() {
        let mut samples = utterance(&[(4_000, 2_000), (2_000, 0)], -70.0);
        let mut reuse = PreviewReuse::default();
        reuse.begin(8);
        reuse.begin(7); // A late ticker cannot replace a newer cache.
        assert!(reuse.plan(7, &samples).is_none());
        let plan = reuse.plan(8, &samples).unwrap();
        reuse.commit(7, plan.clone(), &samples, "Old worker text.");
        reuse.commit(8, plan, &samples, "Current worker text.");
        reuse.fail(7);
        reuse.clear(7);
        assert!(reuse.take_final(7, &samples).is_none());
        samples[123] += 0.001;
        assert!(reuse.take_final(8, &samples).is_none());
        reuse.begin(9);
        let plan = reuse.plan(9, &samples).unwrap();
        reuse.commit(9, plan, &samples, "Current worker text.");
        reuse.fail(9);
        assert!(reuse.take_final(9, &samples).is_none());
    }

    #[test]
    fn crossing_duration_limit_between_preview_and_release_falls_back() {
        let samples = vec![0.0; MAX_RECORDING + 1];
        let mut reuse = PreviewReuse {
            id: 1,
            last_start: MAX_RECORDING - 16_000,
            last_text: "Cached stable words.".into(),
            committed_audio: samples[..MAX_RECORDING - 1_000].to_vec(),
            ..PreviewReuse::default()
        };
        // This cache was within the limit at its last tick. The final tail is
        // still short, but release has crossed the absolute recording limit.
        assert!(reuse.take_final(1, &samples).is_none());
    }

    #[test]
    fn short_no_pause_and_unstable_text_fall_back() {
        let samples = utterance(&[(4_000, 2_000), (4_000, 2_000), (2_000, 0)], -70.0);
        let mut reuse = PreviewReuse::default();
        reuse.begin(1);
        assert!(reuse.plan(1, &samples[..3 * 16000]).is_none());
        assert!(reuse.take_final(1, &samples).is_none());
        reuse.begin(2);
        let plan = reuse.plan(2, &samples[..6 * 16000]).unwrap();
        reuse.commit(2, plan, &samples, "First complete sentence.");
        let plan = reuse.plan(2, &samples[..12 * 16000]).unwrap();
        reuse.commit(
            2,
            plan,
            &samples,
            "Changed complete sentence. More words here.",
        );
        assert!(reuse.take_final(2, &samples).is_none());
    }
}
