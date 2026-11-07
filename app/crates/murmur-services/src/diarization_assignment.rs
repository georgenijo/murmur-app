use crate::meeting_store::{MeetingSegment, MeetingSegmentStatus, MeetingSpeaker};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const MAX_TURNS: usize = 30_000;
pub const MAX_SPEAKERS: u32 = 32;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SpeakerTurn {
    pub speaker: u32,
    pub start_ms: u64,
    pub end_ms: u64,
    pub quality: f64,
}

pub fn valid_turns(turns: &[SpeakerTurn], duration_ms: u64) -> bool {
    !turns.is_empty()
        && turns.len() <= MAX_TURNS
        && turns.iter().all(|turn| {
            turn.speaker < MAX_SPEAKERS
                && turn.start_ms < turn.end_ms
                && turn.end_ms <= duration_ms.saturating_add(100)
                && turn.quality.is_finite()
                && (0.0..=1.0).contains(&turn.quality)
        })
}

pub fn assignments(
    segments: &[MeetingSegment],
    turns: &[SpeakerTurn],
    origin_ms: u64,
) -> Vec<(i64, u32)> {
    let mut accepted = Vec::new();
    let mut labels = HashMap::new();
    for segment in segments {
        if segment.speaker != MeetingSpeaker::Them
            || segment.status != MeetingSegmentStatus::Final
            || segment.text.trim().is_empty()
        {
            continue;
        }
        let Some(start) = segment.start_ms.checked_sub(origin_ms) else {
            continue;
        };
        let Some(end) = segment
            .end_ms
            .checked_sub(origin_ms)
            .filter(|end| *end > start)
        else {
            continue;
        };
        let mut overlaps = turns
            .iter()
            .filter(|turn| turn.start_ms < end && turn.end_ms > start)
            .collect::<Vec<_>>();
        overlaps.sort_by_key(|turn| (turn.start_ms, turn.end_ms));
        let Some(first) = overlaps.first() else {
            continue;
        };
        let speaker = first.speaker;
        let mut previous_end = start;
        let mut coverage = 0;
        let confident = overlaps.iter().all(|turn| {
            let clipped_start = turn.start_ms.max(start);
            let clipped_end = turn.end_ms.min(end);
            if turn.speaker != speaker || turn.quality < 0.5 || clipped_start < previous_end {
                return false;
            }
            coverage += clipped_end - clipped_start;
            previous_end = clipped_end;
            true
        });
        // Existing chunks include at most 250ms pre-roll and 500ms trailing silence.
        // Require 80% coverage even for short chunks. Do not select a dominant voice.
        if !confident
            || coverage * 5 < (end - start) * 4
            || (end - start).saturating_sub(coverage) > 750
        {
            continue;
        }
        let next = labels.len() as u32 + 1;
        let label = *labels.entry(speaker).or_insert(next);
        if label <= MAX_SPEAKERS {
            accepted.push((segment.id, label));
        }
    }
    accepted
}

#[cfg(test)]
mod tests {
    use super::*;
    fn segment(id: i64, speaker: MeetingSpeaker, start_ms: u64, end_ms: u64) -> MeetingSegment {
        MeetingSegment {
            id,
            session_id: "one".into(),
            speaker,
            sequence: id as u64,
            start_ms,
            end_ms,
            status: MeetingSegmentStatus::Final,
            text: "Unchanged transcript".into(),
            audio_available: false,
            error_code: None,
            remote_speaker_id: None,
        }
    }
    fn turn(speaker: u32, start_ms: u64, end_ms: u64) -> SpeakerTurn {
        SpeakerTurn {
            speaker,
            start_ms,
            end_ms,
            quality: 0.9,
        }
    }
    #[test]
    fn preserves_microphone_and_ambiguous_remote_chunks() {
        let segments = vec![
            segment(1, MeetingSpeaker::Me, 0, 1000),
            segment(2, MeetingSpeaker::Them, 0, 1000),
            segment(3, MeetingSpeaker::Them, 1000, 2000),
            segment(4, MeetingSpeaker::Them, 2000, 3000),
        ];
        let before = serde_json::to_string(&segments).unwrap();
        let turns = vec![
            turn(7, 0, 1000),
            turn(7, 1000, 1500),
            turn(8, 1500, 2000),
            turn(8, 2000, 3000),
        ];
        assert_eq!(assignments(&segments, &turns, 0), vec![(2, 1), (4, 2)]);
        assert_eq!(serde_json::to_string(&segments).unwrap(), before);
    }
    #[test]
    fn overlap_low_confidence_missing_coverage_and_wrong_origin_keep_them() {
        let segments = vec![segment(1, MeetingSpeaker::Them, 1000, 2000)];
        for turns in [
            vec![],
            vec![turn(0, 1000, 1200)],
            vec![turn(0, 1000, 1800), turn(0, 1700, 2000)],
            vec![SpeakerTurn {
                quality: 0.49,
                ..turn(0, 1000, 2000)
            }],
        ] {
            assert!(assignments(&segments, &turns, 0).is_empty());
        }
        assert!(assignments(&segments, &[turn(0, 0, 1000)], 2000).is_empty());
    }
    #[test]
    fn measured_four_speaker_fixture_labels_unambiguous_chunks_without_changing_evidence() {
        let turns: Vec<SpeakerTurn> = serde_json::from_str(include_str!(
            "../tests/fixtures/diarization-ami-es2004a.json"
        ))
        .unwrap();
        assert!(valid_turns(&turns, 1_049_355));
        let segments = (0..70)
            .flat_map(|index| {
                let start = index * 15_000;
                let end = (start + 15_000).min(1_049_355);
                [
                    segment(index as i64 * 2 + 1, MeetingSpeaker::Me, start, end),
                    segment(index as i64 * 2 + 2, MeetingSpeaker::Them, start, end),
                ]
            })
            .collect::<Vec<_>>();
        let before = serde_json::to_string(&segments).unwrap();
        let accepted = assignments(&segments, &turns, 0);
        assert_eq!(accepted.len(), 9);
        assert_eq!(
            accepted
                .iter()
                .map(|(_, speaker)| *speaker)
                .collect::<std::collections::BTreeSet<_>>(),
            (1..=4).collect()
        );
        assert!(accepted.iter().all(|(id, _)| id % 2 == 0));
        assert_eq!(assignments(&segments, &turns, 0), accepted);
        assert_eq!(serde_json::to_string(&segments).unwrap(), before);
    }

    #[test]
    fn rejects_unbounded_or_nonfinite_worker_data() {
        assert!(!valid_turns(
            &[SpeakerTurn {
                quality: f64::NAN,
                ..turn(0, 0, 1000)
            }],
            1000
        ));
        assert!(!valid_turns(&[turn(32, 0, 1000)], 1000));
        assert!(!valid_turns(&[turn(0, 0, 1200)], 1000));
    }
}
