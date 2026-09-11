use super::{speaker_label, MeetingWorkspace, MAX_EXPORT_BYTES};
use crate::meeting_store::{MeetingSegmentStatus, MeetingSessionStatus};

#[derive(Clone, Copy)]
pub(super) enum Format {
    Srt,
    Vtt,
}

fn timestamp(ms: u64, format: Format) -> String {
    let separator = match format {
        Format::Srt => ',',
        Format::Vtt => '.',
    };
    format!(
        "{:02}:{:02}:{:02}{separator}{:03}",
        ms / 3_600_000,
        (ms / 60_000) % 60,
        (ms / 1_000) % 60,
        ms % 1_000
    )
}

fn cue_text(value: &str) -> String {
    let visible: String = value
        .chars()
        .filter(|character| {
            (!character.is_control() || character.is_whitespace())
                && !matches!(character, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
        })
        .collect();
    visible
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub(super) fn render(workspace: &MeetingWorkspace, format: Format) -> Result<String, String> {
    if workspace.session.status == MeetingSessionStatus::Active {
        return Err("Stop this meeting before exporting captions.".into());
    }
    if !workspace.segments.iter().any(|segment| {
        segment.status == MeetingSegmentStatus::Final && !cue_text(&segment.text).is_empty()
    }) {
        return Err("No transcribed speech is available for captions.".into());
    }

    let mut segments: Vec<_> = workspace.segments.iter().collect();
    segments.sort_by_key(|segment| (segment.start_ms, segment.id));
    let mut output = match format {
        Format::Srt => String::new(),
        Format::Vtt => "WEBVTT\n\n".into(),
    };
    let mut cue_number = 0;
    for segment in segments {
        let text = match segment.status {
            MeetingSegmentStatus::Final => cue_text(&segment.text),
            MeetingSegmentStatus::Pending => "[Transcription pending]".into(),
            MeetingSegmentStatus::Failed => "[Transcription unavailable]".into(),
        };
        if text.is_empty() {
            continue;
        }
        if segment.end_ms <= segment.start_ms {
            return Err(
                "A transcript segment has invalid timing. Captions were not exported.".into(),
            );
        }
        cue_number += 1;
        let label = cue_text(speaker_label(workspace, segment));
        output.push_str(&format!(
            "{cue_number}\n{} --> {}\n{label}: {text}\n\n",
            timestamp(segment.start_ms, format),
            timestamp(segment.end_ms, format),
        ));
        if output.len() > MAX_EXPORT_BYTES {
            return Err("The meeting captions are too large to export.".into());
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meeting_review::{MeetingReviewExportFormat, MeetingSpeakerLabels};
    use crate::meeting_store::{
        MeetingSegment, MeetingSession, MeetingSpeaker, RemoteSpeakerLabel,
    };

    fn segment(id: i64, start_ms: u64, end_ms: u64, text: &str) -> MeetingSegment {
        MeetingSegment {
            id,
            session_id: "caption-test".into(),
            speaker: MeetingSpeaker::Me,
            remote_speaker_id: None,
            sequence: 0,
            start_ms,
            end_ms,
            status: MeetingSegmentStatus::Final,
            text: text.into(),
            audio_available: false,
            error_code: None,
        }
    }

    fn workspace(segments: Vec<MeetingSegment>) -> MeetingWorkspace {
        MeetingWorkspace {
            session: MeetingSession {
                id: "caption-test".into(),
                started_at_ms: 1_789_000_000_000,
                ended_at_ms: Some(1_789_004_000_000),
                status: MeetingSessionStatus::Complete,
                model_name: "base.en".into(),
                language: "en".into(),
                smart_punctuation: true,
                retain_audio: false,
                duration_ms: 4_000_000,
                segment_count: segments.len() as u64,
                preview: "not part of captions".into(),
                error_code: None,
            },
            segments,
            labels: MeetingSpeakerLabels {
                me: "Alex".into(),
                them: "Team".into(),
            },
            remote_speakers: vec![RemoteSpeakerLabel {
                speaker_id: 1,
                label: "Sam".into(),
            }],
            generated: None,
            review: None,
            active_document: None,
            active_origin: None,
        }
    }

    #[test]
    fn formats_sorted_overlapping_cues_without_changing_their_capture_times() {
        let mut remote = segment(2, 3_599_900, 3_602_020, "We can ship.");
        remote.speaker = MeetingSpeaker::Them;
        remote.remote_speaker_id = Some(1);
        let workspace = workspace(vec![remote, segment(1, 3_599_900, 3_601_007, "Ready?")]);
        let expected = "1\n00:59:59,900 --> 01:00:01,007\nAlex: Ready?\n\n2\n00:59:59,900 --> 01:00:02,020\nSam: We can ship.\n\n";
        assert_eq!(
            super::super::render_export(&workspace, MeetingReviewExportFormat::Srt).unwrap(),
            expected
        );
        assert_eq!(
            super::super::render_export(&workspace, MeetingReviewExportFormat::Vtt).unwrap(),
            format!("WEBVTT\n\n{}", expected.replace(',', "."))
        );
    }

    #[test]
    fn escapes_content_and_labels_and_marks_missing_transcriptions() {
        let mut failed = segment(3, 3_000, 4_000, "private error detail");
        failed.status = MeetingSegmentStatus::Failed;
        failed.speaker = MeetingSpeaker::Them;
        failed.remote_speaker_id = Some(99);
        let mut pending = segment(4, 4_000, 5_000, "unfinished text");
        pending.status = MeetingSegmentStatus::Pending;
        let mut workspace = workspace(vec![
            segment(
                1,
                0,
                1_000,
                "<b>R&D</b>\r\n\n2\n00:00:00.000 --> 01:00:00.000\0\u{202e}",
            ),
            segment(2, 1_000, 2_000, " \n\t"),
            failed,
            pending,
        ]);
        workspace.labels.me = "A < B & C".into();
        for format in [Format::Srt, Format::Vtt] {
            let output = render(&workspace, format).unwrap();
            assert!(output.contains(
                "A &lt; B &amp; C: &lt;b&gt;R&amp;D&lt;/b&gt; 2 00:00:00.000 --&gt; 01:00:00.000"
            ));
            assert!(output.contains("Team: [Transcription unavailable]"));
            assert!(output.contains("A &lt; B &amp; C: [Transcription pending]"));
            assert_eq!(output.matches(" --> ").count(), 3);
            assert!(!output.contains("private error detail"));
            assert!(!output.contains("unfinished text"));
            assert!(!output.contains("not part of captions"));
            assert!(!output.contains('\0'));
            assert!(!output.contains('\u{202e}'));
        }
    }

    #[test]
    fn refuses_active_empty_and_invalid_timing_without_inventing_cues() {
        let mut workspace = workspace(vec![segment(1, 0, 1_000, "Hello")]);
        workspace.session.status = MeetingSessionStatus::Active;
        assert!(render(&workspace, Format::Srt)
            .unwrap_err()
            .contains("Stop this meeting"));
        workspace.session.status = MeetingSessionStatus::Complete;
        for end_ms in [0, 9] {
            workspace.segments = vec![segment(1, 10, end_ms, "Hello")];
            assert!(render(&workspace, Format::Vtt)
                .unwrap_err()
                .contains("invalid timing"));
        }
        workspace.segments.clear();
        assert!(render(&workspace, Format::Srt)
            .unwrap_err()
            .contains("No transcribed speech"));
    }

    #[test]
    fn both_formats_enforce_the_complete_export_size_bound() {
        let workspace = workspace(vec![segment(1, 0, 1_000, &"x".repeat(MAX_EXPORT_BYTES))]);
        for format in [Format::Srt, Format::Vtt] {
            assert!(render(&workspace, format)
                .unwrap_err()
                .contains("too large"));
        }
    }
}
