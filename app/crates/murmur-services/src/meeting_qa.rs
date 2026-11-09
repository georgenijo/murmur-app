//! Deterministic setup for native acceptance of the Meeting review failure rows.
//! Both hooks enter through the real service paths: the save commands and the
//! meeting store that the review, playback and bundle code read.
//!
//! Every hook is gated twice by `meeting_capture::qa::armed`: the
//! `native-host-qa` feature must be compiled in and the process must be the exact
//! QA bundle, whose data folder is separate from the production one. Any other
//! build or identity never reads the environment, so these are inert in production.
use crate::meeting_store::{
    MeetingRepository, MeetingSessionStatus, MeetingSpeaker, SaveMeetingMetadataRequest,
};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

/// `reject` or `delay`: the first meeting save after launch (review details or an
/// export) fails with the service's own rejection, or is held for `DELAY`.
const SAVE_ENV: &str = "MURMUR_QA_MEETING_SAVE";
/// `1` seeds one finished meeting with retained audio in which a segment's audio
/// file is gone, so the audio bundle reports an unavailable segment.
const FIXTURE_ENV: &str = "MURMUR_QA_MEETING_AUDIO_FIXTURE";
/// The synthetic meeting's id, as shown by the QA bundle's own list.
pub(crate) const FIXTURE_SESSION: &str = "qa-retained-audio";
/// Long enough to observe the pending state of a save by hand.
const DELAY: Duration = Duration::from_secs(5);

/// The save the one-shot fault may hit.
#[derive(Clone, Copy)]
pub(crate) enum Save {
    Metadata,
    Export,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fault {
    Reject,
    Delay,
}

static SAVE_SPENT: AtomicBool = AtomicBool::new(false);

fn fault_for(value: Option<&str>) -> Option<Fault> {
    match value?.trim() {
        "reject" => Some(Fault::Reject),
        "delay" => Some(Fault::Delay),
        _ => None,
    }
}

/// The rejection a real save of this kind returns, so the presentation shows the
/// same explanation it would for a genuine failure.
fn rejection(save: Save) -> &'static str {
    match save {
        Save::Metadata => "The meeting title is invalid.",
        Save::Export => "Failed to write export: QA injected rejection",
    }
}

fn consume(value: Option<&str>, spent: &AtomicBool) -> Option<Fault> {
    let fault = fault_for(value)?;
    (!spent.swap(true, Ordering::AcqRel)).then_some(fault)
}

/// Called at the start of a meeting save. One-shot: only the first save after
/// launch is affected, so the retry that follows the failure succeeds.
pub(crate) fn save_gate(save: Save) -> Result<(), String> {
    let value = crate::meeting_capture::qa::setting(SAVE_ENV);
    match consume(value.as_deref(), &SAVE_SPENT) {
        Some(Fault::Reject) => Err(rejection(save).into()),
        Some(Fault::Delay) => {
            std::thread::sleep(DELAY);
            Ok(())
        }
        None => Ok(()),
    }
}

/// Seeds the synthetic retained-audio meeting when the QA bundle asks for it.
pub(crate) fn seed(repository: &MeetingRepository) {
    if crate::meeting_capture::qa::setting(FIXTURE_ENV).as_deref() == Some("1") {
        if let Err(error) = build_fixture(repository) {
            tracing::warn!(target: "meeting", error = %error, "meeting QA fixture was not seeded");
        }
    }
}

fn write_wav(path: &std::path::Path, milliseconds: u64) -> Result<(), String> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).map_err(|e| e.to_string())?;
    for sample in 0..milliseconds * 16 {
        writer
            .write_sample(((sample % 100) as i16 - 50) * 100)
            .map_err(|e| e.to_string())?;
    }
    writer.finalize().map_err(|e| e.to_string())
}

