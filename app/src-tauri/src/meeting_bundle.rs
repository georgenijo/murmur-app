//! Self-contained meeting bundle: the existing review export plus the session's
//! retained WAV segments in one `.zip`.
//!
//! The document is exactly what [`render_export`] produces for the chosen
//! format, and every audio byte is read through the playback path in
//! `meeting_audio` (same ownership, no-follow, and WAV checks). Audio is stored
//! as recorded: entries use the zip `Stored` method, so nothing is transcoded
//! or re-compressed.
//!
//! Size contract: the 8 MiB document limit does not apply to audio. The total
//! retained audio is capped at [`MAX_BUNDLE_AUDIO_BYTES`] (a single zip needs no
//! zip64 below it), the preview command reports the exact size before the user
//! picks a destination, and the archive is streamed to disk one segment at a
//! time rather than built in memory.
//!
//! Completeness contract: a segment whose WAV is missing, pruned, malformed, or
//! outside the owned spool is omitted and listed in `manifest.json` with
//! `"complete": false` and a notice; the save receipt reports the omitted count
//! so the UI can say so. A segment that changes or is revoked mid-export aborts
//! the whole export. A bundle with no audio at all is refused rather than
//! written.

use crate::meeting_audio::{
    audio_file_len, current_audio_revision, read_audio_whole, AudioChunk, AudioCursor,
    AudioManifest, AudioReadPermit, AudioUnavailableReason, PlaybackChannel, AUDIO_UNAVAILABLE,
    MAX_MANIFEST_CHUNKS,
};
use crate::meeting_review::{render_export, MeetingReviewExportFormat, MeetingWorkspace};
use crate::meeting_store::MeetingRepository;
use crate::State;
use serde::Serialize;
use std::io::{Seek, Write};
use tauri::Manager;

