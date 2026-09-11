use crate::meeting_store::MeetingRepository;
use crate::{MutexExt, State};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use tauri::{Emitter, Manager};

pub(crate) const MAX_MANIFEST_CHUNKS: u32 = 256;
pub(crate) const MAX_AUDIO_FILE_BYTES: u64 = 512 * 1024;
const MAX_RANGE_BYTES: u32 = 64 * 1024;
pub(crate) const MAX_CHUNK_DURATION_MS: u64 = 15_500;
pub(crate) const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
pub(crate) const AUDIO_UNAVAILABLE: &str = "This retained meeting audio is unavailable.";
const INVALID_REQUEST: &str = "The meeting audio request is invalid.";
const CAPTURE_BUSY: &str =
    "Pause recording or finish the active voice task before playing meeting audio.";
const INVALIDATED: &str =
    "Meeting audio changed or was deleted. Open the meeting again before playing.";

#[derive(Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PlaybackChannel {
    #[default]
    All,
    Me,
    Them,
}

impl PlaybackChannel {
    pub(crate) fn database_filter(self) -> Option<&'static str> {
        match self {
            Self::All => None,
            Self::Me => Some("me"),
            Self::Them => Some("them"),
        }
    }
}

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AudioCursor {
    pub start_ms: u64,
    pub segment_id: i64,
}

#[derive(Clone, Serialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AudioChunk {
    pub segment_id: i64,
    pub channel: crate::meeting_store::MeetingSpeaker,
    pub start_ms: u64,
    pub end_ms: u64,
}

#[derive(Clone, Copy, Serialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum AudioUnavailableReason {
    NotRetained,
    NotFinished,
    NoAudio,
}

#[derive(Serialize, PartialEq, Eq, Debug)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AudioManifest {
    Unavailable {
        reason: AudioUnavailableReason,
    },
    Available {
        session_id: String,
        duration_ms: u64,
        chunks: Vec<AudioChunk>,
        next_cursor: Option<AudioCursor>,
    },
}

pub(crate) struct OwnedAudioReference {
    pub path: PathBuf,
    pub duration_ms: u64,
}

pub(crate) fn validate_session_id(value: &str) -> Result<(), String> {
    if !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        Ok(())
    } else {
        Err(INVALID_REQUEST.into())
    }
}

pub(crate) fn validate_manifest_request(
    session_id: &str,
    from_ms: u64,
    cursor: Option<&AudioCursor>,
    limit: u32,
) -> Result<(), String> {
    validate_session_id(session_id)?;
    if from_ms > MAX_SAFE_INTEGER
        || limit == 0
        || limit > MAX_MANIFEST_CHUNKS
        || cursor.is_some_and(|cursor| {
            cursor.start_ms > MAX_SAFE_INTEGER
                || cursor.segment_id <= 0
                || cursor.segment_id as u64 > MAX_SAFE_INTEGER
        })
    {
        Err(INVALID_REQUEST.into())
    } else {
        Ok(())
    }
}

fn require_main_window(label: &str) -> Result<(), String> {
    if label == "main" {
        Ok(())
    } else {
        Err("Meeting audio is only available from the main window.".into())
    }
}

fn capture_busy(state: &State) -> bool {
    state.app_state.dictation.lock_or_recover().status != crate::state::DictationStatus::Idle
        || state.app_state.meeting_blocks_asr()
        || state.app_state.transform_status().blocks_recording()
        || state.transform_runtime.is_transform_busy()
        || state.query.status().blocks_pipeline()
        || state.app_state.microphone_preview.is_active()
        || state.microphone_startup_benchmark.is_active()
        || crate::audio_lifecycle::is_audio_active()
}

#[tauri::command]
pub fn get_meeting_audio_capture_busy(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, State>,
) -> Result<bool, String> {
    require_main_window(window.label())?;
    Ok(capture_busy(&state))
}

