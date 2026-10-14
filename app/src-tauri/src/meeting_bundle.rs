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
//! Completeness contract: a segment whose WAV was already missing, pruned,
//! malformed, or outside the owned spool when the export reached it is omitted
//! and listed in `manifest.json` with `"complete": false` and a notice; the save
//! receipt reports the omitted count so the UI can say so. A bundle with no
//! audio at all is refused rather than written.
//!
//! Revocation contract: an export is one [`BundleOperation`] that re-checks, at
//! every segment and immediately before the atomic rename, that the meeting was
//! not deleted, pruned, or invalidated (the audio revision is unchanged and the
//! session still owns its audio) and that no capture started. Any failure
//! aborts and publishes nothing, so a deleted meeting's audio is never
//! published and a bundle is never marked complete over a revoked session.
//!
//! Resources: the operation opens and validates the database once and reuses
//! that connection for every per-segment ownership check. It holds a dedicated
//! single-slot bundle permit instead of the two-slot playback pool, so
//! two-channel playback keeps working while a bundle is being written.

use crate::meeting_audio::{
    audio_file_len, current_audio_revision, read_audio_whole, AudioChunk, AudioCursor,
    AudioManifest, AudioUnavailableReason, PlaybackChannel, AUDIO_UNAVAILABLE, BUNDLE_CAPTURE_BUSY,
    INVALIDATED, MAX_MANIFEST_CHUNKS,
};
use crate::meeting_review::{render_export, MeetingReviewExportFormat, MeetingWorkspace};
use crate::meeting_store::MeetingRepository;
use crate::State;
use rusqlite::Connection;
use serde::Serialize;
use std::io::{Seek, Write};
use std::sync::atomic::{AtomicBool, Ordering};
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
const BUNDLE_REVOKED: &str =
    "This meeting's audio changed or was deleted during the export. Nothing was saved. Try again.";
const BUNDLE_BUSY: &str = "Another meeting export is already running. Try again shortly.";

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

static BUNDLE_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Admission for bundle work. Deliberately separate from the two-slot playback
/// pool in `meeting_audio`: a long export must never make the two concurrent
/// channel requests of ordinary playback fail.
struct BundlePermit;
impl BundlePermit {
    fn acquire() -> Result<Self, String> {
        BUNDLE_ACTIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| Self)
            .map_err(|_| BUNDLE_BUSY.to_string())
    }
}
impl Drop for BundlePermit {
    fn drop(&mut self) {
        BUNDLE_ACTIVE.store(false, Ordering::Release);
    }
}

fn too_large() -> String {
    format!(
        "This meeting's audio is larger than the {} GiB bundle limit. Export the review document on its own instead.",
        MAX_BUNDLE_AUDIO_BYTES / (1024 * 1024 * 1024)
    )
}

fn write_failed<E>(_: E) -> String {
    WRITE_FAILED.into()
}

/// One preview or export of one meeting. Owns the operation-scoped checked
/// database connection and the admission state captured when it started.
pub(crate) struct BundleOperation<'a> {
    repository: &'a MeetingRepository,
    connection: Connection,
    /// Audio revision captured at admission; any later delete, prune, or
    /// invalidation changes it and aborts the operation.
    revision: u64,
    /// True once a recording, dictation, or other voice task has started.
    capture_busy: &'a dyn Fn() -> bool,
}

impl<'a> BundleOperation<'a> {
    pub(crate) fn open(
        repository: &'a MeetingRepository,
        revision: u64,
        capture_busy: &'a dyn Fn() -> bool,
    ) -> Result<Self, String> {
        Ok(Self {
            repository,
            connection: repository.open_audio_connection()?,
            revision,
            capture_busy,
        })
    }

    /// Fail unless the meeting is exactly as it was at admission and capture is
    /// still idle. Called at every bounded step and again right before the
    /// rename that publishes the file.
    pub(crate) fn ensure_current(&self, session_id: &str) -> Result<(), String> {
        if (self.capture_busy)() {
            return Err(BUNDLE_CAPTURE_BUSY.into());
        }
        if current_audio_revision() != self.revision
            || !self
                .repository
                .audio_session_intact_on(&self.connection, session_id)?
        {
            return Err(BUNDLE_REVOKED.into());
        }
        Ok(())
    }