pub const MEETING_BUNDLE_SCHEMA: &str = "murmur.meeting-bundle.v1";
/// Total retained audio one bundle may carry. Larger meetings can still export
/// the document alone.
pub const MAX_BUNDLE_AUDIO_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_BUNDLE_SEGMENTS: usize = 50_000;
const MANIFEST_NAME: &str = "manifest.json";
const OMITTED_REASON: &str = "missing_or_unreadable";
const WRITE_FAILED: &str = "The meeting bundle could not be written.";
const NOTHING_TO_BUNDLE: &str =
    "None of this meeting's retained audio files could be found, so there is no audio to bundle.";

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BundlePreview {
    pub audio_segments: u64,
    pub audio_bytes: u64,
    pub unavailable_segments: u64,
    pub max_audio_bytes: u64,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BundleReceipt {
    pub bytes: u64,
    pub included_segments: u64,
    pub omitted_segments: u64,
}

/// Every retained segment the playback manifest lists for a finished session,
/// or a clear refusal. Reuses the manifest query so "retained" means the same
/// thing here as in playback.
fn retained_chunks(
    repository: &MeetingRepository,
    session_id: &str,
) -> Result<Vec<AudioChunk>, String> {
    let mut chunks = Vec::new();
    let mut cursor: Option<AudioCursor> = None;
    loop {
        match repository.audio_manifest(
            session_id,
            0,
            PlaybackChannel::All,
            cursor.as_ref(),
            MAX_MANIFEST_CHUNKS,
        )? {
            AudioManifest::Unavailable { reason } => {
                return Err(match reason {
                    AudioUnavailableReason::NotRetained => {
                        "Audio was not retained for this meeting, so it cannot be exported with audio."
                    }
                    AudioUnavailableReason::NotFinished => {
                        "Stop this meeting before exporting it with audio."
                    }
                    AudioUnavailableReason::NoAudio => {
                        "This meeting has no retained audio segments to export."
                    }
                }
                .into());
            }
            AudioManifest::Available {
                chunks: page,
                next_cursor,
                ..
            } => {
                chunks.extend(page);
                if chunks.len() > MAX_BUNDLE_SEGMENTS {
                    return Err("This meeting has too many audio segments for one bundle.".into());
                }
                match next_cursor {
                    Some(next) => cursor = Some(next),
                    None => return Ok(chunks),
                }
            }
        }
    }
}

fn too_large() -> String {
    format!(
        "This meeting's audio is larger than the {} GiB bundle limit. Export the review document on its own instead.",
        MAX_BUNDLE_AUDIO_BYTES / (1024 * 1024 * 1024)
    )
}

/// Size and completeness of the bundle the user is about to save.
pub(crate) fn preview_bundle(
    repository: &MeetingRepository,
    session_id: &str,
) -> Result<BundlePreview, String> {
    let mut preview = BundlePreview {
        audio_segments: 0,
        audio_bytes: 0,
        unavailable_segments: 0,
        max_audio_bytes: MAX_BUNDLE_AUDIO_BYTES,
    };
    for chunk in retained_chunks(repository, session_id)? {
        match audio_file_len(repository, session_id, chunk.segment_id) {
            Ok(length) => {
                preview.audio_segments += 1;
                preview.audio_bytes += length;
            }
            Err(error) if error == AUDIO_UNAVAILABLE => preview.unavailable_segments += 1,
            Err(error) => return Err(error),
        }
    }
    if preview.audio_segments == 0 {
        return Err(NOTHING_TO_BUNDLE.into());
    }
    Ok(preview)
}

fn write_failed<E>(_: E) -> String {
    WRITE_FAILED.into()
}

/// Write the bundle for `workspace` into `writer`. `revision` is the audio
/// revision captured when the export started; a delete or invalidation after
/// that aborts the export.
pub(crate) fn write_bundle<W: Write + Seek>(
    repository: &MeetingRepository,
    workspace: &MeetingWorkspace,
    format: MeetingReviewExportFormat,
    revision: u64,
    writer: W,
) -> Result<BundleReceipt, String> {
    write_bundle_limited(
        repository,
        workspace,
        format,
        revision,
        MAX_BUNDLE_AUDIO_BYTES,
        writer,
    )
}

fn write_bundle_limited<W: Write + Seek>(
    repository: &MeetingRepository,
    workspace: &MeetingWorkspace,
    format: MeetingReviewExportFormat,
    revision: u64,
    max_audio_bytes: u64,
    writer: W,
) -> Result<BundleReceipt, String> {
    use zip::write::SimpleFileOptions;
    let session_id = workspace.session.id.as_str();
    if !workspace.session.retain_audio {
        return Err(
            "Audio was not retained for this meeting, so it cannot be exported with audio.".into(),
        );
    }
    let chunks = retained_chunks(repository, session_id)?;
    // Byte-for-byte the same document the standalone export writes.
    let document = render_export(workspace, format)?;
    let document_name = format!("review.{}", format.extension());
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

    let mut zip = zip::ZipWriter::new(writer);
    zip.start_file(document_name.as_str(), options)
        .map_err(write_failed)?;
    zip.write_all(document.as_bytes()).map_err(write_failed)?;

    let mut included = Vec::new();
    let mut omitted = Vec::new();
    let mut audio_bytes = 0_u64;
    for chunk in &chunks {
        let bytes = match read_audio_whole(repository, session_id, chunk.segment_id, revision) {
            Ok(bytes) => bytes,
            Err(error) if error == AUDIO_UNAVAILABLE => {
                omitted.push(serde_json::json!({
                    "segmentId": chunk.segment_id,
                    "channel": chunk.channel,
                    "startMs": chunk.start_ms,
                    "endMs": chunk.end_ms,
                    "reason": OMITTED_REASON,
                }));
                continue;
            }
            Err(error) => return Err(error),
        };
        audio_bytes += bytes.len() as u64;
        if audio_bytes > max_audio_bytes {
            return Err(too_large());
        }
        let name = format!(
            "audio/segment-{:08}-{}.wav",
            chunk.segment_id,
            chunk.channel.as_db()
        );
        zip.start_file(name.as_str(), options)
            .map_err(write_failed)?;
        zip.write_all(&bytes).map_err(write_failed)?;
        included.push(serde_json::json!({
            "segmentId": chunk.segment_id,
            "channel": chunk.channel,
            "startMs": chunk.start_ms,
            "endMs": chunk.end_ms,
            "file": name,
            "bytes": bytes.len(),
        }));
    }
    if included.is_empty() {
        return Err(NOTHING_TO_BUNDLE.into());
    }

    let complete = omitted.is_empty();
    let notice = (!complete).then(|| {
        format!(
            "{} retained audio segment(s) were missing or unreadable and are not in this bundle, so the audio is incomplete.",
            omitted.len()
        )
    });
    let (included_segments, omitted_segments) = (included.len() as u64, omitted.len() as u64);
    let manifest = serde_json::to_string_pretty(&serde_json::json!({
        "schema": MEETING_BUNDLE_SCHEMA,
        "sessionId": session_id,
        "complete": complete,
        "notice": notice,
        "document": {
            "file": document_name,
            "format": format,
            "bytes": document.len(),
        },
        "audio": {
            "encoding": "WAV, 16 kHz mono 16-bit PCM, stored exactly as recorded",
            "totalBytes": audio_bytes,
            "includedSegments": included,
            "omittedSegments": omitted,
        },
    }))
    .map_err(write_failed)?;
    zip.start_file(MANIFEST_NAME, options)
        .map_err(write_failed)?;
    zip.write_all(manifest.as_bytes()).map_err(write_failed)?;
    let mut writer = zip.finish().map_err(write_failed)?;
    Ok(BundleReceipt {
        bytes: writer.stream_position().map_err(write_failed)?,
        included_segments,
        omitted_segments,
    })
}

#[tauri::command]
pub async fn get_meeting_bundle_preview(
    window: tauri::WebviewWindow,
    id: String,
    state: tauri::State<'_, State>,
) -> Result<BundlePreview, String> {
    crate::meeting_audio::require_main_window(window.label())?;
    if crate::meeting_audio::capture_busy(&state) {
        return Err(crate::meeting_audio::BUNDLE_CAPTURE_BUSY.into());
    }
    let repository = state.meeting_store.repository()?;
    let permit = AudioReadPermit::acquire()?;
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        preview_bundle(&repository, id.trim())
    })
    .await
    .map_err(|_| AUDIO_UNAVAILABLE.to_string())?
}