#[tauri::command]
pub async fn get_meeting_audio_manifest(
    window: tauri::WebviewWindow,
    session_id: String,
    from_ms: Option<u64>,
    channel: Option<PlaybackChannel>,
    cursor: Option<AudioCursor>,
    limit: Option<u32>,
    state: tauri::State<'_, State>,
) -> Result<AudioManifest, String> {
    require_main_window(window.label())?;
    let from_ms = from_ms.unwrap_or(0);
    let limit = limit.unwrap_or(128);
    validate_manifest_request(&session_id, from_ms, cursor.as_ref(), limit)?;
    let repository = state.meeting_store.repository()?;
    let permit = AudioReadPermit::acquire()?;
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = permit;
        repository.audio_manifest(
            &session_id,
            from_ms,
            channel.unwrap_or_default(),
            cursor.as_ref(),
            limit,
        )
    })
    .await
    .map_err(|_| AUDIO_UNAVAILABLE.to_string())?
}

static AUDIO_READERS: AtomicUsize = AtomicUsize::new(0);
static AUDIO_REVISION: AtomicU64 = AtomicU64::new(0);

struct AudioReadPermit;
impl AudioReadPermit {
    fn acquire() -> Result<Self, String> {
        AUDIO_READERS
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < 2).then_some(count + 1)
            })
            .map(|_| Self)
            .map_err(|_| "Meeting audio is already loading. Try again shortly.".into())
    }
}
impl Drop for AudioReadPermit {
    fn drop(&mut self) {
        AUDIO_READERS.fetch_sub(1, Ordering::Release);
    }
}

pub(crate) fn revoke_audio_reads() {
    AUDIO_REVISION.fetch_add(1, Ordering::AcqRel);
}

pub(crate) fn invalidate_playback(app: &tauri::AppHandle, session_id: Option<&str>) {
    #[derive(Clone, Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Invalidated<'a> {
        session_id: Option<&'a str>,
    }
    revoke_audio_reads();
    let _ = app.emit_to(
        "main",
        "meeting-audio-invalidated",
        Invalidated { session_id },
    );
}

#[tauri::command]
pub async fn read_meeting_audio_range(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    session_id: String,
    segment_id: i64,
    offset_bytes: u64,
    length_bytes: u32,
    state: tauri::State<'_, State>,
) -> Result<tauri::ipc::Response, String> {
    require_main_window(window.label())?;
    validate_range_request(&session_id, segment_id, offset_bytes, length_bytes)?;
    if capture_busy(&state) {
        return Err(CAPTURE_BUSY.into());
    }
    let repository = state.meeting_store.repository()?;
    let permit = AudioReadPermit::acquire()?;
    let revision = AUDIO_REVISION.load(Ordering::Acquire);
    let bytes = tauri::async_runtime::spawn_blocking(move || -> Result<Vec<u8>, String> {
        let _permit = permit;
        if capture_busy(&app.state::<State>()) {
            return Err(CAPTURE_BUSY.into());
        }
        let bytes = read_audio_range(
            &repository,
            &session_id,
            segment_id,
            offset_bytes,
            length_bytes,
            revision,
        )?;
        if capture_busy(&app.state::<State>()) {
            return Err(CAPTURE_BUSY.into());
        }
        Ok(bytes)
    })
    .await
    .map_err(|_| AUDIO_UNAVAILABLE.to_string())??;
    Ok(tauri::ipc::Response::new(bytes))
}

fn validate_range_request(
    session_id: &str,
    segment_id: i64,
    offset: u64,
    length: u32,
) -> Result<(), String> {
    validate_session_id(session_id)?;
    if segment_id <= 0
        || segment_id as u64 > MAX_SAFE_INTEGER
        || offset > MAX_AUDIO_FILE_BYTES
        || length == 0
        || length > MAX_RANGE_BYTES
    {
        Err(INVALID_REQUEST.into())
    } else {
        Ok(())
    }
}

