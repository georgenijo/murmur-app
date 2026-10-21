//! Review-and-audio ZIP export for finished meetings that kept their audio.
//!
//! The archive holds the exact `render_export` document and every retained
//! WAV chunk, stored byte for byte. Chunks are read one at a time through the
//! session-scoped playback read path (`audio_reference`, no-follow open, WAV
//! header validation, revalidation), so the export has no path resolution of
//! its own. Missing or invalid chunks are never replaced: they are listed in
//! `MISSING-AUDIO.txt` and counted in the returned summary.

use super::{
    admit_export, publish_gate, read_audio_file, ExportAudioSegment, AUDIO_REVISION,
    MAX_EXPORT_AUDIO_BYTES, MAX_EXPORT_AUDIO_FILES,
};
use crate::meeting_review::{render_export, MeetingReviewExportFormat};
use crate::meeting_store::MeetingRepository;
use serde::Serialize;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::Ordering;
use zip::write::{SimpleFileOptions, ZipWriter};
use zip::CompressionMethod;

const GAP_NOTICE_NAME: &str = "MISSING-AUDIO.txt";
const CHANGED: &str =
    "Meeting audio changed or was deleted during the export. Nothing was saved; try again.";
const EXPORT_FAILED: &str = "The meeting audio export could not be written.";

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AudioBundleSummary {
    pub bytes: u64,
    pub audio_files: u32,
    /// Retained chunks that were absent or failed validation and are listed in
    /// `MISSING-AUDIO.txt` instead of the archive.
    pub missing_audio: u32,
    /// Chunks still awaiting transcription; playback excludes them as well.
    pub pending_audio: u32,
}

fn document_name(format: MeetingReviewExportFormat) -> Result<&'static str, String> {
    match format {
        MeetingReviewExportFormat::Markdown => Ok("review.md"),
        MeetingReviewExportFormat::Text => Ok("review.txt"),
        MeetingReviewExportFormat::Json => Ok("review.json"),
        MeetingReviewExportFormat::Srt | MeetingReviewExportFormat::Vtt => {
            Err("Choose Markdown, plain text, or JSON to export with audio.".into())
        }
    }
}

fn entry_name(segment: &ExportAudioSegment) -> String {
    format!(
        "audio/{}-{:08}.wav",
        segment.channel.as_db(),
        segment.sequence
    )
}

fn clock(ms: u64) -> String {
    let seconds = ms / 1_000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

fn gap_notice(missing: &[&ExportAudioSegment], pending: u32) -> String {
    let mut notice = String::from(
        "Some retained meeting audio is not in this export. Murmur never substitutes or recreates audio.\n",
    );
    if !missing.is_empty() {
        notice.push_str(&format!(
            "\nMissing, unreadable, or invalid audio files: {}\n",
            missing.len()
        ));
        for segment in missing {
            notice.push_str(&format!(
                "- {} (segment {}, {}-{})\n",
                entry_name(segment).trim_start_matches("audio/"),
                segment.segment_id,
                clock(segment.start_ms),
                clock(segment.end_ms)
            ));
        }
    }
    if pending > 0 {
        notice.push_str(&format!(
            "\nChunks still waiting for transcription and not exported: {pending}\n"
        ));
    }
    notice
}

fn zip_error<T>(result: Result<T, impl Sized>) -> Result<T, String> {
    result.map_err(|_| EXPORT_FAILED.to_string())
}

/// Saves `path` as a stored (uncompressed) ZIP of the review document and the
/// retained audio of `session_id`. Refuses unretained or active sessions,
/// unsupported formats, archives over the audio bounds, and exports with no
/// readable audio. Failures leave nothing at `path`.
pub(crate) fn export_review_with_audio(
    repository: &MeetingRepository,
    session_id: &str,
    format: MeetingReviewExportFormat,
    path: &Path,
) -> Result<AudioBundleSummary, String> {
    export_with_limits(
        repository,
        session_id,
        format,
        path,
        MAX_EXPORT_AUDIO_FILES as usize,
        MAX_EXPORT_AUDIO_BYTES,
    )
}

pub(super) fn export_with_limits(
    repository: &MeetingRepository,
    session_id: &str,
    format: MeetingReviewExportFormat,
    path: &Path,
    max_files: usize,
    max_bytes: u64,
) -> Result<AudioBundleSummary, String> {
    let document_name = document_name(format)?;
    // Before any session state is read, so a removal still in progress is
    // waited out instead of being adopted as this export's baseline.
    let revision = admit_export();
    let plan = repository.export_audio_plan(session_id)?;
    if plan.segments.is_empty() {
        return Err("This meeting has no retained audio files to export.".into());
    }
    if plan.segments.len() > max_files {
        return Err(format!(
            "This meeting has more than {max_files} audio files, which is more than one export can hold. Nothing was saved."
        ));
    }
    let workspace = repository.workspace(session_id)?;
    let document = render_export(&workspace, format)?;
    let mut summary = AudioBundleSummary {
        bytes: 0,
        audio_files: 0,
        missing_audio: 0,
        pending_audio: plan.pending,
    };
    crate::commands::export::write_file_export(path, &["zip"], |file| {
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Stored)
            .unix_permissions(0o644);
        let mut archive = ZipWriter::new(file);
        zip_error(archive.start_file(document_name, options))?;
        zip_error(archive.write_all(document.as_bytes()))?;
        let mut audio_bytes = 0_u64;
        let mut missing = Vec::new();
        for segment in &plan.segments {
            let Ok(bytes) = read_audio_file(repository, session_id, segment.segment_id, revision)
            else {
                missing.push(segment);
                continue;
            };
            audio_bytes += bytes.len() as u64;
            if audio_bytes > max_bytes {
                return Err(format!(
                    "This meeting's audio is larger than the {} MiB one export can hold. Nothing was saved.",
                    max_bytes / (1024 * 1024)
                ));
            }
            zip_error(archive.start_file(entry_name(segment), options))?;
            zip_error(archive.write_all(&bytes))?;
            summary.audio_files += 1;
        }
        if summary.audio_files == 0 {
            return Err(
                "None of this meeting's retained audio could be read, so nothing was saved.".into(),
            );
        }
        summary.missing_audio = u32::try_from(missing.len()).unwrap_or(u32::MAX);
        if !missing.is_empty() || plan.pending > 0 {
            zip_error(archive.start_file(GAP_NOTICE_NAME, options))?;
            zip_error(archive.write_all(gap_notice(&missing, plan.pending).as_bytes()))?;
        }
        zip_error(archive.finish())?;
        Ok(())
    }, || {
        #[cfg(test)]
        super::tests::before_publication();
        // Held across the rename: a deletion must either revoke first (and
        // this refuses) or wait until the archive is already published.
        let gate = publish_gate();
        if AUDIO_REVISION.load(Ordering::Acquire) != revision {
            return Err(CHANGED.into());
        }
        #[cfg(test)]
        super::tests::while_publishing();
        Ok(gate)
    })
    .map(|bytes| AudioBundleSummary { bytes, ..summary })
}