    /// A segment read said "unavailable". That is an omission only if the file
    /// was already gone while the meeting is untouched; a revoked meeting or a
    /// vanished segment row aborts the export instead.
    fn confirm_already_unavailable(&self, session_id: &str, segment_id: i64) -> Result<(), String> {
        self.ensure_current(session_id)?;
        if self
            .repository
            .audio_segment_present_on(&self.connection, session_id, segment_id)?
        {
            Ok(())
        } else {
            Err(BUNDLE_REVOKED.into())
        }
    }

    /// Every retained segment the playback manifest lists for a finished
    /// session, or a clear refusal. Reuses the manifest query so "retained"
    /// means the same thing here as in playback.
    fn retained_chunks(&self, session_id: &str) -> Result<Vec<AudioChunk>, String> {
        let mut chunks = Vec::new();
        let mut cursor: Option<AudioCursor> = None;
        loop {
            match self.repository.audio_manifest_on(
                &self.connection,
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
                        return Err(
                            "This meeting has too many audio segments for one bundle.".into()
                        );
                    }
                    match next_cursor {
                        Some(next) => cursor = Some(next),
                        None => return Ok(chunks),
                    }
                }
            }
        }
    }

    /// Size and completeness of the bundle the user is about to save.
    pub(crate) fn preview(&self, session_id: &str) -> Result<BundlePreview, String> {
        let mut preview = BundlePreview {
            audio_segments: 0,
            audio_bytes: 0,
            unavailable_segments: 0,
            max_audio_bytes: MAX_BUNDLE_AUDIO_BYTES,
        };
        for chunk in self.retained_chunks(session_id)? {
            self.ensure_current(session_id)?;
            match audio_file_len(
                self.repository,
                &self.connection,
                session_id,
                chunk.segment_id,
            ) {
                Ok(length) => {
                    preview.audio_segments += 1;
                    preview.audio_bytes += length;
                }
                Err(error) if error == AUDIO_UNAVAILABLE => {
                    self.confirm_already_unavailable(session_id, chunk.segment_id)?;
                    preview.unavailable_segments += 1;
                }
                Err(error) => return Err(error),
            }
        }
        if preview.audio_segments == 0 {
            return Err(NOTHING_TO_BUNDLE.into());
        }
        Ok(preview)
    }

    /// Write the bundle for `workspace` into `writer`.
    pub(crate) fn write<W: Write + Seek>(
        &self,
        workspace: &MeetingWorkspace,
        format: MeetingReviewExportFormat,
        writer: W,
    ) -> Result<BundleReceipt, String> {
        self.write_limited(workspace, format, MAX_BUNDLE_AUDIO_BYTES, writer)
    }

    fn write_limited<W: Write + Seek>(
        &self,
        workspace: &MeetingWorkspace,
        format: MeetingReviewExportFormat,
        max_audio_bytes: u64,
        writer: W,
    ) -> Result<BundleReceipt, String> {
        use zip::write::SimpleFileOptions;
        let session_id = workspace.session.id.as_str();
        if !workspace.session.retain_audio {
            return Err(
                "Audio was not retained for this meeting, so it cannot be exported with audio."
                    .into(),
            );
        }
        let chunks = self.retained_chunks(session_id)?;
        // Byte-for-byte the same document the standalone export writes.
        let document = render_export(workspace, format)?;
        let document_name = format!("review.{}", format.extension());
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

        let mut zip = zip::ZipWriter::new(writer);
        zip.start_file(document_name.as_str(), options)
            .map_err(write_failed)?;
        zip.write_all(document.as_bytes()).map_err(write_failed)?;

        let mut included = Vec::new();
        let mut omitted = Vec::new();
        let mut audio_bytes = 0_u64;
        for chunk in &chunks {
            self.ensure_current(session_id)?;
            let bytes = match read_audio_whole(
                self.repository,
                &self.connection,
                session_id,
                chunk.segment_id,
                self.revision,
            ) {
                Ok(bytes) => bytes,
                Err(error) if error == AUDIO_UNAVAILABLE => {
                    self.confirm_already_unavailable(session_id, chunk.segment_id)?;
                    omitted.push(serde_json::json!({
                        "segmentId": chunk.segment_id,
                        "channel": chunk.channel,
                        "startMs": chunk.start_ms,
                        "endMs": chunk.end_ms,
                        "reason": OMITTED_REASON,
                    }));
                    continue;
                }
                Err(error) if error == INVALIDATED => return Err(BUNDLE_REVOKED.into()),
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
        // The last read must not be the last look: a deletion after it would
        // otherwise leave a bundle that claims to be complete.
        self.ensure_current(session_id)?;
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
}

#[tauri::command]
pub async fn get_meeting_bundle_preview(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    id: String,
    state: tauri::State<'_, State>,
) -> Result<BundlePreview, String> {
    crate::meeting_audio::require_main_window(window.label())?;
    if crate::meeting_audio::capture_busy(&state) {
        return Err(BUNDLE_CAPTURE_BUSY.into());
    }
    let repository = state.meeting_store.repository()?;
    let permit = BundlePermit::acquire()?;
    let revision = current_audio_revision();
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        let busy = || crate::meeting_audio::capture_busy(&app.state::<State>());
        let operation = BundleOperation::open(&repository, revision, &busy)?;
        operation.preview(id.trim())
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
        return Err(BUNDLE_CAPTURE_BUSY.into());
    }
    let repository = state.meeting_store.repository()?;
    let permit = BundlePermit::acquire()?;
    let revision = current_audio_revision();
    let receipt = tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        let busy = || crate::meeting_audio::capture_busy(&app.state::<State>());
        let operation = BundleOperation::open(&repository, revision, &busy)?;
        let workspace = repository.workspace(id.trim())?;
        publish_bundle(&operation, &workspace, format, &path)
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

/// Stream the bundle to a unique temp sibling and rename it into place, after a
/// last check that the meeting is untouched and capture is idle.
fn publish_bundle(
    operation: &BundleOperation<'_>,
    workspace: &MeetingWorkspace,
    format: MeetingReviewExportFormat,
    path: &std::path::Path,
) -> Result<BundleReceipt, String> {
    let mut receipt = None;
    crate::commands::export::publish_with(
        path,
        |file| {
            let written = operation.write(workspace, format, &mut *file)?;
            let bytes = written.bytes;
            receipt = Some(written);
            Ok(bytes)
        },
        || operation.ensure_current(workspace.session.id.as_str()),
    )?;
    receipt.ok_or_else(|| WRITE_FAILED.to_string())
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

    fn idle() -> bool {
        false
    }

    fn preview_bundle(
        repository: &MeetingRepository,
        session_id: &str,
    ) -> Result<BundlePreview, String> {
        BundleOperation::open(repository, current_audio_revision(), &idle)?.preview(session_id)
    }

    fn bundle(
        repository: &MeetingRepository,
        session_id: &str,
        format: MeetingReviewExportFormat,
    ) -> Result<(BundleReceipt, Vec<u8>), String> {
        let workspace = repository.workspace(session_id).unwrap();
        let mut output = Cursor::new(Vec::new());
        let receipt = BundleOperation::open(repository, current_audio_revision(), &idle)?.write(
            &workspace,
            format,
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
            BundleOperation::open(&repository, current_audio_revision(), &idle)?.write_limited(
                &workspace,
                MeetingReviewExportFormat::Markdown,
                limit,
                Cursor::new(Vec::new()),
            )
        };
        assert!(attempt(length).is_ok());
        assert!(attempt(length - 1).unwrap_err().contains("bundle limit"));
    }

    /// A meeting with `count` finished segments on alternating channels.
    fn finished_meeting(repository: &MeetingRepository, id: &str, count: u64) -> Vec<i64> {
        session(repository, id, true);
        let ids = (0..count)
            .map(|index| {
                let channel = if index % 2 == 0 {
                    MeetingSpeaker::Me
                } else {
                    MeetingSpeaker::Them
                };
                clip(repository, id, channel, index, index * 1_000).0
            })
            .collect();
        finish(repository, id);
        ids
    }

    fn destination_is_untouched(dir: &std::path::Path) {
        assert_eq!(
            std::fs::read_dir(dir).unwrap().count(),
            0,
            "nothing, not even a temp file, may be left behind"
        );
    }

    #[test]
    fn deleting_the_meeting_between_segments_aborts_and_publishes_nothing() {
        use std::cell::Cell;
        let (_temporary, repository) = repository();
        finished_meeting(&repository, "between", 3);
        let workspace = repository.workspace("between").unwrap();
        let destination = tempfile::tempdir().unwrap();
        let path = destination.path().join("meeting.zip");

        // One idle check per segment: delete just before the third read.
        let checks = Cell::new(0);
        let busy = || {
            checks.set(checks.get() + 1);
            if checks.get() == 3 {
                repository.delete_session("between").unwrap();
            }
            false
        };
        let operation =
            BundleOperation::open(&repository, current_audio_revision(), &busy).unwrap();
        let error = publish_bundle(
            &operation,
            &workspace,
            MeetingReviewExportFormat::Json,
            &path,
        )
        .unwrap_err();
        assert_eq!(error, BUNDLE_REVOKED);
        destination_is_untouched(destination.path());
    }

    #[test]
    fn a_segment_revoked_after_admission_is_an_abort_not_an_omission() {
        use std::cell::Cell;
        let (_temporary, repository) = repository();
        let ids = finished_meeting(&repository, "revoked", 3);
        let workspace = repository.workspace("revoked").unwrap();
        // Remove only the second segment's row behind the exporter's back,
        // without bumping the audio revision: the read reports "unavailable",
        // which must not be mistaken for a file that was already missing.
        let checks = Cell::new(0);
        let busy = || {
            checks.set(checks.get() + 1);
            if checks.get() == 2 {
                let connection =
                    rusqlite::Connection::open(repository.root().join("meetings.sqlite3")).unwrap();
                connection
                    .execute("DELETE FROM meeting_segments WHERE id=?", [ids[1]])
                    .unwrap();
            }
            false
        };
        let operation =
            BundleOperation::open(&repository, current_audio_revision(), &busy).unwrap();
        let error = operation
            .write(
                &workspace,
                MeetingReviewExportFormat::Markdown,
                Cursor::new(Vec::new()),
            )
            .unwrap_err();
        assert_eq!(error, BUNDLE_REVOKED);
    }

    #[test]
    fn deleting_the_meeting_after_the_last_read_but_before_publication_publishes_nothing() {
        let (_temporary, repository) = repository();
        finished_meeting(&repository, "late", 2);
        let workspace = repository.workspace("late").unwrap();
        let destination = tempfile::tempdir().unwrap();
        let path = destination.path().join("meeting.zip");
        std::fs::write(&path, "previous export").unwrap();

        let operation =
            BundleOperation::open(&repository, current_audio_revision(), &idle).unwrap();
        let mut written = None;
        let error = crate::commands::export::publish_with(
            &path,
            |file| {
                let receipt = operation.write(&workspace, MeetingReviewExportFormat::Text, file)?;
                // Every read succeeded and the bundle says complete; now the
                // meeting is deleted before the rename.
                repository.delete_session("late").unwrap();
                written = Some(receipt.bytes);
                Ok(receipt.bytes)
            },
            || operation.ensure_current("late"),
        )
        .unwrap_err();
        assert_eq!(error, BUNDLE_REVOKED);
        assert!(written.is_some(), "the bundle was fully written first");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "previous export");
        assert_eq!(std::fs::read_dir(destination.path()).unwrap().count(), 1);
    }

    #[test]
    fn capture_starting_after_admission_aborts_mid_export_and_before_publication() {
        use std::cell::Cell;
        let (_temporary, repository) = repository();
        finished_meeting(&repository, "capture", 4);
        let workspace = repository.workspace("capture").unwrap();
        let destination = tempfile::tempdir().unwrap();
        let path = destination.path().join("meeting.zip");

        // Capture begins while the second segment is being processed.
        let checks = Cell::new(0);
        let busy = || {
            checks.set(checks.get() + 1);
            checks.get() >= 2
        };
        let operation =
            BundleOperation::open(&repository, current_audio_revision(), &busy).unwrap();
        let error = publish_bundle(
            &operation,
            &workspace,
            MeetingReviewExportFormat::Srt,
            &path,
        )
        .unwrap_err();
        assert_eq!(error, BUNDLE_CAPTURE_BUSY);
        assert_eq!(checks.get(), 2, "no further segment was read");
        destination_is_untouched(destination.path());

        // Capture begins only after the last segment: the final pre-rename
        // check still refuses.
        let flag = Cell::new(false);
        let busy = || flag.get();
        let operation =
            BundleOperation::open(&repository, current_audio_revision(), &busy).unwrap();
        let error = crate::commands::export::publish_with(
            &path,
            |file| {
                let receipt = operation.write(&workspace, MeetingReviewExportFormat::Srt, file)?;
                flag.set(true);
                Ok(receipt.bytes)
            },
            || operation.ensure_current("capture"),
        )
        .unwrap_err();
        assert_eq!(error, BUNDLE_CAPTURE_BUSY);
        destination_is_untouched(destination.path());
    }

    #[test]
    fn an_export_validates_the_database_once_however_many_segments() {
        let (_temporary, repository) = repository();
        finished_meeting(&repository, "small", 2);
        finished_meeting(&repository, "large", 24);
        let measure = |id: &str| {
            let workspace = repository.workspace(id).unwrap();
            let before = crate::meeting_store::checked_opens();
            let operation =
                BundleOperation::open(&repository, current_audio_revision(), &idle).unwrap();
            operation.preview(id).unwrap();
            operation
                .write(
                    &workspace,
                    MeetingReviewExportFormat::Markdown,
                    Cursor::new(Vec::new()),
                )
                .unwrap();
            crate::meeting_store::checked_opens() - before
        };
        assert_eq!(measure("small"), 1);
        assert_eq!(measure("large"), 1);
    }

    #[test]
    fn a_running_export_does_not_take_playback_slots() {
        use crate::meeting_audio::AudioReadPermit;
        use std::sync::mpsc;
        let (_temporary, repository) = repository();
        finished_meeting(&repository, "playing", 3);
        let workspace = repository.workspace("playing").unwrap();

        let (reached, reached_rx) = mpsc::channel();
        let (resume, resume_rx) = mpsc::channel::<()>();
        let permit = BundlePermit::acquire().unwrap();
        let exporter = {
            let repository = repository.clone();
            std::thread::spawn(move || {
                let _permit = permit;
                // Park the export mid-way, between two segments.
                let checks = std::cell::Cell::new(0);
                let busy = || {
                    checks.set(checks.get() + 1);
                    if checks.get() == 2 {
                        reached.send(()).unwrap();
                        resume_rx.recv().unwrap();
                    }
                    false
                };
                BundleOperation::open(&repository, current_audio_revision(), &busy)
                    .unwrap()
                    .write(
                        &workspace,
                        MeetingReviewExportFormat::Markdown,
                        Cursor::new(Vec::new()),
                    )
            })
        };
        reached_rx.recv().unwrap();

        // Mid-export: both channels of ordinary playback are still admitted,
        // and a second bundle is the only thing refused.
        let me = AudioReadPermit::acquire().unwrap();
        let them = AudioReadPermit::acquire().unwrap();
        assert!(AudioReadPermit::acquire().is_err());
        drop((me, them));
        assert_eq!(BundlePermit::acquire().err().as_deref(), Some(BUNDLE_BUSY));

        resume.send(()).unwrap();
        let receipt = exporter.join().unwrap().unwrap();
        assert_eq!(receipt.included_segments, 3);
        assert!(BundlePermit::acquire().is_ok());
    }
}
