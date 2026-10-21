//! ZIP export of a finished meeting's review document plus its retained WAVs.
//!
//! Audio is read only through `meeting_audio::read_whole_audio_segment`, the
//! session-scoped boundary playback uses: the SQLite join, the exact generated
//! `audio_path`, no-follow opens, bounded RIFF validation, and identity
//! revalidation. The archive stores every WAV byte-for-byte (no compression,
//! transcoding, mixing, or synthesis) beside the unmodified `render_export`
//! document. A retained file that is simply absent, or that is still pending
//! transcription (playback refuses to read those), is omitted and listed in
//! `audio/MISSING-AUDIO.txt`; any unsafe or malformed file aborts the export.
//!
//! The archive is built in an exclusively created, uniquely named private
//! sibling. Capture is re-checked before every segment read and again under the
//! audio mutation lock, where the session is also revalidated, immediately
//! before the rename; deleting retained audio takes the same lock, so a delete
//! can neither complete mid-publish nor be followed by a publish of its audio.

use crate::meeting_audio::{
    capture_busy, current_audio_revision, lock_audio_mutation, read_whole_audio_segment,
    validate_session_id, AudioReadPermit, AudioSegmentError, MAX_AUDIO_FILE_BYTES,
};
use crate::meeting_review::{render_export, MeetingReviewExportFormat, MeetingWorkspace};
use crate::meeting_store::{MeetingRepository, MeetingSpeaker};
use crate::State;
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{Seek, Write};
use std::path::Path;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

/// Most retained chunks one export may contain. Capture chunks are at most
/// 15.5 seconds, so this covers over 35 hours of one channel while keeping the
/// archive below the 65,535-entry classic ZIP limit.
pub(crate) const MAX_EXPORT_AUDIO_SEGMENTS: usize = 8_192;
/// Most WAV bytes (headers included) one export may contain: 1 GiB, roughly
/// nine hours of single-channel capture. The ZIP is never larger than this plus
/// the review document (at most 8 MiB), the notice, and ZIP framing, so it
/// stays inside classic (non-ZIP64) limits.
pub(crate) const MAX_EXPORT_AUDIO_BYTES: u64 = 1024 * 1024 * 1024;
pub(crate) const TOO_LARGE: &str =
    "This meeting's retained audio is too large to export in one ZIP.";
const NOTHING_AVAILABLE: &str =
    "None of this meeting's retained audio files are available to export, so nothing was exported.";
const BUSY: &str =
    "Pause recording or finish the active voice task before exporting meeting audio.";
const CHANGED: &str =
    "Meeting audio changed or was deleted during the export. Nothing was exported; try again.";
const MISSING_NOTICE_PATH: &str = "audio/MISSING-AUDIO.txt";
/// PCM16 mono at 16 kHz is 32 bytes per millisecond.
const BYTES_PER_MS: u64 = 32;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MeetingAudioExportFormat {
    Markdown,
    Text,
    Json,
}

impl MeetingAudioExportFormat {
    fn review_format(self) -> MeetingReviewExportFormat {
        match self {
            Self::Markdown => MeetingReviewExportFormat::Markdown,
            Self::Text => MeetingReviewExportFormat::Text,
            Self::Json => MeetingReviewExportFormat::Json,
        }
    }