/// Three finished segments with retained audio; the second one's audio file is
/// removed, so the real manifest, playback and bundle code see it as unavailable.
/// Idempotent: an existing fixture is left alone.
pub(crate) fn build_fixture(repository: &MeetingRepository) -> Result<(), String> {
    if repository.get_session(FIXTURE_SESSION).is_ok() {
        return Ok(());
    }
    repository.create_session(FIXTURE_SESSION, "base.en", "en", true, true)?;
    let segments = [
        (MeetingSpeaker::Me, 0, 0, "Synthetic opening line.", true),
        (
            MeetingSpeaker::Them,
            0,
            3_000,
            "Synthetic reply whose audio file is gone.",
            false,
        ),
        (
            MeetingSpeaker::Me,
            1,
            6_000,
            "Synthetic closing line.",
            true,
        ),
    ];
    for (speaker, sequence, start_ms, text, present) in segments {
        let relative = format!(
            "audio/{FIXTURE_SESSION}/{}-{sequence:08}.wav",
            speaker.as_db()
        );
        let path = repository.root().join(&relative);
        std::fs::create_dir_all(path.parent().ok_or("fixture path")?).map_err(|e| e.to_string())?;
        write_wav(&path, 3_000)?;
        let id = repository.insert_pending_segment(
            FIXTURE_SESSION,
            speaker,
            sequence,
            start_ms,
            start_ms + 3_000,
            &relative,
        )?;
        repository.finalize_segment(id, text, true)?;
        if !present {
            std::fs::remove_file(&path).map_err(|e| e.to_string())?;
        }
    }
    repository.finish_session(FIXTURE_SESSION, MeetingSessionStatus::Complete, None)?;
    repository.save_metadata(SaveMeetingMetadataRequest {
        session_id: FIXTURE_SESSION.into(),
        title: Some("Synthetic retained audio".into()),
        attendees: Vec::new(),
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_save_fault_is_one_shot_and_needs_a_known_value() {
        let spent = AtomicBool::new(false);
        assert_eq!(consume(Some("nonsense"), &spent), None);
        assert_eq!(consume(None, &spent), None);
        assert!(!spent.load(Ordering::Acquire), "unknown values do not arm");
        assert_eq!(consume(Some("reject"), &spent), Some(Fault::Reject));
        assert_eq!(consume(Some("reject"), &spent), None);
        let spent = AtomicBool::new(false);
        assert_eq!(consume(Some(" delay "), &spent), Some(Fault::Delay));
        assert_eq!(consume(Some("delay"), &spent), None);
    }

    #[test]
    fn hooks_are_inert_outside_the_qa_bundle() {
        // Test builds never carry the QA identity, so neither hook reads its variable.
        std::env::set_var(SAVE_ENV, "reject");
        std::env::set_var(FIXTURE_ENV, "1");
        assert!(save_gate(Save::Metadata).is_ok());
        assert!(save_gate(Save::Export).is_ok());
        let temp = tempfile::TempDir::new().unwrap();
        let (repository, _) =
            MeetingRepository::initialize(temp.path().canonicalize().unwrap()).unwrap();
        seed(&repository);
        assert!(repository.get_session(FIXTURE_SESSION).is_err());
        std::env::remove_var(SAVE_ENV);
        std::env::remove_var(FIXTURE_ENV);
    }

    #[test]
    fn injected_rejections_are_the_real_ones_the_presentation_explains() {
        for (save, command) in [
            (Save::Metadata, "save_meeting_metadata"),
            (Save::Export, "save_meeting_review_export"),
        ] {
            assert!(
                crate::meeting_failure::explain(command, rejection(save)).is_some(),
                "{command} would show the generic failure"
            );
        }
    }

    #[test]
    fn the_fixture_has_three_segments_and_exactly_one_missing_audio_file() {
        let temp = tempfile::TempDir::new().unwrap();
        let (repository, _) =
            MeetingRepository::initialize(temp.path().canonicalize().unwrap()).unwrap();
        build_fixture(&repository).unwrap();
        // Seeding again leaves the meeting alone.
        build_fixture(&repository).unwrap();
        let workspace = repository.workspace(FIXTURE_SESSION).unwrap();
        assert_eq!(workspace.segments.len(), 3);
        let present = std::fs::read_dir(repository.root().join("audio").join(FIXTURE_SESSION))
            .unwrap()
            .count();
        assert_eq!(present, 2);
    }
}