#[tauri::command]
pub async fn save_meeting_bundle_export(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    id: String,
    format: MeetingReviewExportFormat,
    path: String,
    state: tauri::State<'_, State>,
) -> Result<BundleReceipt, String> {
    crate::meeting_audio::require_main_window(window.label())?;
    let path = std::path::PathBuf::from(path);
    crate::commands::export::validate_bundle_path(&path)?;
    if crate::meeting_audio::capture_busy(&state) {
        return Err(crate::meeting_audio::BUNDLE_CAPTURE_BUSY.into());
    }
    let repository = state.meeting_store.repository()?;
    let permit = AudioReadPermit::acquire()?;
    let revision = current_audio_revision();
    let receipt = tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        if crate::meeting_audio::capture_busy(&app.state::<State>()) {
            return Err(crate::meeting_audio::BUNDLE_CAPTURE_BUSY.to_string());
        }
        let workspace = repository.workspace(id.trim())?;
        let mut receipt = None;
        crate::commands::export::publish_with(&path, |file| {
            let written = write_bundle(&repository, &workspace, format, revision, &mut *file)?;
            let bytes = written.bytes;
            receipt = Some(written);
            Ok(bytes)
        })?;
        receipt.ok_or_else(|| WRITE_FAILED.to_string())
    })
    .await
    .map_err(|_| AUDIO_UNAVAILABLE.to_string())??;
    // Content-free: counts and size only, never what was said.
    tracing::info!(
        target: "pipeline",
        bytes = receipt.bytes,
        included = receipt.included_segments,
        omitted = receipt.omitted_segments,
        "meeting bundle written"
    );
    Ok(receipt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meeting_store::{MeetingSessionStatus, MeetingSpeaker};
    use std::io::{Cursor, Read};
    use std::path::PathBuf;

    fn repository() -> (tempfile::TempDir, MeetingRepository) {
        let temporary = tempfile::tempdir().unwrap();
        let (repository, _) =
            MeetingRepository::initialize(temporary.path().canonicalize().unwrap()).unwrap();
        (temporary, repository)
    }

    fn session(repository: &MeetingRepository, id: &str, retained: bool) {
        repository
            .create_session(id, "base.en", "en", true, retained)
            .unwrap();
    }

    fn clip(
        repository: &MeetingRepository,
        session_id: &str,
        channel: MeetingSpeaker,
        sequence: u64,
        start_ms: u64,
    ) -> (i64, PathBuf) {
        let relative = format!("audio/{session_id}/{}-{sequence:08}.wav", channel.as_db());
        let path = repository.root().join(&relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut writer = hound::WavWriter::create(
            &path,
            hound::WavSpec {
                channels: 1,
                sample_rate: 16_000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for sample in 0..1_000 * 16 {
            writer.write_sample((sample % 1_000) as i16).unwrap();
        }
        writer.finalize().unwrap();
        let id = repository
            .insert_pending_segment(
                session_id,
                channel,
                sequence,
                start_ms,
                start_ms + 1_000,
                &relative,
            )
            .unwrap();
        repository
            .finalize_segment(id, "synthetic test speech", true)
            .unwrap();
        (id, path)
    }

    fn finish(repository: &MeetingRepository, session_id: &str) {
        repository
            .finish_session(session_id, MeetingSessionStatus::Complete, None)
            .unwrap();
    }

    fn bundle(
        repository: &MeetingRepository,
        session_id: &str,
        format: MeetingReviewExportFormat,
    ) -> Result<(BundleReceipt, Vec<u8>), String> {
        let workspace = repository.workspace(session_id).unwrap();
        let mut output = Cursor::new(Vec::new());
        let receipt = write_bundle(
            repository,
            &workspace,
            format,
            current_audio_revision(),
            &mut output,
        )?;
        Ok((receipt, output.into_inner()))
    }

    fn entry(archive: &mut zip::ZipArchive<Cursor<Vec<u8>>>, name: &str) -> Vec<u8> {
        let mut bytes = Vec::new();
        archive
            .by_name(name)
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        bytes
    }

    fn names(archive: &zip::ZipArchive<Cursor<Vec<u8>>>) -> Vec<String> {
        let mut names = archive.file_names().map(str::to_string).collect::<Vec<_>>();
        names.sort();
        names
    }

    #[test]
    fn bundle_holds_the_exact_document_every_wav_and_a_manifest_without_local_paths() {
        let (_temporary, repository) = repository();
        session(&repository, "full", true);
        let (me, me_path) = clip(&repository, "full", MeetingSpeaker::Me, 0, 0);
        let (them, them_path) = clip(&repository, "full", MeetingSpeaker::Them, 0, 500);
        finish(&repository, "full");

        for format in [
            MeetingReviewExportFormat::Markdown,
            MeetingReviewExportFormat::Json,
            MeetingReviewExportFormat::Srt,
        ] {
            let (receipt, bytes) = bundle(&repository, "full", format).unwrap();
            assert_eq!(receipt.bytes, bytes.len() as u64);
            assert_eq!(
                (receipt.included_segments, receipt.omitted_segments),
                (2, 0)
            );
            let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
            let document_name = format!("review.{}", format.extension());
            let me_name = format!("audio/segment-{me:08}-me.wav");
            let them_name = format!("audio/segment-{them:08}-them.wav");
            let mut expected = vec![
                document_name.clone(),
                me_name.clone(),
                them_name.clone(),
                MANIFEST_NAME.to_string(),
            ];
            expected.sort();
            assert_eq!(names(&archive), expected);
            assert_eq!(
                entry(&mut archive, &document_name),
                render_export(&repository.workspace("full").unwrap(), format)
                    .unwrap()
                    .into_bytes()
            );
            assert_eq!(
                entry(&mut archive, &me_name),
                std::fs::read(&me_path).unwrap()
            );
            assert_eq!(
                entry(&mut archive, &them_name),
                std::fs::read(&them_path).unwrap()
            );
            for index in 0..archive.len() {
                assert_eq!(
                    archive.by_index(index).unwrap().compression(),
                    zip::CompressionMethod::Stored,
                    "audio must never be re-compressed or transcoded"
                );
            }

            let manifest_text = String::from_utf8(entry(&mut archive, MANIFEST_NAME)).unwrap();
            let manifest: serde_json::Value = serde_json::from_str(&manifest_text).unwrap();
            assert_eq!(manifest["schema"], MEETING_BUNDLE_SCHEMA);
            assert_eq!(manifest["complete"], true);
            assert!(manifest["notice"].is_null());
            assert_eq!(manifest["document"]["file"], document_name);
            assert_eq!(
                manifest["audio"]["includedSegments"]
                    .as_array()
                    .unwrap()
                    .len(),
                2
            );
            assert!(manifest["audio"]["omittedSegments"]
                .as_array()
                .unwrap()
                .is_empty());
            assert!(!manifest_text.contains(repository.root().to_str().unwrap()));
            assert!(!manifest_text.contains("synthetic test speech"));
        }
    }

    #[test]
    fn a_missing_wav_is_omitted_with_a_visible_note_never_a_silently_shorter_bundle() {
        let (_temporary, repository) = repository();
        session(&repository, "gap", true);
        let (kept, _) = clip(&repository, "gap", MeetingSpeaker::Me, 0, 0);
        let (pruned, pruned_path) = clip(&repository, "gap", MeetingSpeaker::Them, 0, 500);
        finish(&repository, "gap");
        std::fs::remove_file(&pruned_path).unwrap();

        let preview = preview_bundle(&repository, "gap").unwrap();
        assert_eq!(
            (preview.audio_segments, preview.unavailable_segments),
            (1, 1)
        );

        let (receipt, bytes) = bundle(&repository, "gap", MeetingReviewExportFormat::Text).unwrap();
        assert_eq!(
            (receipt.included_segments, receipt.omitted_segments),
            (1, 1)
        );
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut expected = vec![
            "review.txt".to_string(),
            format!("audio/segment-{kept:08}-me.wav"),
            MANIFEST_NAME.to_string(),
        ];
        expected.sort();
        assert_eq!(names(&archive), expected);
        let manifest: serde_json::Value =
            serde_json::from_slice(&entry(&mut archive, MANIFEST_NAME)).unwrap();
        assert_eq!(manifest["complete"], false);
        assert!(manifest["notice"].as_str().unwrap().contains("incomplete"));
        let omitted = manifest["audio"]["omittedSegments"].as_array().unwrap();
        assert_eq!(omitted.len(), 1);
        assert_eq!(omitted[0]["segmentId"], pruned);
        assert_eq!(omitted[0]["reason"], OMITTED_REASON);
    }

    #[test]
    fn a_bundle_with_no_readable_audio_is_refused_not_written_empty() {
        let (_temporary, repository) = repository();
        session(&repository, "gone", true);
        let (_, path) = clip(&repository, "gone", MeetingSpeaker::Me, 0, 0);
        finish(&repository, "gone");
        std::fs::remove_file(path).unwrap();
        assert_eq!(
            preview_bundle(&repository, "gone").unwrap_err(),
            NOTHING_TO_BUNDLE
        );
        assert_eq!(
            bundle(&repository, "gone", MeetingReviewExportFormat::Markdown).unwrap_err(),
            NOTHING_TO_BUNDLE
        );
    }

    #[test]
    fn sessions_without_retained_audio_are_refused() {
        let (_temporary, repository) = repository();
        session(&repository, "private", false);
        clip(&repository, "private", MeetingSpeaker::Me, 0, 0);
        finish(&repository, "private");
        let error = bundle(&repository, "private", MeetingReviewExportFormat::Json).unwrap_err();
        assert!(error.contains("not retained"), "{error}");
        assert!(preview_bundle(&repository, "private")
            .unwrap_err()
            .contains("not retained"));

        session(&repository, "live", true);
        clip(&repository, "live", MeetingSpeaker::Me, 0, 0);
        let error = bundle(&repository, "live", MeetingReviewExportFormat::Json).unwrap_err();
        assert!(error.contains("Stop this meeting"), "{error}");

        session(&repository, "silent", true);
        finish(&repository, "silent");
        let error = bundle(&repository, "silent", MeetingReviewExportFormat::Json).unwrap_err();
        assert!(error.contains("no retained audio"), "{error}");
    }

    #[test]
    fn a_forged_traversal_path_is_never_read_into_the_bundle() {
        let (temporary, repository) = repository();
        session(&repository, "forged", true);
        let (honest, _) = clip(&repository, "forged", MeetingSpeaker::Me, 0, 0);
        let (forged, forged_path) = clip(&repository, "forged", MeetingSpeaker::Them, 0, 500);
        finish(&repository, "forged");
        let outside = temporary.path().canonicalize().unwrap().join("outside.wav");
        // A still-valid WAV whose samples differ from every real segment.
        let mut secret = std::fs::read(&forged_path).unwrap();
        *secret.last_mut().unwrap() ^= 0x55;
        std::fs::write(&outside, &secret).unwrap();

        assert!(repository.audio_path("audio/../outside.wav").is_err());
        let connection =
            rusqlite::Connection::open(repository.root().join("meetings.sqlite3")).unwrap();
        connection
            .execute(
                "UPDATE meeting_segments SET audio_relative_path='audio/../outside.wav' WHERE id=?",
                [forged],
            )
            .unwrap();

        let (receipt, bytes) =
            bundle(&repository, "forged", MeetingReviewExportFormat::Markdown).unwrap();
        assert_eq!(
            (receipt.included_segments, receipt.omitted_segments),
            (1, 1)
        );
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        assert!(!names(&archive).iter().any(|name| name.contains("..")));
        assert_eq!(
            names(&archive)
                .iter()
                .filter(|name| name.starts_with("audio/"))
                .collect::<Vec<_>>(),
            vec![&format!("audio/segment-{honest:08}-me.wav")]
        );
        let manifest: serde_json::Value =
            serde_json::from_slice(&entry(&mut archive, MANIFEST_NAME)).unwrap();
        assert_eq!(manifest["audio"]["omittedSegments"][0]["segmentId"], forged);
        for index in 0..archive.len() {
            let mut contents = Vec::new();
            archive
                .by_index(index)
                .unwrap()
                .read_to_end(&mut contents)
                .unwrap();
            assert_ne!(contents, secret);
        }
    }

    #[test]
    fn audio_beyond_the_documented_limit_fails_closed() {
        let (_temporary, repository) = repository();
        session(&repository, "big", true);
        let (_, path) = clip(&repository, "big", MeetingSpeaker::Me, 0, 0);
        finish(&repository, "big");
        let length = std::fs::metadata(path).unwrap().len();
        let workspace = repository.workspace("big").unwrap();
        let attempt = |limit| {
            write_bundle_limited(
                &repository,
                &workspace,
                MeetingReviewExportFormat::Markdown,
                current_audio_revision(),
                limit,
                Cursor::new(Vec::new()),
            )
        };
        assert!(attempt(length).is_ok());
        assert!(attempt(length - 1).unwrap_err().contains("bundle limit"));
    }
}