    fn document_name(self) -> String {
        format!("meeting-review.{}", self.review_format().extension())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExportSegment {
    pub segment_id: i64,
    pub channel: MeetingSpeaker,
    pub sequence: u64,
    pub start_ms: u64,
    pub end_ms: u64,
    /// Still awaiting transcription: playback refuses to read it, so the
    /// export lists it as omitted instead of reading it.
    pub pending: bool,
}

impl ExportSegment {
    fn file_name(&self) -> String {
        format!("{}-{:08}.wav", self.channel.as_db(), self.sequence)
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MeetingAudioExportSummary {
    pub bytes: u64,
    pub audio_files: u32,
    pub missing_files: u32,
    pub pending_files: u32,
}

fn zip_error(_: zip::result::ZipError) -> String {
    "The meeting audio export could not be written.".into()
}

fn io_error(_: std::io::Error) -> String {
    "The meeting audio export could not be written.".into()
}

fn missing_notice(missing: &[&ExportSegment], pending: &[&ExportSegment]) -> String {
    let mut notice = String::from(
        "Murmur meeting audio export\n\n\
         The audio segments listed below are not in this archive, and nothing was\n\
         substituted for them.\n\n",
    );
    let mut section = |heading: &str, segments: &[&ExportSegment]| {
        if segments.is_empty() {
            return;
        }
        notice.push_str(heading);
        notice.push('\n');
        for segment in segments {
            notice.push_str(&format!(
                "- audio/{} (segment {}, {}-{} ms)\n",
                segment.file_name(),
                segment.segment_id,
                segment.start_ms,
                segment.end_ms
            ));
        }
        notice.push('\n');
    };
    section("Not found on this Mac when the export ran:", missing);
    section(
        "Still waiting for transcription, so the audio was not read:",
        pending,
    );
    notice
}

/// What the archive holds, kept so the locked pre-publish check can confirm the
/// exported files are still the session's retained audio.
pub(crate) struct BuiltExport {
    summary: MeetingAudioExportSummary,
    segments: Vec<ExportSegment>,
    written: Vec<(i64, u64)>,
}

/// Write the archive for `workspace` into `writer`. Nothing is retained on
/// failure beyond what the caller's atomic publish discards. `busy` is polled
/// before every segment read.
pub(crate) fn write_audio_export<W: Write + Seek>(
    repository: &MeetingRepository,
    workspace: &MeetingWorkspace,
    format: MeetingAudioExportFormat,
    writer: W,
    revision: u64,
    busy: &dyn Fn() -> bool,
) -> Result<BuiltExport, String> {
    let session_id = workspace.session.id.as_str();
    let segments = repository.audio_export_segments(session_id)?;
    let expected = segments
        .iter()
        .filter(|segment| !segment.pending)
        .map(|segment| (segment.end_ms - segment.start_ms) * BYTES_PER_MS)
        .sum::<u64>();
    if expected > MAX_EXPORT_AUDIO_BYTES {
        return Err(TOO_LARGE.into());
    }
    // Render before touching any audio so a document failure writes nothing.
    let document = render_export(workspace, format.review_format())?;

    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .unix_permissions(0o644);
    let mut archive = ZipWriter::new(writer);
    archive
        .start_file(format.document_name(), options)
        .map_err(zip_error)?;
    archive.write_all(document.as_bytes()).map_err(io_error)?;

    let mut total = 0_u64;
    let mut written = Vec::new();
    let mut missing = Vec::new();
    let mut pending = Vec::new();
    for segment in &segments {
        if segment.pending {
            pending.push(segment);
            continue;
        }
        if busy() {
            return Err(BUSY.into());
        }
        match read_whole_audio_segment(repository, session_id, segment.segment_id, revision) {
            Ok(bytes) => {
                total += bytes.len() as u64;
                if total > MAX_EXPORT_AUDIO_BYTES || bytes.len() as u64 > MAX_AUDIO_FILE_BYTES {
                    return Err(TOO_LARGE.into());
                }
                archive
                    .start_file(format!("audio/{}", segment.file_name()), options)
                    .map_err(zip_error)?;
                archive.write_all(&bytes).map_err(io_error)?;
                written.push((segment.segment_id, bytes.len() as u64));
            }
            Err(AudioSegmentError::Missing) => missing.push(segment),
            Err(AudioSegmentError::Unavailable(message)) => return Err(message),
        }
    }
    if written.is_empty() {
        return Err(NOTHING_AVAILABLE.into());
    }
    if !missing.is_empty() || !pending.is_empty() {
        archive
            .start_file(MISSING_NOTICE_PATH, options)
            .map_err(zip_error)?;
        archive
            .write_all(missing_notice(&missing, &pending).as_bytes())
            .map_err(io_error)?;
    }
    let (missing_files, pending_files) = (missing.len() as u32, pending.len() as u32);
    let mut writer = archive.finish().map_err(zip_error)?;
    let bytes = writer.stream_position().map_err(io_error)?;
    Ok(BuiltExport {
        summary: MeetingAudioExportSummary {
            bytes,
            audio_files: written.len() as u32,
            missing_files,
            pending_files,
        },
        segments,
        written,
    })
}

/// Confirm, under the audio mutation lock, that the session still holds exactly
/// the retained audio the archive was built from: the session, its segment
/// list, and every exported file must still be the same. This never trusts an
/// unchanged revision, because a delete bumps the revision before it removes
/// anything, so an equal revision does not prove the audio still exists.
fn still_current(
    repository: &MeetingRepository,
    session_id: &str,
    built: &BuiltExport,
) -> Result<(), String> {
    let segments = repository
        .audio_export_segments(session_id)
        .map_err(|_| CHANGED.to_string())?;
    if segments != built.segments {
        return Err(CHANGED.into());
    }
    for (segment_id, length) in &built.written {
        let reference = repository
            .audio_reference(session_id, *segment_id)
            .map_err(|_| CHANGED.to_string())?;
        let intact = std::fs::symlink_metadata(&reference.path)
            .is_ok_and(|metadata| metadata.is_file() && metadata.len() == *length);
        if !intact {
            return Err(CHANGED.into());
        }
    }
    Ok(())
}

/// Execution-time gates for one export: capture is polled while reading and
/// again at publication. `before_revision` runs just before the starting audio
/// revision is sampled, and `before_publish` after the archive is built and
/// before the publish lock is taken (test seams, no-ops in production).
pub(crate) struct ExportGates<'a> {
    pub busy: &'a dyn Fn() -> bool,
    pub before_revision: &'a dyn Fn(),
    pub before_publish: &'a dyn Fn(),
}

/// A private temp sibling of `path`, exclusively created so concurrent exports
/// to one destination (or a pre-existing file) never share or truncate a file.
fn create_temp(path: &Path) -> Result<(File, std::path::PathBuf), String> {
    use std::os::unix::fs::OpenOptionsExt;
    for _ in 0..8 {
        let temp = path.with_file_name(format!(
            ".murmur-audio-export-{}.tmp",
            uuid::Uuid::new_v4().simple()
        ));
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&temp)
        {
            Ok(file) => return Ok((file, temp)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => break,
        }
    }
    Err("The export file could not be created.".into())
}

/// Build into a private temp sibling and rename it into place, so a refusal or
/// failure part-way never leaves a partial archive at the chosen path.
fn publish_atomically(
    path: &Path,
    gates: &ExportGates<'_>,
    build: impl FnOnce(&mut File) -> Result<BuiltExport, String>,
    verify: impl FnOnce(&BuiltExport) -> Result<(), String>,
) -> Result<MeetingAudioExportSummary, String> {
    crate::commands::export::validate_export_path_for(path, &["zip"])?;
    let (mut file, temp) = create_temp(path)?;
    let built = build(&mut file).and_then(|built| {
        file.sync_all()
            .map_err(|_| "The export file could not be saved.".to_string())?;
        Ok(built)
    });
    drop(file);
    let built = match built {
        Ok(built) => built,
        Err(error) => {
            let _ = std::fs::remove_file(&temp);
            return Err(error);
        }
    };
    (gates.before_publish)();
    let result = {
        let _publish = lock_audio_mutation();
        if (gates.busy)() {
            Err(BUSY.to_string())
        } else {
            verify(&built).and_then(|()| {
                std::fs::rename(&temp, path)
                    .map_err(|_| "The export file could not be saved.".to_string())
            })
        }
    };
    match result {
        Ok(()) => Ok(built.summary),
        Err(error) => {
            let _ = std::fs::remove_file(&temp);
            Err(error)
        }
    }
}

pub(crate) fn export_to_path(
    repository: &MeetingRepository,
    session_id: &str,
    format: MeetingAudioExportFormat,
    path: &Path,
    gates: &ExportGates<'_>,
) -> Result<MeetingAudioExportSummary, String> {
    validate_session_id(session_id)?;
    (gates.before_revision)();
    // Sampled under the mutation lock: a delete holds it from its revocation to
    // the removal, so this waits out any delete in progress and then sees the
    // session as it really is, never an already-revoked revision over audio
    // that is about to disappear.
    let revision = {
        let _lock = lock_audio_mutation();
        current_audio_revision()
    };
    // Refuse before creating any file when the session cannot export audio.
    repository.audio_export_segments(session_id)?;
    let workspace = repository.workspace(session_id)?;
    publish_atomically(
        path,
        gates,
        |file| write_audio_export(repository, &workspace, format, file, revision, gates.busy),
        |built| still_current(repository, session_id, built),
    )
}

#[tauri::command]
pub async fn save_meeting_audio_export(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    id: String,
    format: MeetingAudioExportFormat,
    path: String,
    state: tauri::State<'_, State>,
) -> Result<MeetingAudioExportSummary, String> {
    if window.label() != "main" {
        return Err("Meeting audio is only available from the main window.".into());
    }
    let id = id.trim().to_string();
    validate_session_id(&id)?;
    if state.meetings.status().session_id.as_deref() == Some(id.as_str())
        && state.meetings.is_active()
    {
        return Err("Stop this meeting before exporting its audio.".into());
    }
    if capture_busy(&state) {
        return Err(BUSY.into());
    }
    let repository = state.meeting_store.repository()?;
    let permit = AudioReadPermit::acquire()?;
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        use tauri::Manager;
        let busy = || capture_busy(&app.state::<State>());
        let gates = ExportGates {
            busy: &busy,
            before_revision: &|| {},
            before_publish: &|| {},
        };
        let summary = export_to_path(&repository, &id, format, Path::new(&path), &gates)?;
        // Content-free: counts only, never names, paths, or audio.
        tracing::info!(
            target: "pipeline",
            audio_files = summary.audio_files,
            missing_files = summary.missing_files,
            pending_files = summary.pending_files,
            bytes = summary.bytes,
            "meeting audio export written"
        );
        Ok(summary)
    })
    .await
    .map_err(|_| "The meeting audio export stopped unexpectedly.".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meeting_store::MeetingSessionStatus;
    use std::cell::{Cell, RefCell};
    use std::io::Read;
    use std::path::PathBuf;

    /// Export with gates that never report capture and do nothing at publish.
    fn export_to_path(
        repository: &MeetingRepository,
        session_id: &str,
        format: MeetingAudioExportFormat,
        path: &Path,
    ) -> Result<MeetingAudioExportSummary, String> {
        super::export_to_path(
            repository,
            session_id,
            format,
            path,
            &ExportGates {
                busy: &|| false,
                before_revision: &|| {},
                before_publish: &|| {},
            },
        )
    }

    /// A segment row left `pending`, with its real WAV on disk.
    fn pending_clip(
        repository: &MeetingRepository,
        session_id: &str,
        channel: MeetingSpeaker,
        sequence: u64,
        start_ms: u64,
        duration_ms: u64,
    ) -> PathBuf {
        let relative = format!("audio/{session_id}/{}-{sequence:08}.wav", channel.as_db());
        repository
            .insert_pending_segment(
                session_id,
                channel,
                sequence,
                start_ms,
                start_ms + duration_ms,
                &relative,
            )
            .unwrap();
        let path = repository.root().join(relative);
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
        for sample in 0..duration_ms * 16 {
            writer.write_sample((sample % 1_000) as i16).unwrap();
        }
        writer.finalize().unwrap();
        path
    }

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

    fn finish(repository: &MeetingRepository, id: &str) {
        repository
            .finish_session(id, MeetingSessionStatus::Complete, None)
            .unwrap();
    }

    /// A real generated-format WAV and its segment row.
    fn clip(
        repository: &MeetingRepository,
        session_id: &str,
        channel: MeetingSpeaker,
        sequence: u64,
        start_ms: u64,
        duration_ms: u64,
    ) -> (i64, PathBuf) {
        let (id, path) = row(
            repository,
            session_id,
            channel,
            sequence,
            start_ms,
            duration_ms,
        );
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
        for sample in 0..duration_ms * 16 {
            writer
                .write_sample(((sample + sequence * 7) % 1_000) as i16)
                .unwrap();
        }
        writer.finalize().unwrap();
        (id, path)
    }

    /// A segment row whose WAV is not written.
    fn row(
        repository: &MeetingRepository,
        session_id: &str,
        channel: MeetingSpeaker,
        sequence: u64,
        start_ms: u64,
        duration_ms: u64,
    ) -> (i64, PathBuf) {
        let relative = format!("audio/{session_id}/{}-{sequence:08}.wav", channel.as_db());
        let id = repository
            .insert_pending_segment(
                session_id,
                channel,
                sequence,
                start_ms,
                start_ms + duration_ms,
                &relative,
            )
            .unwrap();
        repository
            .finalize_segment(id, "synthetic test speech", true)
            .unwrap();
        (id, repository.root().join(relative))
    }

    /// Many final audio-bearing rows in one transaction, with no WAV files.
    fn bulk_rows(repository: &MeetingRepository, session_id: &str, count: usize, duration_ms: u64) {
        let mut connection =
            rusqlite::Connection::open(repository.root().join("meetings.sqlite3")).unwrap();
        let transaction = connection.transaction().unwrap();
        for sequence in 0..count as u64 {
            transaction
                .execute(
                    "INSERT INTO meeting_segments(session_id, speaker, sequence, start_ms, end_ms, status, text, audio_relative_path) VALUES (?1, 'me', ?2, ?3, ?4, 'final', 'x', ?5)",
                    rusqlite::params![
                        session_id,
                        sequence as i64,
                        (sequence * duration_ms) as i64,
                        ((sequence + 1) * duration_ms) as i64,
                        format!("audio/{session_id}/me-{sequence:08}.wav"),
                    ],
                )
                .unwrap();
        }
        transaction.commit().unwrap();
    }

    fn fixture() -> (tempfile::TempDir, MeetingRepository, Vec<PathBuf>) {
        let (temporary, repository) = repository();
        session(&repository, "retained", true);
        let paths = vec![
            clip(&repository, "retained", MeetingSpeaker::Me, 0, 0, 1_000).1,
            clip(&repository, "retained", MeetingSpeaker::Them, 0, 500, 1_500).1,
            clip(&repository, "retained", MeetingSpeaker::Me, 1, 2_000, 750).1,
        ];
        finish(&repository, "retained");
        (temporary, repository, paths)
    }

    fn destination(temporary: &tempfile::TempDir) -> PathBuf {
        let directory = temporary.path().canonicalize().unwrap().join("exports");
        std::fs::create_dir_all(&directory).unwrap();
        directory.join("meeting.zip")
    }

    fn entries(path: &Path) -> Vec<(String, Vec<u8>)> {
        let mut archive = zip::ZipArchive::new(File::open(path).unwrap()).unwrap();
        (0..archive.len())
            .map(|index| {
                let mut entry = archive.by_index(index).unwrap();
                assert_eq!(entry.compression(), CompressionMethod::Stored);
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes).unwrap();
                (entry.name().to_string(), bytes)
            })
            .collect()
    }

    fn directory_names(path: &Path) -> Vec<String> {
        let mut names = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    fn names(archive: &[(String, Vec<u8>)]) -> Vec<&str> {
        archive.iter().map(|entry| entry.0.as_str()).collect()
    }

    #[test]
    fn exports_the_exact_document_and_every_retained_wav_for_each_format() {
        let (temporary, repository, paths) = fixture();
        let workspace = repository.workspace("retained").unwrap();
        for (format, review, name) in [
            (
                MeetingAudioExportFormat::Markdown,
                MeetingReviewExportFormat::Markdown,
                "meeting-review.md",
            ),
            (
                MeetingAudioExportFormat::Text,
                MeetingReviewExportFormat::Text,
                "meeting-review.txt",
            ),
            (
                MeetingAudioExportFormat::Json,
                MeetingReviewExportFormat::Json,
                "meeting-review.json",
            ),
        ] {
            let path = destination(&temporary);
            let summary = export_to_path(&repository, "retained", format, &path).unwrap();
            let archive = entries(&path);
            assert_eq!(
                names(&archive),
                [
                    name,
                    "audio/me-00000000.wav",
                    "audio/them-00000000.wav",
                    "audio/me-00000001.wav"
                ]
            );
            assert_eq!(
                archive[0].1,
                render_export(&workspace, review).unwrap().into_bytes()
            );
            // Timeline order: me@0, them@500, me@2000 match the written paths.
            for (entry, wav) in archive[1..].iter().zip(&paths) {
                assert_eq!(entry.1, std::fs::read(wav).unwrap());
            }
            assert_eq!(summary.audio_files, 3);
            assert_eq!(summary.missing_files, 0);
            assert_eq!(summary.bytes, std::fs::metadata(&path).unwrap().len());
            assert_eq!(directory_names(&path), ["meeting.zip"]);
        }
    }

    #[test]
    fn document_format_rejects_captions() {
        for value in ["markdown", "text", "json"] {
            let json = format!("\"{value}\"");
            assert!(serde_json::from_str::<MeetingAudioExportFormat>(&json).is_ok());
        }
        for value in ["srt", "vtt", "zip"] {
            let json = format!("\"{value}\"");
            assert!(serde_json::from_str::<MeetingAudioExportFormat>(&json).is_err());
        }
    }

    #[test]
    fn refuses_unretained_active_unknown_and_empty_sessions_without_creating_a_file() {
        let (temporary, repository) = repository();
        session(&repository, "unretained", false);
        row(&repository, "unretained", MeetingSpeaker::Me, 0, 0, 1_000);
        finish(&repository, "unretained");
        session(&repository, "active", true);
        clip(&repository, "active", MeetingSpeaker::Me, 0, 0, 1_000);
        session(&repository, "empty", true);
        finish(&repository, "empty");
        let path = destination(&temporary);
        for (id, expected) in [
            ("unretained", "did not keep its audio"),
            ("active", "Stop this meeting"),
            ("empty", "no retained audio"),
            ("unknown", "unavailable"),
            ("../escape", "invalid"),
        ] {
            let error = export_to_path(&repository, id, MeetingAudioExportFormat::Markdown, &path)
                .unwrap_err();
            assert!(error.contains(expected), "{id}: {error}");
            assert!(directory_names(&path).is_empty(), "{id}");
        }
    }

    #[test]
    fn missing_wavs_are_omitted_and_listed_in_a_visible_notice() {
        let (temporary, repository, paths) = fixture();
        std::fs::remove_file(&paths[1]).unwrap();
        let path = destination(&temporary);
        let summary = export_to_path(
            &repository,
            "retained",
            MeetingAudioExportFormat::Text,
            &path,
        )
        .unwrap();
        assert_eq!((summary.audio_files, summary.missing_files), (2, 1));
        let archive = entries(&path);
        assert_eq!(
            names(&archive),
            [
                "meeting-review.txt",
                "audio/me-00000000.wav",
                "audio/me-00000001.wav",
                "audio/MISSING-AUDIO.txt"
            ]
        );
        let notice = String::from_utf8(archive[3].1.clone()).unwrap();
        assert!(notice.contains("audio/them-00000000.wav"));
        assert!(notice.contains("500-2000 ms"));
        assert!(!notice.contains("audio/me-0000000"));
        assert_eq!(archive[1].1, std::fs::read(&paths[0]).unwrap());
        assert_eq!(archive[2].1, std::fs::read(&paths[2]).unwrap());
    }

    #[test]
    fn exports_nothing_when_every_retained_wav_is_missing() {
        let (temporary, repository, _) = fixture();
        std::fs::remove_dir_all(repository.root().join("audio/retained")).unwrap();
        let path = destination(&temporary);
        std::fs::write(&path, "previous export").unwrap();
        let error = export_to_path(
            &repository,
            "retained",
            MeetingAudioExportFormat::Json,
            &path,
        )
        .unwrap_err();
        assert!(error.contains("nothing was exported"), "{error}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "previous export");
        assert_eq!(directory_names(&path), ["meeting.zip"]);
    }

    #[test]
    fn unsafe_references_and_files_fail_closed_through_the_audio_boundary() {
        use std::os::unix::fs::symlink;
        let (temporary, repository, paths) = fixture();
        let path = destination(&temporary);
        let outside = temporary.path().canonicalize().unwrap().join("outside.wav");
        std::fs::copy(&paths[0], &outside).unwrap();
        let export = || {
            export_to_path(
                &repository,
                "retained",
                MeetingAudioExportFormat::Text,
                &path,
            )
        };

        // A symlinked WAV is refused rather than followed or treated as missing.
        let original = std::fs::read(&paths[1]).unwrap();
        std::fs::remove_file(&paths[1]).unwrap();
        symlink(&outside, &paths[1]).unwrap();
        let error = export().unwrap_err();
        assert!(error.contains("unavailable"), "{error}");
        assert!(directory_names(&path).is_empty());
        std::fs::remove_file(&paths[1]).unwrap();

        // A hardlinked or malformed WAV is refused.
        std::fs::hard_link(&outside, &paths[1]).unwrap();
        assert!(export().is_err());
        std::fs::remove_file(&paths[1]).unwrap();
        std::fs::write(&paths[1], &original[..60]).unwrap();
        assert!(export().is_err());
        std::fs::write(&paths[1], &original).unwrap();
        assert!(export().is_ok());
        std::fs::remove_file(&path).unwrap();

        // Forged database references cannot reach outside the owned path.
        let connection =
            rusqlite::Connection::open(repository.root().join("meetings.sqlite3")).unwrap();
        for forged in [
            "audio/../outside.wav",
            "../outside.wav",
            "/etc/hosts",
            "audio/other/me-00000000.wav",
        ] {
            connection
                .execute(
                    "UPDATE meeting_segments SET audio_relative_path=? WHERE session_id='retained' AND speaker='them'",
                    [forged],
                )
                .unwrap();
            let error = export().unwrap_err();
            assert!(error.contains("unavailable"), "{forged}: {error}");
            assert!(directory_names(&path).is_empty(), "{forged}");
        }
    }

    #[test]
    fn export_bounds_refuse_before_writing() {
        let (temporary, repository) = repository();
        session(&repository, "bytes", true);
        // 2,200 maximum-length chunks exceed 1 GiB without writing any audio.
        bulk_rows(&repository, "bytes", 2_200, 15_500);
        finish(&repository, "bytes");
        let path = destination(&temporary);
        let error = export_to_path(
            &repository,
            "bytes",
            MeetingAudioExportFormat::Markdown,
            &path,
        )
        .unwrap_err();
        assert_eq!(error, TOO_LARGE);
        assert!(directory_names(&path).is_empty());

        session(&repository, "count", true);
        bulk_rows(&repository, "count", MAX_EXPORT_AUDIO_SEGMENTS + 1, 10);
        finish(&repository, "count");
        let error = export_to_path(
            &repository,
            "count",
            MeetingAudioExportFormat::Markdown,
            &path,
        )
        .unwrap_err();
        assert_eq!(error, TOO_LARGE);
        assert!(directory_names(&path).is_empty());
    }

    #[test]
    fn mid_export_failure_preserves_an_existing_destination_and_leaves_no_temp() {
        let (temporary, repository, paths) = fixture();
        let path = destination(&temporary);
        std::fs::write(&path, "previous export").unwrap();
        // The second file fails validation after the first was already written.
        std::fs::write(&paths[1], b"not a wav file").unwrap();
        assert!(export_to_path(
            &repository,
            "retained",
            MeetingAudioExportFormat::Markdown,
            &path
        )
        .is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "previous export");
        assert_eq!(directory_names(&path), ["meeting.zip"]);
    }

    #[test]
    fn only_zip_destinations_are_accepted() {
        let (temporary, repository, _) = fixture();
        let directory = destination(&temporary).parent().unwrap().to_path_buf();
        for name in ["meeting.md", "meeting", ".hidden.zip", "meeting.sh"] {
            let path = directory.join(name);
            assert!(
                export_to_path(
                    &repository,
                    "retained",
                    MeetingAudioExportFormat::Json,
                    &path
                )
                .is_err(),
                "{name}"
            );
        }
        assert!(export_to_path(
            &repository,
            "retained",
            MeetingAudioExportFormat::Json,
            Path::new("relative.zip")
        )
        .is_err());
        assert!(directory_names(&directory.join("x")).is_empty());
    }

    fn gated(
        repository: &MeetingRepository,
        path: &Path,
        format: MeetingAudioExportFormat,
        busy: &dyn Fn() -> bool,
        before_publish: &dyn Fn(),
    ) -> Result<MeetingAudioExportSummary, String> {
        super::export_to_path(
            repository,
            "retained",
            format,
            path,
            &ExportGates {
                busy,
                before_revision: &|| {},
                before_publish,
            },
        )
    }

    #[test]
    fn pending_wavs_are_listed_as_omitted_and_never_read() {
        let (temporary, repository) = repository();
        session(&repository, "mixed", true);
        let first = clip(&repository, "mixed", MeetingSpeaker::Me, 0, 0, 1_000).1;
        let pending = pending_clip(&repository, "mixed", MeetingSpeaker::Them, 0, 1_000, 500);
        finish(&repository, "mixed");
        let path = destination(&temporary);

        let summary =
            export_to_path(&repository, "mixed", MeetingAudioExportFormat::Json, &path).unwrap();
        assert_eq!(
            (
                summary.audio_files,
                summary.missing_files,
                summary.pending_files
            ),
            (1, 0, 1)
        );
        let archive = entries(&path);
        assert_eq!(
            names(&archive),
            [
                "meeting-review.json",
                "audio/me-00000000.wav",
                "audio/MISSING-AUDIO.txt"
            ]
        );
        assert_eq!(archive[1].1, std::fs::read(&first).unwrap());
        assert!(!archive
            .iter()
            .any(|entry| entry.1 == std::fs::read(&pending).unwrap()));
        let notice = String::from_utf8(archive[2].1.clone()).unwrap();
        assert!(notice.contains("Still waiting for transcription"));
        assert!(notice.contains("audio/them-00000000.wav"));
        assert!(notice.contains("1000-1500 ms"));
        assert!(!notice.contains("Not found on this Mac"));

        // Missing and pending are accounted separately in one notice.
        std::fs::remove_file(&first).unwrap();
        clip(&repository, "mixed", MeetingSpeaker::Me, 1, 2_000, 500);
        let summary =
            export_to_path(&repository, "mixed", MeetingAudioExportFormat::Text, &path).unwrap();
        assert_eq!(
            (
                summary.audio_files,
                summary.missing_files,
                summary.pending_files
            ),
            (1, 1, 1)
        );

        // A session whose only audio is pending has nothing safe to export.
        session(&repository, "only-pending", true);
        pending_clip(&repository, "only-pending", MeetingSpeaker::Me, 0, 0, 500);
        finish(&repository, "only-pending");
        std::fs::remove_file(&path).unwrap();
        let error = export_to_path(
            &repository,
            "only-pending",
            MeetingAudioExportFormat::Text,
            &path,
        )
        .unwrap_err();
        assert!(error.contains("nothing was exported"), "{error}");
        assert!(directory_names(&path).is_empty());
    }

    #[test]
    fn pre_existing_temp_files_are_never_reused_or_truncated() {
        use std::os::unix::fs::symlink;
        let (temporary, repository, _) = fixture();
        let path = destination(&temporary);
        let directory = path.parent().unwrap();
        let victim = directory.join("victim.txt");
        std::fs::write(&victim, "precious").unwrap();
        // The old deterministic temp names, planted as a symlink and a hardlink.
        symlink(&victim, directory.join(".meeting.zip.murmur-tmp")).unwrap();
        std::fs::hard_link(&victim, directory.join(".meeting.zip.tmp")).unwrap();

        export_to_path(
            &repository,
            "retained",
            MeetingAudioExportFormat::Markdown,
            &path,
        )
        .unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "precious");
        assert_eq!(entries(&path).len(), 4);
        assert_eq!(
            directory_names(&path),
            [
                ".meeting.zip.murmur-tmp",
                ".meeting.zip.tmp",
                "meeting.zip",
                "victim.txt"
            ]
        );
        let mode = std::fs::metadata(&path).unwrap();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(mode.permissions().mode() & 0o077, 0);
    }

    #[test]
    fn overlapping_exports_to_one_destination_never_corrupt_the_published_archive() {
        let (temporary, repository, paths) = fixture();
        let path = destination(&temporary);
        let inner = RefCell::new(None);
        let outer = gated(
            &repository,
            &path,
            MeetingAudioExportFormat::Text,
            &|| false,
            &|| {
                // A second export of the same destination runs to completion
                // while the first has built its archive but not yet published.
                let result = export_to_path(
                    &repository,
                    "retained",
                    MeetingAudioExportFormat::Json,
                    &path,
                );
                assert!(result.is_ok());
                *inner.borrow_mut() = Some(entries(&path));
            },
        )
        .unwrap();
        let inner = inner.into_inner().unwrap();
        assert_eq!(inner[0].0, "meeting-review.json");
        assert_eq!(inner.len(), 4);
        // The first export then replaces it whole, and is itself intact.
        let archive = entries(&path);
        assert_eq!(archive[0].0, "meeting-review.txt");
        assert_eq!(archive.len(), 4);
        assert_eq!(archive[1].1, std::fs::read(&paths[0]).unwrap());
        assert_eq!(outer.bytes, std::fs::metadata(&path).unwrap().len());
        assert_eq!(directory_names(&path), ["meeting.zip"]);
    }

    #[test]
    fn deleting_the_session_before_publish_fails_closed_and_keeps_the_destination() {
        let (temporary, repository, _) = fixture();
        let path = destination(&temporary);
        std::fs::write(&path, "previous export").unwrap();
        let error = gated(
            &repository,
            &path,
            MeetingAudioExportFormat::Markdown,
            &|| false,
            &|| repository.delete_session("retained").unwrap(),
        )
        .unwrap_err();
        assert_eq!(error, CHANGED);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "previous export");
        assert_eq!(directory_names(&path), ["meeting.zip"]);
    }

    #[test]
    fn a_removed_wav_before_publish_fails_closed() {
        let (temporary, repository, paths) = fixture();
        let path = destination(&temporary);
        let error = gated(
            &repository,
            &path,
            MeetingAudioExportFormat::Markdown,
            &|| false,
            &|| {
                // Revocation happened (as a delete would) and a file is gone.
                crate::meeting_audio::revoke_audio_reads();
                std::fs::remove_file(&paths[2]).unwrap();
            },
        )
        .unwrap_err();
        assert_eq!(error, CHANGED);
        assert!(directory_names(&path).is_empty());
    }

    #[test]
    fn deleting_an_unrelated_session_does_not_block_publication() {
        let (temporary, repository, _) = fixture();
        session(&repository, "other", true);
        clip(&repository, "other", MeetingSpeaker::Me, 0, 0, 500);
        finish(&repository, "other");
        let path = destination(&temporary);
        let summary = gated(
            &repository,
            &path,
            MeetingAudioExportFormat::Markdown,
            &|| false,
            &|| repository.delete_session("other").unwrap(),
        )
        .unwrap();
        assert_eq!(summary.audio_files, 3);
        assert_eq!(entries(&path).len(), 4);
    }

    #[test]
    fn a_delete_cannot_complete_while_the_export_is_publishing() {
        let (temporary, repository, _) = fixture();
        let path = destination(&temporary);
        let calls = Cell::new(0);
        let deleter = RefCell::new(None);
        let (sender, receiver) = std::sync::mpsc::channel();
        let (about_to_lock, lock_requested) = std::sync::mpsc::channel();
        // Calls 1-3 are the segment reads; call 4 runs under the publish lock.
        let busy = || {
            calls.set(calls.get() + 1);
            if calls.get() == 4 {
                let deleting = repository.clone();
                let sender = sender.clone();
                let about_to_lock = about_to_lock.clone();
                *deleter.borrow_mut() = Some(std::thread::spawn(move || {
                    deleting
                        .delete_session_with(
                            "retained",
                            &|| about_to_lock.send(()).unwrap(),
                            &|| {},
                        )
                        .unwrap();
                    sender.send(()).unwrap();
                }));
                // The deleter is provably at its lock request; the publish lock
                // is held, so it can neither revoke nor finish until we return.
                lock_requested.recv().unwrap();
                assert!(receiver
                    .recv_timeout(std::time::Duration::from_millis(200))
                    .is_err());
                assert!(repository.workspace("retained").is_ok());
            }
            false
        };
        let summary = gated(
            &repository,
            &path,
            MeetingAudioExportFormat::Text,
            &busy,
            &|| {},
        )
        .unwrap();
        assert_eq!(summary.audio_files, 3);
        deleter.into_inner().unwrap().join().unwrap();
        receiver.recv().unwrap();
        assert_eq!(entries(&path).len(), 4);
        assert!(repository.workspace("retained").is_err());
    }

    #[test]
    fn an_equal_revision_never_proves_the_exported_audio_still_exists() {
        // A delete bumps the revision before removing anything, so an export can
        // sample that revision, finish reading, and see it unchanged after the
        // delete completes. Revalidation must not rely on the revision.
        let (_temporary, repository, _) = fixture();
        let workspace = repository.workspace("retained").unwrap();
        let revision = current_audio_revision();
        let built = write_audio_export(
            &repository,
            &workspace,
            MeetingAudioExportFormat::Markdown,
            std::io::Cursor::new(Vec::new()),
            revision,
            &|| false,
        )
        .unwrap();
        assert!(still_current(&repository, "retained", &built).is_ok());
        repository
            .delete_session_with("retained", &|| {}, &|| {})
            .unwrap();
        assert_ne!(current_audio_revision(), revision);
        assert_eq!(
            still_current(&repository, "retained", &built).unwrap_err(),
            CHANGED
        );
    }

    #[test]
    fn an_export_starting_during_a_paused_delete_waits_and_never_publishes_deleted_audio() {
        let (temporary, repository, _) = fixture();
        let path = destination(&temporary);
        let (revoked_sender, revoked) = std::sync::mpsc::channel();
        let (resume_sender, resume) = std::sync::mpsc::channel::<()>();
        let resume = std::sync::Mutex::new(resume);
        let (ready_sender, ready) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            let deleter = scope.spawn(|| {
                repository
                    .delete_session_with("retained", &|| {}, &|| {
                        // Revoked and holding the lock, nothing removed yet.
                        revoked_sender.send(()).unwrap();
                        resume.lock().unwrap().recv().unwrap();
                    })
                    .unwrap();
            });
            revoked.recv().unwrap();
            let exporter = scope.spawn(|| {
                super::export_to_path(
                    &repository,
                    "retained",
                    MeetingAudioExportFormat::Markdown,
                    &path,
                    &ExportGates {
                        busy: &|| false,
                        before_revision: &|| ready_sender.send(()).unwrap(),
                        before_publish: &|| {},
                    },
                )
            });
            // The exporter is at its revision sample, which needs the lock the
            // paused delete holds; let the delete finish removing everything.
            ready.recv().unwrap();
            resume_sender.send(()).unwrap();
            deleter.join().unwrap();
            let error = exporter.join().unwrap().unwrap_err();
            assert!(error.contains("unavailable"), "{error}");
        });
        assert!(directory_names(&path).is_empty());
        assert!(repository.workspace("retained").is_err());
    }

    #[test]
    fn capture_starting_during_or_after_the_read_refuses_and_cleans_up() {
        for flip_at in [1, 2, 4] {
            let (temporary, repository, _) = fixture();
            let path = destination(&temporary);
            std::fs::write(&path, "previous export").unwrap();
            let calls = Cell::new(0);
            let busy = || {
                calls.set(calls.get() + 1);
                calls.get() >= flip_at
            };
            let error = gated(
                &repository,
                &path,
                MeetingAudioExportFormat::Json,
                &busy,
                &|| {},
            )
            .unwrap_err();
            assert_eq!(error, BUSY, "flip {flip_at}");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "previous export");
            assert_eq!(directory_names(&path), ["meeting.zip"], "flip {flip_at}");
            assert_eq!(calls.get(), flip_at);
        }
    }
}