fn read_audio_range(
    repository: &MeetingRepository,
    session_id: &str,
    segment_id: i64,
    offset: u64,
    length: u32,
    revision: u64,
) -> Result<Vec<u8>, String> {
    validate_range_request(session_id, segment_id, offset, length)?;
    let reference = repository.audio_reference(session_id, segment_id)?;
    let mut file = open_audio_file(&reference.path)?;
    let metadata = file.metadata().map_err(|_| AUDIO_UNAVAILABLE)?;
    validate_wav(&mut file, metadata.len(), reference.duration_ms)?;
    if offset > metadata.len() {
        return Err(INVALID_REQUEST.into());
    }
    let amount = u64::from(length).min(metadata.len() - offset) as usize;
    file.seek(SeekFrom::Start(offset))
        .map_err(|_| AUDIO_UNAVAILABLE)?;
    let mut bytes = vec![0; amount];
    file.read_exact(&mut bytes).map_err(|_| AUDIO_UNAVAILABLE)?;
    revalidate_audio_read(
        repository, session_id, segment_id, &reference, &file, &metadata, revision,
    )?;
    Ok(bytes)
}

fn revalidate_audio_read(
    repository: &MeetingRepository,
    session_id: &str,
    segment_id: i64,
    reference: &OwnedAudioReference,
    file: &File,
    before: &std::fs::Metadata,
    revision: u64,
) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    let same_file = |after: &std::fs::Metadata| {
        after.is_file()
            && after.nlink() == 1
            && after.dev() == before.dev()
            && after.ino() == before.ino()
            && after.len() == before.len()
            && after.mtime() == before.mtime()
            && after.mtime_nsec() == before.mtime_nsec()
    };
    let after = file.metadata().map_err(|_| INVALIDATED)?;
    if !same_file(&after) {
        return Err(INVALIDATED.into());
    }
    if AUDIO_REVISION.load(Ordering::Acquire) != revision {
        let current = repository
            .audio_reference(session_id, segment_id)
            .map_err(|_| INVALIDATED)?;
        if current.path != reference.path || current.duration_ms != reference.duration_ms {
            return Err(INVALIDATED.into());
        }
    }
    // A renamed file can keep nlink=1 while an old descriptor survives. Verify
    // the current no-follow path still names that same inode before returning.
    let current = open_audio_file(&reference.path).map_err(|_| INVALIDATED)?;
    if !same_file(&current.metadata().map_err(|_| INVALIDATED)?) {
        return Err(INVALIDATED.into());
    }
    Ok(())
}

fn open_audio_file(path: &Path) -> Result<File, String> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW_ANY | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| AUDIO_UNAVAILABLE)?;
    let metadata = file.metadata().map_err(|_| AUDIO_UNAVAILABLE)?;
    if !metadata.is_file()
        || metadata.nlink() != 1
        || !(44..=MAX_AUDIO_FILE_BYTES).contains(&metadata.len())
    {
        return Err(AUDIO_UNAVAILABLE.into());
    }
    Ok(file)
}

fn validate_wav(file: &mut File, length: u64, expected_duration_ms: u64) -> Result<(), String> {
    let mut header = [0; 12];
    file.read_exact(&mut header)
        .map_err(|_| AUDIO_UNAVAILABLE)?;
    let riff_bytes =
        u32::from_le_bytes(header[4..8].try_into().map_err(|_| AUDIO_UNAVAILABLE)?) as u64;
    if &header[..4] != b"RIFF"
        || &header[8..] != b"WAVE"
        || riff_bytes.checked_add(8) != Some(length)
    {
        return Err(AUDIO_UNAVAILABLE.into());
    }
    let mut format_seen = false;
    let mut position = 12_u64;
    while position.checked_add(8).is_some_and(|end| end <= length) {
        let mut chunk = [0; 8];
        file.read_exact(&mut chunk).map_err(|_| AUDIO_UNAVAILABLE)?;
        let size =
            u32::from_le_bytes(chunk[4..8].try_into().map_err(|_| AUDIO_UNAVAILABLE)?) as u64;
        position += 8;
        let end = position
            .checked_add(size)
            .filter(|end| *end <= length)
            .ok_or(AUDIO_UNAVAILABLE)?;
        match &chunk[..4] {
            b"fmt " if !format_seen && matches!(size, 16 | 18) => {
                let mut format = [0; 18];
                file.read_exact(&mut format[..size as usize])
                    .map_err(|_| AUDIO_UNAVAILABLE)?;
                let expected = [1, 0, 1, 0, 128, 62, 0, 0, 0, 125, 0, 0, 2, 0, 16, 0, 0, 0];
                if format != expected {
                    return Err(AUDIO_UNAVAILABLE.into());
                }
                format_seen = true;
            }
            b"data"
                if format_seen
                    && size > 0
                    && size <= MAX_CHUNK_DURATION_MS * 32
                    && size.is_multiple_of(2)
                    && end == length =>
            {
                let duration_ms = size * 1_000 / (16_000 * 2);
                if duration_ms > MAX_CHUNK_DURATION_MS
                    || duration_ms.abs_diff(expected_duration_ms) > 2
                {
                    return Err(AUDIO_UNAVAILABLE.into());
                }
                return Ok(());
            }
            _ => return Err(AUDIO_UNAVAILABLE.into()),
        }
        position = end;
    }
    Err(AUDIO_UNAVAILABLE.into())
}

pub(crate) fn remove_owned_audio(
    root: &Path,
    session_id: &str,
    relative: &str,
) -> Result<(), String> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;
    validate_session_id(session_id)?;
    let expected_parent = Path::new("audio").join(session_id);
    let relative = Path::new(relative);
    if relative.parent() != Some(expected_parent.as_path()) {
        return Err(AUDIO_UNAVAILABLE.into());
    }
    let filename = relative
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| name.ends_with(".wav"))
        .ok_or(AUDIO_UNAVAILABLE)?;
    let name = std::ffi::CString::new(filename).map_err(|_| AUDIO_UNAVAILABLE)?;
    let directory = match OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW_ANY | libc::O_CLOEXEC).open(root.join(expected_parent)) {
        Ok(directory) => directory,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err("Some retained audio could not be removed. The meeting remains available so you can retry deletion.".into()),
    };
    if unsafe { libc::unlinkat(directory.as_raw_fd(), name.as_ptr(), 0) } == 0 {
        return Ok(());
    }
    if std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound {
        Ok(())
    } else {
        Err("Some retained audio could not be removed. The meeting remains available so you can retry deletion.".into())
    }
}

pub(crate) fn remove_empty_session_audio(root: &Path, session_id: &str) -> Result<(), String> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;
    validate_session_id(session_id)?;
    let parent = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW_ANY | libc::O_CLOEXEC)
        .open(root.join("audio"))
    {
        Ok(parent) => parent,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err("The meeting audio folder could not be removed safely.".into()),
    };
    let name = std::ffi::CString::new(session_id).map_err(|_| AUDIO_UNAVAILABLE)?;
    if unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), libc::AT_REMOVEDIR) } == 0 {
        return Ok(());
    }
    match std::io::Error::last_os_error().raw_os_error() {
        Some(libc::ENOENT | libc::ENOTEMPTY) => Ok(()),
        _ => Err("The meeting audio folder could not be removed safely.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meeting_store::{MeetingSessionStatus, MeetingSpeaker};

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
        duration_ms: u64,
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
        for sample in 0..duration_ms * 16 {
            writer.write_sample((sample % 1_000) as i16).unwrap();
        }
        writer.finalize().unwrap();
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
        (id, path)
    }

    fn finish(repository: &MeetingRepository, session_id: &str) {
        repository
            .finish_session(session_id, MeetingSessionStatus::Complete, None)
            .unwrap();
    }

    fn read(
        repository: &MeetingRepository,
        session_id: &str,
        id: i64,
        offset: u64,
        length: u32,
    ) -> Result<Vec<u8>, String> {
        read_audio_range(
            repository,
            session_id,
            id,
            offset,
            length,
            AUDIO_REVISION.load(Ordering::Acquire),
        )
    }

    #[test]
    fn meeting_audio_manifest_preserves_overlaps_gaps_global_duration_and_keyset_pages() {
        let (_temporary, repository) = repository();
        session(&repository, "timeline", true);
        let (me, _) = clip(&repository, "timeline", MeetingSpeaker::Me, 0, 0, 1_000);
        let (them, _) = clip(&repository, "timeline", MeetingSpeaker::Them, 0, 500, 1_000);
        let (later, _) = clip(
            &repository,
            "timeline",
            MeetingSpeaker::Me,
            1,
            3_600_000,
            1_000,
        );
        finish(&repository, "timeline");
        let AudioManifest::Available {
            duration_ms,
            chunks,
            next_cursor,
            ..
        } = repository
            .audio_manifest("timeline", 750, PlaybackChannel::All, None, 1)
            .unwrap()
        else {
            panic!("expected retained audio");
        };
        assert_eq!(duration_ms, 3_601_000);
        assert_eq!(chunks[0].segment_id, me);
        assert_eq!(chunks[0].start_ms, 0);
        let AudioManifest::Available {
            chunks,
            next_cursor,
            ..
        } = repository
            .audio_manifest(
                "timeline",
                750,
                PlaybackChannel::All,
                next_cursor.as_ref(),
                1,
            )
            .unwrap()
        else {
            panic!("expected next page");
        };
        assert_eq!(chunks[0].segment_id, them);
        let AudioManifest::Available {
            chunks,
            next_cursor,
            ..
        } = repository
            .audio_manifest(
                "timeline",
                750,
                PlaybackChannel::All,
                next_cursor.as_ref(),
                1,
            )
            .unwrap()
        else {
            panic!("expected last page");
        };
        assert_eq!(chunks[0].segment_id, later);
        assert!(next_cursor.is_none());
        let AudioManifest::Available {
            duration_ms,
            chunks,
            ..
        } = repository
            .audio_manifest("timeline", 2_000, PlaybackChannel::Them, None, 128)
            .unwrap()
        else {
            panic!("empty channel is still available");
        };
        assert_eq!(duration_ms, 3_601_000);
        assert!(chunks.is_empty());
    }

    #[test]
    fn meeting_audio_consent_and_finished_session_gate_every_read() {
        let (_temporary, repository) = repository();
        session(&repository, "unretained", false);
        let (id, _) = clip(&repository, "unretained", MeetingSpeaker::Me, 0, 0, 1_000);
        finish(&repository, "unretained");
        assert_eq!(
            repository
                .audio_manifest("unretained", 0, PlaybackChannel::All, None, 128)
                .unwrap(),
            AudioManifest::Unavailable {
                reason: AudioUnavailableReason::NotRetained
            }
        );
        assert!(read(&repository, "unretained", id, 0, 64).is_err());
        session(&repository, "active", true);
        let (active, _) = clip(&repository, "active", MeetingSpeaker::Me, 0, 0, 1_000);
        assert_eq!(
            repository
                .audio_manifest("active", 0, PlaybackChannel::All, None, 128)
                .unwrap(),
            AudioManifest::Unavailable {
                reason: AudioUnavailableReason::NotFinished
            }
        );
        assert!(read(&repository, "active", active, 0, 64).is_err());
        session(&repository, "empty", true);
        finish(&repository, "empty");
        assert_eq!(
            repository
                .audio_manifest("empty", 0, PlaybackChannel::All, None, 128)
                .unwrap(),
            AudioManifest::Unavailable {
                reason: AudioUnavailableReason::NoAudio
            }
        );
        repository
            .finish_session(
                "active",
                MeetingSessionStatus::Failed,
                Some("supervisor_unavailable"),
            )
            .unwrap();
        assert!(read(&repository, "active", active, 0, 64).is_ok());
    }

    #[test]
    fn meeting_audio_binary_ranges_round_trip_maximum_chunk_and_eof() {
        let (_temporary, repository) = repository();
        session(&repository, "ranges", true);
        let (id, path) = clip(
            &repository,
            "ranges",
            MeetingSpeaker::Me,
            0,
            0,
            MAX_CHUNK_DURATION_MS,
        );
        finish(&repository, "ranges");
        let expected = std::fs::read(&path).unwrap();
        let reference = repository
            .audio_reference("ranges", id)
            .expect("owned reference");
        let mut checked = open_audio_file(&reference.path).expect("safe file open");
        validate_wav(&mut checked, expected.len() as u64, reference.duration_ms)
            .expect("valid generated WAV");
        let mut assembled = Vec::new();
        for _ in 0..9 {
            let chunk = read(
                &repository,
                "ranges",
                id,
                assembled.len() as u64,
                MAX_RANGE_BYTES,
            )
            .unwrap();
            assert!(chunk.len() <= MAX_RANGE_BYTES as usize);
            let finished = chunk.len() < MAX_RANGE_BYTES as usize;
            assembled.extend(chunk);
            if finished {
                break;
            }
        }
        assert_eq!(assembled, expected);
        assert!(read(&repository, "ranges", id, expected.len() as u64, 64)
            .unwrap()
            .is_empty());
        assert!(read(&repository, "ranges", id, expected.len() as u64 + 1, 64).is_err());
        assert_eq!(
            read(&repository, "ranges", id, 123, 67).unwrap(),
            expected[123..190]
        );
    }

    #[test]
    fn meeting_audio_refuses_other_sessions_and_forged_database_paths() {
        let (_temporary, repository) = repository();
        session(&repository, "owner", true);
        session(&repository, "other", true);
        let (id, _) = clip(&repository, "owner", MeetingSpeaker::Me, 0, 0, 1_000);
        let (_other, other_path) = clip(&repository, "other", MeetingSpeaker::Me, 0, 0, 1_000);
        finish(&repository, "owner");
        finish(&repository, "other");
        assert!(read(&repository, "other", id, 0, 64).is_err());
        let connection =
            rusqlite::Connection::open(repository.root().join("meetings.sqlite3")).unwrap();
        connection.execute("UPDATE meeting_segments SET audio_relative_path='audio/other/me-00000000.wav' WHERE id=?", [id]).unwrap();
        assert!(read(&repository, "owner", id, 0, 64).is_err());
        assert!(repository.delete_session("owner").is_err());
        assert!(other_path.exists());
        assert!(repository.get_session("owner").is_ok());
    }

    #[test]
    fn meeting_audio_refuses_file_parent_symlinks_and_hardlinks() {
        use std::os::unix::fs::symlink;
        let (temporary, repository) = repository();
        session(&repository, "links", true);
        let (id, path) = clip(&repository, "links", MeetingSpeaker::Me, 0, 0, 1_000);
        finish(&repository, "links");
        let outside = temporary.path().canonicalize().unwrap().join("outside.wav");
        std::fs::rename(&path, &outside).unwrap();
        symlink(&outside, &path).unwrap();
        assert!(read(&repository, "links", id, 0, 64).is_err());
        std::fs::remove_file(&path).unwrap();
        std::fs::hard_link(&outside, &path).unwrap();
        assert!(read(&repository, "links", id, 0, 64).is_err());
        std::fs::remove_file(&path).unwrap();
        std::fs::rename(&outside, &path).unwrap();
        let directory = path.parent().unwrap();
        let outside_directory = temporary.path().canonicalize().unwrap().join("elsewhere");
        std::fs::rename(directory, &outside_directory).unwrap();
        symlink(&outside_directory, directory).unwrap();
        assert!(read(&repository, "links", id, 0, 64).is_err());
        assert!(repository.delete_session("links").is_err());
        assert!(outside_directory.join("me-00000000.wav").exists());
    }

    #[test]
    fn meeting_audio_refuses_wrong_format_sizes_and_truncated_wavs_without_private_errors() {
        let (_temporary, repository) = repository();
        session(&repository, "malformed", true);
        let (id, path) = clip(&repository, "malformed", MeetingSpeaker::Me, 0, 0, 1_000);
        finish(&repository, "malformed");
        let original = std::fs::read(&path).unwrap();
        for (offset, replacement) in [
            (0, b'S'),
            (8, b'S'),
            (20, 3),
            (22, 2),
            (24, 0),
            (34, 32),
            (40, 255),
        ] {
            let mut bytes = original.clone();
            bytes[offset] = replacement;
            std::fs::write(&path, bytes).unwrap();
            let error = read(&repository, "malformed", id, 0, 64).unwrap_err();
            assert!(!error.contains(path.to_str().unwrap()));
        }
        std::fs::write(&path, &original[..43]).unwrap();
        assert!(read(&repository, "malformed", id, 0, 64).is_err());
        File::create(&path)
            .unwrap()
            .set_len(MAX_AUDIO_FILE_BYTES + 1)
            .unwrap();
        assert!(read(&repository, "malformed", id, 0, 64).is_err());
    }

    #[test]
    fn meeting_audio_delete_removes_bytes_revokes_reads_and_can_retry_file_failures() {
        let (_temporary, repository) = repository();
        session(&repository, "delete", true);
        let (id, path) = clip(&repository, "delete", MeetingSpeaker::Me, 0, 0, 1_000);
        finish(&repository, "delete");
        let original = std::fs::read(&path).unwrap();
        let revision = AUDIO_REVISION.load(Ordering::Acquire);
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(repository.delete_session("delete").is_err());
        assert!(repository.get_session("delete").is_ok());
        assert!(read_audio_range(&repository, "delete", id, 0, 64, revision).is_err());
        std::fs::remove_dir(&path).unwrap();
        std::fs::write(&path, original).unwrap();
        repository.delete_session("delete").unwrap();
        assert!(!path.exists());
        assert!(read(&repository, "delete", id, 0, 64).is_err());
    }

    #[test]
    fn meeting_audio_unrelated_deletion_preserves_exact_owned_reads() {
        let (_temporary, repository) = repository();
        session(&repository, "playing", true);
        session(&repository, "other-delete", true);
        let (id, path) = clip(&repository, "playing", MeetingSpeaker::Me, 0, 0, 1_000);
        clip(
            &repository,
            "other-delete",
            MeetingSpeaker::Them,
            0,
            0,
            1_000,
        );
        finish(&repository, "playing");
        finish(&repository, "other-delete");
        let revision = AUDIO_REVISION.load(Ordering::Acquire);
        repository.delete_session("other-delete").unwrap();
        assert_ne!(AUDIO_REVISION.load(Ordering::Acquire), revision);
        let bytes = read_audio_range(&repository, "playing", id, 0, 64, revision).unwrap();
        assert_eq!(bytes, std::fs::read(&path).unwrap()[..64]);
        repository.delete_session("playing").unwrap();
        assert!(read_audio_range(&repository, "playing", id, 0, 64, revision).is_err());
    }

    #[test]
    fn meeting_audio_old_descriptors_cannot_survive_owned_file_replacement_or_deletion() {
        let (_temporary, repository) = repository();
        session(&repository, "identity", true);
        let (id, path) = clip(&repository, "identity", MeetingSpeaker::Me, 0, 0, 1_000);
        finish(&repository, "identity");
        let reference = repository.audio_reference("identity", id).unwrap();
        let old_file = open_audio_file(&path).unwrap();
        let before = old_file.metadata().unwrap();
        let revision = AUDIO_REVISION.load(Ordering::Acquire);
        let moved = path.with_extension("old.wav");
        std::fs::rename(&path, &moved).unwrap();
        std::fs::copy(&moved, &path).unwrap();
        assert!(revalidate_audio_read(
            &repository,
            "identity",
            id,
            &reference,
            &old_file,
            &before,
            revision
        )
        .is_err());
        std::fs::remove_file(&path).unwrap();
        std::fs::rename(&moved, &path).unwrap();
        assert!(revalidate_audio_read(
            &repository,
            "identity",
            id,
            &reference,
            &old_file,
            &before,
            revision
        )
        .is_ok());
        repository.delete_session("identity").unwrap();
        assert!(revalidate_audio_read(
            &repository,
            "identity",
            id,
            &reference,
            &old_file,
            &before,
            revision
        )
        .is_err());
    }

    #[test]
    fn meeting_audio_clear_removes_all_owned_files_and_refuses_symlinked_audio_root() {
        use std::os::unix::fs::symlink;
        let (temporary, repository) = repository();
        session(&repository, "clear", true);
        let (_, path) = clip(&repository, "clear", MeetingSpeaker::Me, 0, 0, 1_000);
        finish(&repository, "clear");
        let audio_root = repository.root().join("audio");
        let moved = temporary.path().canonicalize().unwrap().join("moved-audio");
        std::fs::rename(&audio_root, &moved).unwrap();
        symlink(&moved, &audio_root).unwrap();
        assert!(repository.delete_all().is_err());
        assert!(repository.get_session("clear").is_ok());
        assert!(moved.join("clear/me-00000000.wav").exists());
        std::fs::remove_file(&audio_root).unwrap();
        std::fs::rename(&moved, &audio_root).unwrap();
        repository.delete_all().unwrap();
        assert!(!path.exists());
        assert!(audio_root.is_dir());
    }

    #[test]
    fn meeting_audio_pruning_invalidates_only_sessions_selected_for_removal() {
        let (_temporary, repository) = repository();
        for id in ["older", "newer"] {
            session(&repository, id, true);
            clip(&repository, id, MeetingSpeaker::Me, 0, 0, 1_000);
            finish(&repository, id);
        }
        let mut invalidated = Vec::new();
        let revision = AUDIO_REVISION.load(Ordering::Acquire);
        assert_eq!(
            repository
                .prune(None, 10, |id| invalidated.push(id.to_string()))
                .unwrap(),
            0
        );
        assert!(invalidated.is_empty());
        assert_eq!(AUDIO_REVISION.load(Ordering::Acquire), revision);
        assert_eq!(
            repository
                .prune(None, 1, |id| invalidated.push(id.to_string()))
                .unwrap(),
            1
        );
        assert_eq!(invalidated.len(), 1);
        assert!(repository.get_session(&invalidated[0]).is_err());
        assert!(!repository
            .root()
            .join("audio")
            .join(&invalidated[0])
            .exists());
    }

    #[test]
    fn meeting_audio_ipc_and_admission_are_bounded() {
        assert!(require_main_window("main").is_ok());
        for window in ["overlay", "log-viewer", "query-popover", ""] {
            assert!(require_main_window(window).is_err());
        }
        for (offset, length) in [(0, 0), (0, MAX_RANGE_BYTES + 1), (u64::MAX, 64)] {
            assert!(validate_range_request("session", 1, offset, length).is_err());
        }
        assert!(validate_range_request("../outside", 1, 0, 64).is_err());
        assert!(validate_range_request("session", -1, 0, 64).is_err());
        assert!(validate_manifest_request("session", 0, None, 257).is_err());
        assert!(validate_manifest_request("session", u64::MAX, None, 128).is_err());
        let first = AudioReadPermit::acquire().unwrap();
        let second = AudioReadPermit::acquire().unwrap();
        assert!(AudioReadPermit::acquire().is_err());
        drop(first);
        assert!(AudioReadPermit::acquire().is_ok());
        drop(second);
    }
}
