use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior, MAIN_DB};

use super::migrations;
use super::types::*;
use crate::meeting_review::{
    self, ActiveReviewOrigin, GeneratedMeetingReview, MeetingSpeakerLabels, MeetingWorkspace,
    ReviewEditBase, SaveMeetingReviewRequest, SavedMeetingReview,
};

const DATABASE_NAME: &str = "meetings.sqlite3";
const MAX_BACKUPS: usize = 3;

fn storage_error() -> String {
    "The local meeting transcript store is unavailable.".to_string()
}

fn db_error(_: rusqlite::Error) -> String {
    storage_error()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

fn to_i64(value: u64) -> Result<i64, String> {
    i64::try_from(value).map_err(|_| "The meeting timestamp is out of range.".to_string())
}

fn to_u64(value: i64) -> rusqlite::Result<u64> {
    u64::try_from(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })
}

fn to_u32(value: i64) -> rusqlite::Result<u32> {
    u32::try_from(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })
}

fn valid_session_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn owned_audio_path(root: &Path, relative: &str) -> Option<PathBuf> {
    if relative.is_empty() || relative.len() > 512 {
        return None;
    }
    let path = Path::new(relative);
    let mut components = path.components();
    if !matches!(components.next(), Some(Component::Normal(part)) if part == "audio")
        || components.clone().count() < 2
        || components.any(|component| !matches!(component, Component::Normal(_)))
    {
        return None;
    }
    Some(root.join(path))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitializationOutcome {
    Opened,
    Recovered,
    Reinitialized,
}

#[derive(Clone, Debug)]
pub struct MeetingRepository {
    root: PathBuf,
    db_path: PathBuf,
}

struct InitializationLease {
    root: PathBuf,
    _file: fs::File,
}

impl InitializationLease {
    fn acquire(root: &Path) -> Result<Self, String> {
        let mut options = fs::OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let file = options
            .open(root.join(".meeting-initialize.lock"))
            .map_err(|_| storage_error())?;
        file.try_lock().map_err(|error| match error {
            fs::TryLockError::WouldBlock => {
                "Another Murmur instance is initializing meeting data. Try again shortly."
                    .to_string()
            }
            fs::TryLockError::Error(_) => storage_error(),
        })?;
        Ok(Self {
            root: root.to_path_buf(),
            _file: file,
        })
    }
}

struct RecoveryCandidate {
    path: PathBuf,
}

impl RecoveryCandidate {
    fn sweep_abandoned(lease: &InitializationLease) -> Result<(), String> {
        let root = &lease.root;
        for entry in fs::read_dir(root).map_err(|_| storage_error())? {
            let entry = entry.map_err(|_| storage_error())?;
            let name = entry.file_name();
            let Some((id, suffix)) = name
                .to_str()
                .and_then(|name| name.strip_prefix(".meeting-recovery-"))
                .and_then(|name| name.split_once(".sqlite3"))
            else {
                continue;
            };
            if uuid::Uuid::parse_str(id).is_ok() && matches!(suffix, "" | "-wal" | "-shm") {
                fs::remove_file(entry.path()).map_err(|_| storage_error())?;
            }
        }
        Ok(())
    }

    fn from_backup(lease: &InitializationLease, backup: &Path) -> Result<Option<Self>, String> {
        let root = &lease.root;
        let source = Connection::open_with_flags(backup, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(db_error)?;
        match migrations::quick_check(&source)
            .and_then(|()| migrations::validate_supported_schema(&source))
        {
            Ok(()) => {}
            Err(error) if error.is_invalid_backup() => return Ok(None),
            Err(error) => return Err(error.message()),
        }
        let path = root.join(format!(
            ".meeting-recovery-{}.sqlite3",
            uuid::Uuid::new_v4()
        ));
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options.open(&path).map_err(|_| storage_error())?;
        let candidate = Self { path };
        source
            .backup(MAIN_DB, &candidate.path, None)
            .map_err(db_error)?;
        let connection = Connection::open(&candidate.path).map_err(db_error)?;
        configure_connection(&connection).map_err(db_error)?;
        match migrations::migrate(&connection) {
            Ok(()) => {}
            Err(error) if error.is_invalid_backup() => return Ok(None),
            Err(error) => return Err(error.message()),
        }
        migrations::quick_check(&connection).map_err(migrations::MeetingDatabaseError::message)?;
        connection
            .pragma_update(None, "journal_mode", "DELETE")
            .map_err(db_error)?;
        drop(connection);
        fs::File::open(&candidate.path)
            .and_then(|file| file.sync_all())
            .map_err(|_| storage_error())?;
        Ok(Some(candidate))
    }

    fn publish(self, path: &Path, _lease: &InitializationLease) -> Result<(), String> {
        fs::rename(&self.path, path).map_err(|_| storage_error())?;
        if let Some(parent) = path.parent() {
            fs::File::open(parent)
                .and_then(|directory| directory.sync_all())
                .map_err(|_| storage_error())?;
        }
        Ok(())
    }
}

impl Drop for RecoveryCandidate {
    fn drop(&mut self) {
        remove_sidecars(&self.path);
        let _ = fs::remove_file(&self.path);
    }
}

impl MeetingRepository {
    pub fn initialize(root: PathBuf) -> Result<(Self, InitializationOutcome), String> {
        fs::create_dir_all(&root).map_err(|_| storage_error())?;
        let lease = InitializationLease::acquire(&root)?;
        fs::create_dir_all(root.join("audio")).map_err(|_| storage_error())?;
        fs::create_dir_all(root.join("backups")).map_err(|_| storage_error())?;
        fs::create_dir_all(root.join("quarantine")).map_err(|_| storage_error())?;
        RecoveryCandidate::sweep_abandoned(&lease)?;
        let repository = Self {
            db_path: root.join(DATABASE_NAME),
            root,
        };
        let mut outcome = InitializationOutcome::Opened;
        if repository.db_path.exists() {
            let connection = repository.open_raw()?;
            let validity = configure_connection(&connection)
                .map_err(migrations::MeetingDatabaseError::from)
                .and_then(|()| migrations::quick_check(&connection));
            drop(connection);
            match validity {
                Ok(()) => {}
                Err(error) if error.is_corrupt() => {
                    outcome = repository.recover_corrupt_database(&lease)?
                }
                Err(error) => return Err(error.message()),
            }
        } else if !backup_files_newest_first(&repository.root.join("backups"))?.is_empty() {
            outcome = repository.recover_corrupt_database(&lease)?;
        }
        let connection = repository.open_raw()?;
        configure_connection(&connection).map_err(db_error)?;
        let old_version = migrations::schema_version(&connection)
            .map_err(migrations::MeetingDatabaseError::message)?;
        if old_version > 0 && old_version < MEETING_STORE_SCHEMA_VERSION {
            repository.create_backup(&connection, old_version)?;
        }
        migrations::migrate(&connection).map_err(migrations::MeetingDatabaseError::message)?;
        migrations::quick_check(&connection).map_err(migrations::MeetingDatabaseError::message)?;
        connection
            .execute(
                "UPDATE meeting_sessions SET status='interrupted', ended_at_ms=COALESCE(ended_at_ms, ?) WHERE status='active'",
                [to_i64(now_ms())?],
            )
            .map_err(db_error)?;
        repository.create_backup(&connection, MEETING_STORE_SCHEMA_VERSION)?;
        drop(connection);
        repository.sweep_orphan_audio()?;
        Ok((repository, outcome))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn audio_path(&self, relative: &str) -> Result<PathBuf, String> {
        owned_audio_path(&self.root, relative)
            .ok_or_else(|| "The meeting audio spool reference is invalid.".to_string())
    }

    pub fn status(&self) -> Result<MeetingStoreStatus, String> {
        let connection = self.open_checked()?;
        let session_count = connection
            .query_row("SELECT COUNT(*) FROM meeting_sessions", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(db_error)?;
        let pending = connection
            .query_row(
                "SELECT COUNT(*) FROM meeting_segments WHERE status='pending'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(db_error)?;
        Ok(MeetingStoreStatus {
            availability: MeetingStoreAvailability::Available,
            schema_version: MEETING_STORE_SCHEMA_VERSION,
            session_count: to_u64(session_count).map_err(db_error)?,
            pending_segment_count: to_u64(pending).map_err(db_error)?,
        })
    }

    pub fn create_session(
        &self,
        id: &str,
        model_name: &str,
        language: &str,
        smart_punctuation: bool,
        retain_audio: bool,
    ) -> Result<MeetingSession, String> {
        if !valid_session_id(id) || model_name.len() > 128 || language.len() > 32 {
            return Err("The meeting session metadata is invalid.".to_string());
        }
        let started_at_ms = now_ms();
        let connection = self.open_checked()?;
        connection
            .execute(
                "INSERT INTO meeting_sessions(id, started_at_ms, status, model_name, language, smart_punctuation, retain_audio) VALUES (?, ?, 'active', ?, ?, ?, ?)",
                params![id, to_i64(started_at_ms)?, model_name, language, smart_punctuation, retain_audio],
            )
            .map_err(db_error)?;
        self.get_session(id)
    }

    pub fn finish_session(
        &self,
        id: &str,
        status: MeetingSessionStatus,
        error_code: Option<&str>,
    ) -> Result<(), String> {
        if status == MeetingSessionStatus::Active {
            return Err("An active meeting cannot be finalized as active.".to_string());
        }
        let connection = self.open_checked()?;
        connection
            .execute(
                "UPDATE meeting_sessions SET status=?, ended_at_ms=?, error_code=? WHERE id=?",
                params![status.as_db(), to_i64(now_ms())?, error_code, id],
            )
            .map_err(db_error)?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn insert_pending_segment(
        &self,
        session_id: &str,
        speaker: MeetingSpeaker,
        sequence: u64,
        start_ms: u64,
        end_ms: u64,
        audio_relative_path: &str,
    ) -> Result<i64, String> {
        if !valid_session_id(session_id)
            || owned_audio_path(&self.root, audio_relative_path).is_none()
        {
            return Err("The meeting audio spool reference is invalid.".to_string());
        }
        let connection = self.open_checked()?;
        connection
            .execute(
                "INSERT INTO meeting_segments(session_id, speaker, sequence, start_ms, end_ms, status, audio_relative_path) VALUES (?, ?, ?, ?, ?, 'pending', ?)",
                params![
                    session_id,
                    speaker.as_db(),
                    to_i64(sequence)?,
                    to_i64(start_ms)?,
                    to_i64(end_ms)?,
                    audio_relative_path,
                ],
            )
            .map_err(db_error)?;
        Ok(connection.last_insert_rowid())
    }

    pub fn finalize_segment(&self, id: i64, text: &str, keep_audio: bool) -> Result<(), String> {
        if text.len() > 256 * 1024 {
            return Err("The meeting transcript segment is too large.".to_string());
        }
        let mut connection = self.open_checked()?;
        let transaction = connection.transaction().map_err(db_error)?;
        let session_id: String = transaction
            .query_row(
                "SELECT session_id FROM meeting_segments WHERE id=? AND status='pending'",
                [id],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        transaction
            .execute(
                "UPDATE meeting_segments SET status='final', text=?, audio_relative_path=CASE WHEN ? THEN audio_relative_path ELSE NULL END, error_code=NULL WHERE id=? AND status='pending'",
                params![text, keep_audio, id],
            )
            .map_err(db_error)?;
        transaction
            .execute("DELETE FROM meeting_segments_fts WHERE segment_id=?", [id])
            .map_err(db_error)?;
        if !text.trim().is_empty() {
            transaction
                .execute(
                    "INSERT INTO meeting_segments_fts(segment_id, session_id, text) VALUES (?, ?, ?)",
                    params![id, session_id, text],
                )
                .map_err(db_error)?;
        }
        transaction.commit().map_err(db_error)
    }

    pub fn fail_segment(&self, id: i64, error_code: &str) -> Result<(), String> {
        let connection = self.open_checked()?;
        connection
            .execute(
                "UPDATE meeting_segments SET status='failed', error_code=? WHERE id=? AND status='pending'",
                params![error_code, id],
            )
            .map_err(db_error)?;
        Ok(())
    }

    /// Returns only the oldest pending item so crash recovery and live
    /// inference never materialize an unbounded meeting backlog in memory.
    pub fn next_pending_segment(&self) -> Result<Option<PendingMeetingSegment>, String> {
        let connection = self.open_checked()?;
        connection
            .query_row(
                "SELECT g.id, g.session_id, g.speaker, g.sequence, g.start_ms, g.end_ms, g.audio_relative_path, s.model_name, s.language, s.smart_punctuation, s.retain_audio
                 FROM meeting_segments g JOIN meeting_sessions s ON s.id=g.session_id
                 WHERE g.status='pending' AND g.audio_relative_path IS NOT NULL ORDER BY g.id ASC LIMIT 1",
                [],
                |row| {
                Ok(PendingMeetingSegment {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    speaker: MeetingSpeaker::from_db(&row.get::<_, String>(2)?)?,
                    sequence: to_u64(row.get(3)?)?,
                    start_ms: to_u64(row.get(4)?)?,
                    end_ms: to_u64(row.get(5)?)?,
                    audio_relative_path: row.get(6)?,
                    model_name: row.get(7)?,
                    language: row.get(8)?,
                    smart_punctuation: row.get(9)?,
                    retain_audio: row.get(10)?,
                })
                },
            )
            .optional()
            .map_err(db_error)
    }

    pub fn list_sessions(
        &self,
        query: Option<&str>,
        offset: u64,
        limit: u32,
    ) -> Result<MeetingPage, String> {
        let limit = limit.clamp(1, MAX_MEETING_PAGE_SIZE);
        let connection = self.open_checked()?;
        let query = query.map(str::trim).filter(|query| !query.is_empty());
        let (total, sessions) = if let Some(query) = query {
            let fts_query = fts_query(query)?;
            let total = connection
                .query_row(
                    "SELECT COUNT(*) FROM meeting_sessions s WHERE s.id IN (SELECT session_id FROM meeting_segments_fts WHERE meeting_segments_fts MATCH ?)",
                    [&fts_query],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(db_error)?;
            let sql = format!(
                "{} WHERE s.id IN (SELECT session_id FROM meeting_segments_fts WHERE meeting_segments_fts MATCH ?) ORDER BY s.started_at_ms DESC, s.id DESC LIMIT ? OFFSET ?",
                session_query()
            );
            let mut statement = connection.prepare(&sql).map_err(db_error)?;
            let sessions = statement
                .query_map(
                    params![fts_query, i64::from(limit), to_i64(offset)?],
                    row_to_session,
                )
                .map_err(db_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(db_error)?;
            (to_u64(total).map_err(db_error)?, sessions)
        } else {
            let total = connection
                .query_row("SELECT COUNT(*) FROM meeting_sessions", [], |row| {
                    row.get::<_, i64>(0)
                })
                .map_err(db_error)?;
            let sql = format!(
                "{} ORDER BY s.started_at_ms DESC, s.id DESC LIMIT ? OFFSET ?",
                session_query()
            );
            let mut statement = connection.prepare(&sql).map_err(db_error)?;
            let sessions = statement
                .query_map(params![i64::from(limit), to_i64(offset)?], row_to_session)
                .map_err(db_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(db_error)?;
            (to_u64(total).map_err(db_error)?, sessions)
        };
        Ok(MeetingPage {
            sessions,
            total,
            offset,
            limit,
        })
    }

    pub fn get_session(&self, id: &str) -> Result<MeetingSession, String> {
        let connection = self.open_checked()?;
        session_by_id(&connection, id)?
            .ok_or_else(|| "The meeting transcript no longer exists.".to_string())
    }

    pub fn detail(&self, id: &str) -> Result<MeetingDetail, String> {
        let connection = self.open_checked()?;
        let session = session_by_id(&connection, id)?
            .ok_or_else(|| "The meeting transcript no longer exists.".to_string())?;
        let mut statement = connection
            .prepare(
                "SELECT id, session_id, speaker, remote_speaker_id, sequence, start_ms, end_ms, status, text, audio_relative_path IS NOT NULL, error_code
                 FROM meeting_segments WHERE session_id=? ORDER BY start_ms ASC, id ASC",
            )
            .map_err(db_error)?;
        let segments = statement
            .query_map([id], row_to_segment)
            .map_err(db_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_error)?;
        let mut artifact = connection
            .query_row(
                "SELECT artifact_json FROM meeting_artifacts WHERE session_id=?",
                [id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(db_error)?
            .and_then(|json| serde_json::from_str(&json).ok());
        if let Some(value) = artifact.as_mut() {
            let allowed = segments.iter().map(|segment| segment.id).collect();
            if !crate::meeting_artifact::validate_artifact(value, &allowed) {
                artifact = None;
            }
        }
        Ok(MeetingDetail {
            session,
            segments,
            artifact,
        })
    }

    pub fn workspace(&self, id: &str) -> Result<MeetingWorkspace, String> {
        let detail = self.detail(id)?;
        let allowed = detail
            .segments
            .iter()
            .filter(|segment| {
                segment.status == MeetingSegmentStatus::Final && !segment.text.trim().is_empty()
            })
            .map(|segment| segment.id)
            .collect::<std::collections::HashSet<_>>();
        let connection = self.open_checked()?;
        let generated = connection
            .query_row(
                "SELECT artifact_json, revision FROM meeting_artifacts WHERE session_id=?",
                [id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()
            .map_err(db_error)?
            .map(|(json, revision)| {
                let mut artifact: crate::meeting_artifact::MeetingArtifactV1 =
                    serde_json::from_str(&json)
                        .map_err(|_| "The stored meeting draft is invalid.".to_string())?;
                if !crate::meeting_artifact::validate_artifact(&mut artifact, &allowed) {
                    return Err("The stored meeting draft is invalid.".to_string());
                }
                Ok(GeneratedMeetingReview {
                    revision: to_u64(revision).map_err(db_error)?,
                    document: meeting_review::document_from_artifact(&artifact),
                })
            })
            .transpose()?;
        let stored_review = connection
            .query_row(
                "SELECT revision, based_on_artifact_revision, me_label, them_label, review_json
                 FROM meeting_reviews WHERE session_id=?",
                [id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Option<i64>>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(db_error)?;
        let (labels, review) = match stored_review {
            Some((revision, based_on, me, them, json)) => {
                let labels = meeting_review::validate_labels(MeetingSpeakerLabels { me, them })?;
                let mut document = json
                    .map(|json| {
                        serde_json::from_str(&json)
                            .map_err(|_| "The stored meeting review is invalid.".to_string())
                    })
                    .transpose()?;
                if document
                    .as_mut()
                    .is_some_and(|document| !meeting_review::validate_document(document, &allowed))
                {
                    return Err("The stored meeting review is invalid.".into());
                }
                let review = SavedMeetingReview {
                    revision: to_u64(revision).map_err(db_error)?,
                    based_on_generated_revision: based_on
                        .map(to_u64)
                        .transpose()
                        .map_err(db_error)?,
                    document,
                };
                (labels, Some(review))
            }
            None => (MeetingSpeakerLabels::default(), None),
        };
        let (active_document, active_origin) = review
            .as_ref()
            .and_then(|review| review.document.clone())
            .map(|document| (Some(document), Some(ActiveReviewOrigin::Reviewed)))
            .or_else(|| {
                generated.as_ref().map(|generated| {
                    (
                        Some(generated.document.clone()),
                        Some(ActiveReviewOrigin::Generated),
                    )
                })
            })
            .unwrap_or((None, None));
        let mut statement = connection
            .prepare(
                "SELECT speaker_id, label FROM meeting_remote_speakers
                 WHERE session_id=? ORDER BY speaker_id ASC",
            )
            .map_err(db_error)?;
        let remote_speakers = statement
            .query_map([id], |row| {
                Ok(RemoteSpeakerLabel {
                    speaker_id: to_u32(row.get(0)?)?,
                    label: row.get(1)?,
                })
            })
            .map_err(db_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_error)?;
        Ok(MeetingWorkspace {
            session: detail.session,
            segments: detail.segments,
            labels,
            remote_speakers,
            generated,
            review,
            active_document,
            active_origin,
        })
    }

    pub fn apply_remote_speakers(
        &self,
        session_id: &str,
        assignments: &[(i64, u32)],
    ) -> Result<(), String> {
        if !valid_session_id(session_id)
            || assignments
                .iter()
                .any(|(_, speaker_id)| !(1..=32).contains(speaker_id))
        {
            return Err("The remote speaker assignments are invalid.".to_string());
        }
        let mut segment_ids = std::collections::HashSet::with_capacity(assignments.len());
        if assignments
            .iter()
            .any(|(segment_id, _)| !segment_ids.insert(*segment_id))
        {
            return Err("The remote speaker assignments contain a duplicate segment.".to_string());
        }

        let mut connection = self.open_checked()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let status = transaction
            .query_row(
                "SELECT status FROM meeting_sessions WHERE id=?",
                [session_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(db_error)?;
        if status.as_deref() != Some("complete") {
            return Err("Remote speakers can be applied only to a completed meeting.".to_string());
        }
        let existing: i64 = transaction
            .query_row(
                "SELECT COUNT(*) FROM meeting_segments
                 WHERE session_id=? AND remote_speaker_id IS NOT NULL",
                [session_id],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        if existing != 0 {
            return Err("Remote speakers were already applied to this meeting.".to_string());
        }

        let speaker_ids = assignments
            .iter()
            .map(|(_, speaker_id)| *speaker_id)
            .collect::<std::collections::BTreeSet<_>>();
        for speaker_id in speaker_ids {
            transaction
                .execute(
                    "INSERT INTO meeting_remote_speakers(session_id, speaker_id, label)
                     VALUES (?, ?, ?)",
                    params![session_id, speaker_id, format!("Speaker {speaker_id}")],
                )
                .map_err(db_error)?;
        }
        for (segment_id, speaker_id) in assignments {
            let changed = transaction
                .execute(
                    "UPDATE meeting_segments SET remote_speaker_id=?
                     WHERE id=? AND session_id=? AND speaker='them'
                       AND status='final' AND remote_speaker_id IS NULL",
                    params![speaker_id, segment_id, session_id],
                )
                .map_err(db_error)?;
            if changed != 1 {
                return Err(
                    "A remote speaker assignment does not belong to this completed meeting."
                        .to_string(),
                );
            }
        }
        transaction.commit().map_err(db_error)
    }

    pub fn rename_remote_speaker(
        &self,
        session_id: &str,
        speaker_id: u32,
        label: &str,
    ) -> Result<MeetingWorkspace, String> {
        if !valid_session_id(session_id) || !(1..=32).contains(&speaker_id) {
            return Err("The remote speaker label is invalid.".to_string());
        }
        let label = meeting_review::validate_remote_speaker_label(label)?;
        let mut connection = self.open_checked()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_error)?;
        let changed = transaction
            .execute(
                "UPDATE meeting_remote_speakers SET label=?
                 WHERE session_id=? AND speaker_id=?",
                params![label, session_id, speaker_id],
            )
            .map_err(db_error)?;
        if changed != 1 {
            return Err("The remote speaker no longer exists in this meeting.".to_string());
        }
        transaction.commit().map_err(db_error)?;
        self.workspace(session_id)
    }

    pub fn save_artifact(
        &self,
        session_id: &str,
        artifact: &crate::meeting_artifact::MeetingArtifactV1,
        runtime_ms: u64,
        peak_rss_mb: u64,
    ) -> Result<(), String> {
        if !valid_session_id(session_id) {
            return Err(storage_error());
        }
        let json = serde_json::to_string(artifact).map_err(|_| storage_error())?;
        let connection = self.open_checked()?;
        connection
            .execute(
                "INSERT INTO meeting_artifacts(session_id, artifact_json, created_at_ms, runtime_ms, peak_rss_mb, revision)
                 VALUES(?,?,?,?,?,1)
                 ON CONFLICT(session_id) DO UPDATE SET artifact_json=excluded.artifact_json, created_at_ms=excluded.created_at_ms, runtime_ms=excluded.runtime_ms, peak_rss_mb=excluded.peak_rss_mb, revision=meeting_artifacts.revision+1",
                params![session_id, json, to_i64(now_ms())?, to_i64(runtime_ms)?, to_i64(peak_rss_mb)?],
            )
            .map_err(db_error)?;
        Ok(())
    }

    pub fn save_review(
        &self,
        request: SaveMeetingReviewRequest,
    ) -> Result<MeetingWorkspace, String> {
        let session_id = request.session_id.trim().to_string();
        if !valid_session_id(&session_id) {
            return Err("The meeting review request is invalid.".into());
        }
        let labels = meeting_review::validate_labels(request.labels)?;
        let workspace = self.workspace(&session_id)?;
        let allowed = workspace
            .segments
            .iter()
            .filter(|segment| {
                segment.status == MeetingSegmentStatus::Final && !segment.text.trim().is_empty()
            })
            .map(|segment| segment.id)
            .collect::<std::collections::HashSet<_>>();
        let base = match request.base {
            ReviewEditBase::LabelsOnly => None,
            ReviewEditBase::Generated { generated_revision } => {
                let generated = workspace
                    .generated
                    .as_ref()
                    .filter(|item| item.revision == generated_revision)
                    .ok_or_else(|| {
                        "The generated draft changed. Reload it and try again.".to_string()
                    })?;
                Some(&generated.document)
            }
            ReviewEditBase::Review { review_revision } => {
                let review = workspace
                    .review
                    .as_ref()
                    .filter(|item| item.revision == review_revision)
                    .and_then(|item| item.document.as_ref())
                    .ok_or_else(|| "The review changed. Reload it and try again.".to_string())?;
                Some(review)
            }
        };
        let document = match (base, request.document) {
            (Some(base), Some(edit)) => Some(meeting_review::apply_edit(base, edit, &allowed)?),
            (None, None) => workspace
                .review
                .as_ref()
                .and_then(|review| review.document.clone()),
            _ => return Err("The meeting review request is incomplete.".into()),
        };
        let mut connection = self.open_checked()?;
        let transaction = connection.transaction().map_err(db_error)?;
        let current_revision = transaction
            .query_row(
                "SELECT revision FROM meeting_reviews WHERE session_id=?",
                [&session_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(db_error)?
            .map(to_u64)
            .transpose()
            .map_err(db_error)?;
        if current_revision != request.expected_review_revision {
            return Err("The review changed. Reload it and try again.".into());
        }
        if let ReviewEditBase::Generated { generated_revision } = &request.base {
            let current_generated = transaction
                .query_row(
                    "SELECT revision FROM meeting_artifacts WHERE session_id=?",
                    [&session_id],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
                .map_err(db_error)?
                .map(to_u64)
                .transpose()
                .map_err(db_error)?;
            if current_generated != Some(*generated_revision) {
                return Err("The generated draft changed. Reload it and try again.".into());
            }
        }
        let next_revision = current_revision.unwrap_or(0).saturating_add(1);
        let based_on = match &request.base {
            ReviewEditBase::Generated { generated_revision } => Some(*generated_revision),
            ReviewEditBase::Review { .. } => workspace
                .review
                .as_ref()
                .and_then(|review| review.based_on_generated_revision),
            ReviewEditBase::LabelsOnly => workspace
                .review
                .as_ref()
                .and_then(|review| review.based_on_generated_revision),
        };
        let json = document
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|_| storage_error())?;
        transaction
            .execute(
                "INSERT INTO meeting_reviews(session_id, revision, based_on_artifact_revision, me_label, them_label, review_json, updated_at_ms)
                 VALUES(?,?,?,?,?,?,?)
                 ON CONFLICT(session_id) DO UPDATE SET revision=excluded.revision, based_on_artifact_revision=excluded.based_on_artifact_revision, me_label=excluded.me_label, them_label=excluded.them_label, review_json=excluded.review_json, updated_at_ms=excluded.updated_at_ms",
                params![session_id, to_i64(next_revision)?, based_on.map(to_i64).transpose()?, labels.me, labels.them, json, to_i64(now_ms())?],
            )
            .map_err(db_error)?;
        transaction.commit().map_err(db_error)?;
        self.workspace(&session_id)
    }

    pub fn restore_review_from_generated(
        &self,
        request: crate::meeting_review::RestoreMeetingReviewRequest,
    ) -> Result<MeetingWorkspace, String> {
        let workspace = self.workspace(request.session_id.trim())?;
        let generated = workspace
            .generated
            .as_ref()
            .filter(|item| item.revision == request.generated_revision)
            .ok_or_else(|| "The generated draft changed. Reload it and try again.".to_string())?;
        if workspace.review.as_ref().map(|review| review.revision)
            != request.expected_review_revision
        {
            return Err("The review changed. Reload it and try again.".into());
        }
        let edit = crate::meeting_review::EditableReviewDocument {
            summary: crate::meeting_review::EditableReviewText {
                key: generated.document.summary.key.clone(),
                text: generated.document.summary.text.clone(),
            },
            decisions: generated
                .document
                .decisions
                .iter()
                .map(|item| crate::meeting_review::EditableReviewText {
                    key: item.key.clone(),
                    text: item.text.clone(),
                })
                .collect(),
            action_items: generated
                .document
                .action_items
                .iter()
                .map(|item| crate::meeting_review::EditableReviewAction {
                    key: item.key.clone(),
                    text: item.text.clone(),
                    owner: item.owner.clone(),
                    due_date: item.due_date.clone(),
                })
                .collect(),
            open_questions: generated
                .document
                .open_questions
                .iter()
                .map(|item| crate::meeting_review::EditableReviewText {
                    key: item.key.clone(),
                    text: item.text.clone(),
                })
                .collect(),
        };
        self.save_review(SaveMeetingReviewRequest {
            session_id: request.session_id,
            expected_review_revision: request.expected_review_revision,
            base: ReviewEditBase::Generated {
                generated_revision: request.generated_revision,
            },
            labels: workspace.labels,
            document: Some(edit),
        })
    }

    pub fn delete_session(&self, id: &str) -> Result<(), String> {
        let connection = self.open_checked()?;
        let paths = audio_paths_for_session(&connection, id)?;
        connection
            .execute("DELETE FROM meeting_segments_fts WHERE session_id=?", [id])
            .map_err(db_error)?;
        let changed = connection
            .execute("DELETE FROM meeting_sessions WHERE id=?", [id])
            .map_err(db_error)?;
        if changed == 0 {
            return Err("The meeting transcript no longer exists.".to_string());
        }
        for relative in paths {
            if let Some(path) = owned_audio_path(&self.root, &relative) {
                let _ = fs::remove_file(path);
            }
        }
        if valid_session_id(id) {
            let _ = fs::remove_dir(self.root.join("audio").join(id));
        }
        Ok(())
    }

    pub fn delete_all(&self) -> Result<(), String> {
        let connection = self.open_checked()?;
        connection
            .execute_batch(
                "BEGIN IMMEDIATE; DELETE FROM meeting_segments_fts; DELETE FROM meeting_sessions; COMMIT; PRAGMA wal_checkpoint(TRUNCATE); VACUUM;",
            )
            .map_err(db_error)?;
        let audio_root = self.root.join("audio");
        let _ = fs::remove_dir_all(&audio_root);
        fs::create_dir_all(audio_root).map_err(|_| storage_error())
    }

    pub fn prune(&self, retention_days: Option<u32>, max_sessions: u32) -> Result<u64, String> {
        let connection = self.open_checked()?;
        let cutoff = retention_days
            .filter(|days| *days > 0)
            .map(|days| now_ms().saturating_sub(days as u64 * 86_400_000));
        let mut ids = Vec::new();
        if let Some(cutoff) = cutoff {
            let mut statement = connection
                .prepare(
                    "SELECT id FROM meeting_sessions WHERE status!='active' AND started_at_ms < ?",
                )
                .map_err(db_error)?;
            ids.extend(
                statement
                    .query_map([to_i64(cutoff)?], |row| row.get::<_, String>(0))
                    .map_err(db_error)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(db_error)?,
            );
        }
        if max_sessions > 0 {
            let mut statement = connection
                .prepare("SELECT id FROM meeting_sessions WHERE status!='active' ORDER BY started_at_ms DESC LIMIT -1 OFFSET ?")
                .map_err(db_error)?;
            ids.extend(
                statement
                    .query_map([max_sessions], |row| row.get::<_, String>(0))
                    .map_err(db_error)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(db_error)?,
            );
        }
        ids.sort();
        ids.dedup();
        drop(connection);
        for id in &ids {
            self.delete_session(id)?;
        }
        Ok(ids.len() as u64)
    }

    fn open_raw(&self) -> Result<Connection, String> {
        Connection::open(&self.db_path).map_err(|_| storage_error())
    }

    fn open_checked(&self) -> Result<Connection, String> {
        let connection = self.open_raw()?;
        configure_connection(&connection).map_err(db_error)?;
        migrations::quick_check(&connection).map_err(migrations::MeetingDatabaseError::message)?;
        migrations::validate_schema(&connection)
            .map_err(migrations::MeetingDatabaseError::message)?;
        Ok(connection)
    }

    fn create_backup(&self, source: &Connection, version: u32) -> Result<(), String> {
        let path = self.root.join("backups").join(format!(
            "meetings-v{version}-{}-{}.sqlite3",
            now_ms(),
            uuid::Uuid::new_v4()
        ));
        source.backup(MAIN_DB, &path, None).map_err(db_error)?;
        let check = Connection::open(&path).map_err(db_error)?;
        check
            .pragma_update(None, "journal_mode", "DELETE")
            .map_err(db_error)?;
        migrations::quick_check(&check).map_err(migrations::MeetingDatabaseError::message)?;
        migrations::validate_supported_schema(&check)
            .map_err(migrations::MeetingDatabaseError::message)?;
        drop(check);
        fs::File::open(&path)
            .and_then(|file| file.sync_all())
            .map_err(|_| storage_error())?;
        fs::File::open(self.root.join("backups"))
            .and_then(|directory| directory.sync_all())
            .map_err(|_| storage_error())?;
        for old in backup_files_newest_first(&self.root.join("backups"))?
            .into_iter()
            .skip(MAX_BACKUPS)
        {
            let _ = fs::remove_file(&old);
            remove_sidecars(&old);
        }
        Ok(())
    }

    fn recover_corrupt_database(
        &self,
        lease: &InitializationLease,
    ) -> Result<InitializationOutcome, String> {
        let mut candidate = None;
        for backup in backup_files_newest_first(&self.root.join("backups"))? {
            if let Some(restored) = RecoveryCandidate::from_backup(lease, &backup)? {
                candidate = Some(restored);
                break;
            }
        }
        self.quarantine_live_database(lease)?;
        if let Some(candidate) = candidate {
            candidate.publish(&self.db_path, lease)?;
            Ok(InitializationOutcome::Recovered)
        } else {
            Ok(InitializationOutcome::Reinitialized)
        }
    }

    fn quarantine_live_database(&self, _lease: &InitializationLease) -> Result<(), String> {
        let quarantine = self.root.join("quarantine").join(format!(
            "meetings-corrupt-{}-{}.sqlite3",
            now_ms(),
            uuid::Uuid::new_v4()
        ));
        // Move sidecars first so an interrupted quarantine still leaves the main
        // database present. Missing-main recovery also retries the same backups.
        for suffix in ["-wal", "-shm", ""] {
            let source = PathBuf::from(format!("{}{suffix}", self.db_path.display()));
            if source.exists() {
                let target = PathBuf::from(format!("{}{suffix}", quarantine.display()));
                fs::rename(source, target).map_err(|_| storage_error())?;
            }
        }
        fs::File::open(self.root.join("quarantine"))
            .and_then(|directory| directory.sync_all())
            .map_err(|_| storage_error())?;
        fs::File::open(&self.root)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| storage_error())?;
        Ok(())
    }

    fn sweep_orphan_audio(&self) -> Result<(), String> {
        let connection = self.open_checked()?;
        let mut statement = connection
            .prepare("SELECT audio_relative_path FROM meeting_segments WHERE audio_relative_path IS NOT NULL")
            .map_err(db_error)?;
        let owned = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(db_error)?
            .collect::<Result<std::collections::HashSet<_>, _>>()
            .map_err(db_error)?;
        drop(statement);
        drop(connection);
        let audio_root = self.root.join("audio");
        for entry in walk_files(&audio_root)? {
            let Ok(relative) = entry.strip_prefix(&self.root) else {
                continue;
            };
            let relative = relative.to_string_lossy().replace('\\', "/");
            if !owned.contains(&relative) {
                let _ = fs::remove_file(entry);
            }
        }
        Ok(())
    }
}

fn configure_connection(connection: &Connection) -> rusqlite::Result<()> {
    connection.pragma_update(None, "foreign_keys", "ON")?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    connection.pragma_update(None, "secure_delete", "ON")?;
    connection.busy_timeout(std::time::Duration::from_secs(2))
}

fn row_to_session(row: &rusqlite::Row<'_>) -> rusqlite::Result<MeetingSession> {
    let started_at_ms = to_u64(row.get(1)?)?;
    let ended_at_ms = row.get::<_, Option<i64>>(2)?.map(to_u64).transpose()?;
    let duration_ms = ended_at_ms
        .unwrap_or_else(now_ms)
        .saturating_sub(started_at_ms);
    Ok(MeetingSession {
        id: row.get(0)?,
        started_at_ms,
        ended_at_ms,
        status: MeetingSessionStatus::from_db(&row.get::<_, String>(3)?)?,
        model_name: row.get(4)?,
        language: row.get(5)?,
        smart_punctuation: row.get(6)?,
        retain_audio: row.get(7)?,
        duration_ms,
        segment_count: to_u64(row.get(8)?)?,
        preview: row.get(9)?,
        error_code: row.get(10)?,
    })
}

fn session_query() -> &'static str {
    "SELECT s.id, s.started_at_ms, s.ended_at_ms, s.status, s.model_name, s.language, s.smart_punctuation, s.retain_audio,
            (SELECT COUNT(*) FROM meeting_segments g WHERE g.session_id=s.id AND g.status='final'),
            COALESCE((SELECT substr(text, 1, 240) FROM meeting_segments g WHERE g.session_id=s.id AND g.status='final' AND text!='' ORDER BY start_ms ASC, id ASC LIMIT 1), ''),
            s.error_code
     FROM meeting_sessions s"
}

fn session_by_id(connection: &Connection, id: &str) -> Result<Option<MeetingSession>, String> {
    let sql = format!("{} WHERE s.id=?", session_query());
    connection
        .query_row(&sql, [id], row_to_session)
        .optional()
        .map_err(db_error)
}

fn row_to_segment(row: &rusqlite::Row<'_>) -> rusqlite::Result<MeetingSegment> {
    Ok(MeetingSegment {
        id: row.get(0)?,
        session_id: row.get(1)?,
        speaker: MeetingSpeaker::from_db(&row.get::<_, String>(2)?)?,
        remote_speaker_id: row.get::<_, Option<i64>>(3)?.map(to_u32).transpose()?,
        sequence: to_u64(row.get(4)?)?,
        start_ms: to_u64(row.get(5)?)?,
        end_ms: to_u64(row.get(6)?)?,
        status: MeetingSegmentStatus::from_db(&row.get::<_, String>(7)?)?,
        text: row.get(8)?,
        audio_available: row.get(9)?,
        error_code: row.get(10)?,
    })
}

fn fts_query(query: &str) -> Result<String, String> {
    let tokens = query
        .split_whitespace()
        .filter(|token| !token.is_empty())
        .take(16)
        .map(|token| format!("\"{}\"", token.replace('"', "\"\"")))
        .collect::<Vec<_>>();
    if tokens.is_empty() || query.len() > 512 {
        return Err("Enter a shorter meeting search.".to_string());
    }
    Ok(tokens.join(" AND "))
}

fn audio_paths_for_session(connection: &Connection, id: &str) -> Result<Vec<String>, String> {
    let mut statement = connection
        .prepare("SELECT audio_relative_path FROM meeting_segments WHERE session_id=? AND audio_relative_path IS NOT NULL")
        .map_err(db_error)?;
    let paths = statement
        .query_map([id], |row| row.get(0))
        .map_err(db_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_error)?;
    Ok(paths)
}

fn remove_sidecars(path: &Path) {
    for suffix in ["-wal", "-shm"] {
        let _ = fs::remove_file(format!("{}{}", path.display(), suffix));
    }
}

fn backup_files_newest_first(path: &Path) -> Result<Vec<PathBuf>, String> {
    Ok(directory_files_newest_first(path)?
        .into_iter()
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "sqlite3")
        })
        .collect())
}

fn directory_files_newest_first(path: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = fs::read_dir(path)
        .map_err(|_| storage_error())?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    files.sort_by_key(|path| {
        std::cmp::Reverse(
            path.metadata()
                .and_then(|metadata| metadata.modified())
                .unwrap_or(UNIX_EPOCH),
        )
    });
    Ok(files)
}

fn walk_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    if !root.exists() {
        return Ok(files);
    }
    let mut directories = vec![root.to_path_buf()];
    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(directory).map_err(|_| storage_error())? {
            let entry = entry.map_err(|_| storage_error())?;
            let file_type = entry.file_type().map_err(|_| storage_error())?;
            let path = entry.path();
            if file_type.is_dir() {
                directories.push(path);
            } else if file_type.is_file() || file_type.is_symlink() {
                files.push(path);
            }
        }
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn repository() -> (TempDir, MeetingRepository) {
        let temp = TempDir::new().unwrap();
        let (repository, _) = MeetingRepository::initialize(temp.path().to_path_buf()).unwrap();
        (temp, repository)
    }

    fn final_segment(
        repository: &MeetingRepository,
        session_id: &str,
        speaker: MeetingSpeaker,
        sequence: u64,
        text: &str,
    ) -> i64 {
        let channel = speaker.as_db();
        let relative = format!("audio/{session_id}/{channel}-{sequence}.wav");
        let audio = repository.root().join(&relative);
        fs::create_dir_all(audio.parent().unwrap()).unwrap();
        fs::write(&audio, b"wav").unwrap();
        let id = repository
            .insert_pending_segment(
                session_id,
                speaker,
                sequence,
                sequence * 1_000,
                sequence * 1_000 + 500,
                &relative,
            )
            .unwrap();
        repository.finalize_segment(id, text, false).unwrap();
        fs::remove_file(audio).unwrap();
        id
    }

    fn v3_backup_fixture(path: &Path) -> MeetingWorkspace {
        let (_source_root, source) = repository();
        source
            .create_session("recovered-meeting", "base.en", "en", true, false)
            .unwrap();
        let segment = final_segment(
            &source,
            "recovered-meeting",
            MeetingSpeaker::Them,
            0,
            "Preserved transcript evidence",
        );
        source
            .finish_session("recovered-meeting", MeetingSessionStatus::Complete, None)
            .unwrap();
        let artifact = crate::meeting_artifact::MeetingArtifactV1 {
            schema: crate::meeting_artifact::MEETING_ARTIFACT_SCHEMA.into(),
            summary: crate::meeting_artifact::SourcedMeetingText {
                text: "Generated summary".into(),
                source_segment_ids: vec![segment],
            },
            decisions: Vec::new(),
            action_items: Vec::new(),
            open_questions: Vec::new(),
        };
        source
            .save_artifact("recovered-meeting", &artifact, 10, 20)
            .unwrap();
        let expected = source
            .save_review(SaveMeetingReviewRequest {
                session_id: "recovered-meeting".into(),
                expected_review_revision: None,
                base: ReviewEditBase::Generated {
                    generated_revision: 1,
                },
                labels: MeetingSpeakerLabels {
                    me: "Owner".into(),
                    them: "Guests".into(),
                },
                document: Some(crate::meeting_review::EditableReviewDocument {
                    summary: crate::meeting_review::EditableReviewText {
                        key: "summary".into(),
                        text: "Reviewed summary that must survive recovery".into(),
                    },
                    decisions: Vec::new(),
                    action_items: Vec::new(),
                    open_questions: Vec::new(),
                }),
            })
            .unwrap();
        source
            .open_checked()
            .unwrap()
            .backup(MAIN_DB, path, None)
            .unwrap();
        let connection = Connection::open(path).unwrap();
        connection.execute_batch(
            "PRAGMA foreign_keys=OFF;
             BEGIN IMMEDIATE;
             CREATE TABLE meeting_segments_v3 (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               session_id TEXT NOT NULL REFERENCES meeting_sessions(id) ON DELETE CASCADE,
               speaker TEXT NOT NULL CHECK(speaker IN ('me','them')),
               sequence INTEGER NOT NULL CHECK(sequence >= 0),
               start_ms INTEGER NOT NULL CHECK(start_ms >= 0),
               end_ms INTEGER NOT NULL CHECK(end_ms >= start_ms),
               status TEXT NOT NULL CHECK(status IN ('pending','final','failed')),
               text TEXT NOT NULL DEFAULT '', audio_relative_path TEXT, error_code TEXT,
               UNIQUE(session_id, speaker, sequence)
             );
             INSERT INTO meeting_segments_v3 SELECT id, session_id, speaker, sequence, start_ms, end_ms,
               status, text, audio_relative_path, error_code FROM meeting_segments;
             DROP TABLE meeting_segments;
             ALTER TABLE meeting_segments_v3 RENAME TO meeting_segments;
             CREATE INDEX meeting_segments_session_time_idx ON meeting_segments(session_id, start_ms, id);
             CREATE INDEX meeting_segments_pending_idx ON meeting_segments(status, id);
             DROP TABLE meeting_remote_speakers;
             PRAGMA user_version=3;
             COMMIT;
             PRAGMA foreign_keys=ON;
             PRAGMA journal_mode=DELETE;"
        ).unwrap();
        migrations::quick_check(&connection).unwrap();
        assert_eq!(migrations::schema_version(&connection).unwrap(), 3);
        migrations::validate_supported_schema(&connection).unwrap();
        assert!(migrations::validate_schema(&connection).is_err());
        expected
    }

    #[test]
    fn corrupted_main_recovers_healthy_v3_backup_without_changing_backup_or_evidence() {
        let root = TempDir::new().unwrap();
        fs::create_dir(root.path().join("backups")).unwrap();
        let backup = root.path().join("backups/meetings-v3-fixture.sqlite3");
        let expected = v3_backup_fixture(&backup);
        let original_backup = fs::read(&backup).unwrap();
        let corrupt_main = b"corrupt live meeting database";
        fs::write(root.path().join(DATABASE_NAME), corrupt_main).unwrap();

        let (recovered, outcome) =
            MeetingRepository::initialize(root.path().to_path_buf()).unwrap();

        assert_eq!(outcome, InitializationOutcome::Recovered);
        assert_eq!(
            serde_json::to_value(recovered.workspace("recovered-meeting").unwrap()).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
        assert_eq!(
            recovered.status().unwrap().schema_version,
            MEETING_STORE_SCHEMA_VERSION
        );
        assert_eq!(
            recovered
                .list_sessions(Some("Preserved transcript"), 0, 10)
                .unwrap()
                .total,
            1
        );
        assert_eq!(fs::read(&backup).unwrap(), original_backup);
        let quarantined = directory_files_newest_first(&root.path().join("quarantine")).unwrap();
        assert_eq!(quarantined.len(), 1);
        assert_eq!(fs::read(&quarantined[0]).unwrap(), corrupt_main);
        assert!(!fs::read_dir(root.path())
            .unwrap()
            .flatten()
            .any(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with(".meeting-recovery-")));
    }

    #[test]
    fn corrupted_main_recovers_committed_v3_wal_snapshot() {
        let root = TempDir::new().unwrap();
        fs::create_dir(root.path().join("backups")).unwrap();
        let backup = root.path().join("backups/meetings-v3-wal.sqlite3");
        let mut expected = v3_backup_fixture(&backup);
        let writer = Connection::open(&backup).unwrap();
        writer.pragma_update(None, "journal_mode", "WAL").unwrap();
        writer
            .execute("UPDATE meeting_sessions SET model_name='tiny.en'", [])
            .unwrap();
        expected.session.model_name = "tiny.en".into();
        let original_backup = fs::read(&backup).unwrap();
        assert!(PathBuf::from(format!("{}-wal", backup.display())).exists());
        fs::write(root.path().join(DATABASE_NAME), b"corrupt database").unwrap();

        let (recovered, outcome) =
            MeetingRepository::initialize(root.path().to_path_buf()).unwrap();

        assert_eq!(outcome, InitializationOutcome::Recovered);
        assert_eq!(
            serde_json::to_value(recovered.workspace("recovered-meeting").unwrap()).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
        assert_eq!(migrations::schema_version(&writer).unwrap(), 3);
        assert_eq!(fs::read(&backup).unwrap(), original_backup);
    }

    #[test]
    fn recovery_migrates_supported_v1_and_v2_backups() {
        for version in [1, 2] {
            let root = TempDir::new().unwrap();
            fs::create_dir(root.path().join("backups")).unwrap();
            let backup = root.path().join("backups/meetings-older.sqlite3");
            let mut expected = v3_backup_fixture(&backup);
            let connection = Connection::open(&backup).unwrap();
            connection.execute_batch("DROP TABLE meeting_reviews; ALTER TABLE meeting_artifacts DROP COLUMN revision;").unwrap();
            if version == 1 {
                connection
                    .execute_batch("DROP TABLE meeting_artifacts")
                    .unwrap();
            }
            connection
                .pragma_update(None, "user_version", version)
                .unwrap();
            migrations::validate_supported_schema(&connection).unwrap();
            drop(connection);
            expected.review = None;
            expected.labels = MeetingSpeakerLabels::default();
            if version == 1 {
                expected.generated = None;
            }
            expected.active_document = expected
                .generated
                .as_ref()
                .map(|generated| generated.document.clone());
            expected.active_origin = expected
                .generated
                .as_ref()
                .map(|_| ActiveReviewOrigin::Generated);
            let original = fs::read(&backup).unwrap();
            fs::write(root.path().join(DATABASE_NAME), b"corrupt live database").unwrap();
            let (recovered, outcome) =
                MeetingRepository::initialize(root.path().to_path_buf()).unwrap();
            assert_eq!(outcome, InitializationOutcome::Recovered);
            assert_eq!(
                serde_json::to_value(recovered.workspace("recovered-meeting").unwrap()).unwrap(),
                serde_json::to_value(expected).unwrap()
            );
            assert_eq!(fs::read(&backup).unwrap(), original);
        }
    }

    #[test]
    fn locked_backup_validation_aborts_without_quarantining_the_main() {
        let root = TempDir::new().unwrap();
        fs::create_dir(root.path().join("backups")).unwrap();
        let backup = root.path().join("backups/meetings-v3-locked.sqlite3");
        let expected = v3_backup_fixture(&backup);
        let writer = Connection::open(&backup).unwrap();
        writer.execute_batch("BEGIN EXCLUSIVE").unwrap();
        let original_backup = fs::read(&backup).unwrap();
        let original_main = b"corrupt live database awaiting readable backup";
        fs::write(root.path().join(DATABASE_NAME), original_main).unwrap();

        let result = MeetingRepository::initialize(root.path().to_path_buf());

        assert!(
            result.is_err(),
            "A busy healthy backup must not trigger reinitialization"
        );
        assert_eq!(
            fs::read(root.path().join(DATABASE_NAME)).unwrap(),
            original_main
        );
        assert_eq!(fs::read(&backup).unwrap(), original_backup);
        assert_eq!(
            fs::read_dir(root.path().join("quarantine"))
                .unwrap()
                .count(),
            0
        );
        writer.execute_batch("ROLLBACK").unwrap();
        let (recovered, outcome) =
            MeetingRepository::initialize(root.path().to_path_buf()).unwrap();
        assert_eq!(outcome, InitializationOutcome::Recovered);
        assert_eq!(
            serde_json::to_value(recovered.workspace("recovered-meeting").unwrap()).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
    }

    #[test]
    fn locked_live_database_is_not_misclassified_as_corrupt() {
        let (root, repository) = repository();
        repository
            .create_session("live", "base.en", "en", true, false)
            .unwrap();
        final_segment(
            &repository,
            "live",
            MeetingSpeaker::Me,
            0,
            "New evidence after the startup backup",
        );
        repository
            .finish_session("live", MeetingSessionStatus::Complete, None)
            .unwrap();
        let writer = Connection::open(&repository.db_path).unwrap();
        writer
            .pragma_update(None, "journal_mode", "DELETE")
            .unwrap();
        writer.execute_batch("BEGIN EXCLUSIVE").unwrap();
        let original = fs::read(&repository.db_path).unwrap();

        let result = MeetingRepository::initialize(root.path().to_path_buf());

        assert!(
            result.is_err(),
            "A busy main database must not be quarantined or restored over"
        );
        assert_eq!(fs::read(&repository.db_path).unwrap(), original);
        assert_eq!(
            fs::read_dir(root.path().join("quarantine"))
                .unwrap()
                .count(),
            0
        );
        writer.execute_batch("ROLLBACK").unwrap();
        let (opened, outcome) = MeetingRepository::initialize(root.path().to_path_buf()).unwrap();
        assert_eq!(outcome, InitializationOutcome::Opened);
        assert_eq!(
            opened.detail("live").unwrap().segments[0].text,
            "New evidence after the startup backup"
        );
    }

    #[test]
    fn schema_inspection_preserves_operational_errors_from_a_locked_disk_backup() {
        let root = TempDir::new().unwrap();
        let backup = root.path().join("v3.sqlite3");
        v3_backup_fixture(&backup);
        let reader =
            Connection::open_with_flags(&backup, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        reader.busy_timeout(std::time::Duration::ZERO).unwrap();
        migrations::quick_check(&reader).unwrap();
        let writer = Connection::open(&backup).unwrap();
        writer.execute_batch("BEGIN EXCLUSIVE").unwrap();
        for result in [
            migrations::quick_check(&reader),
            migrations::validate_supported_schema(&reader),
        ] {
            let error = result.unwrap_err();
            assert!(
                matches!(&error, migrations::MeetingDatabaseError::Sqlite(rusqlite::Error::SqliteFailure(cause, _), _) if matches!(cause.code, rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked))
            );
            assert!(!error.is_invalid_backup());
            assert!(!error.is_corrupt());
        }
        writer.execute_batch("ROLLBACK").unwrap();
        migrations::quick_check(&reader).unwrap();
        migrations::validate_supported_schema(&reader).unwrap();
    }

    #[test]
    fn initialization_lease_child_process() {
        use std::io::Read;
        let Some(root) = std::env::var_os("MURMUR_TEST_MEETING_INITIALIZE_ROOT").map(PathBuf::from)
        else {
            return;
        };
        let lease = InitializationLease::acquire(&root).unwrap();
        let candidate = RecoveryCandidate::from_backup(&lease, &root.join("backups/v3.sqlite3"))
            .unwrap()
            .unwrap();
        let connection = Connection::open(&candidate.path).unwrap();
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .unwrap();
        connection
            .execute(
                "UPDATE meeting_sessions SET language='candidate-active'",
                [],
            )
            .unwrap();
        fs::write(
            root.join("initialize-child-ready"),
            candidate.path.file_name().unwrap().as_encoded_bytes(),
        )
        .unwrap();
        let _ = std::io::stdin().read_exact(&mut [0u8]);
        drop(connection);
        drop(candidate);
        drop(lease);
    }

    #[test]
    fn another_process_cannot_sweep_an_active_candidate_and_crash_releases_the_lease() {
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};
        let root = TempDir::new().unwrap();
        fs::create_dir(root.path().join("backups")).unwrap();
        let backup = root.path().join("backups/v3.sqlite3");
        let expected = v3_backup_fixture(&backup);
        let original_backup = fs::read(&backup).unwrap();
        let original_main = b"corrupt main awaiting the leased recovery";
        fs::write(root.path().join(DATABASE_NAME), original_main).unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "meeting_store::repository::tests::initialization_lease_child_process",
                "--test-threads=1",
            ])
            .env("MURMUR_TEST_MEETING_INITIALIZE_ROOT", root.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let ready = root.path().join("initialize-child-ready");
        let deadline = Instant::now() + Duration::from_secs(10);
        while !ready.exists() && Instant::now() < deadline {
            assert!(
                child.try_wait().unwrap().is_none(),
                "Lease holder exited before publishing its candidate"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(ready.exists(), "Lease holder did not become ready");
        let candidate = root.path().join(fs::read_to_string(&ready).unwrap());
        let protected = ["", "-wal", "-shm"].map(|suffix| {
            let path = PathBuf::from(format!("{}{suffix}", candidate.display()));
            let bytes = fs::read(&path).unwrap();
            (path, bytes)
        });

        let result = MeetingRepository::initialize(root.path().to_path_buf());

        assert!(result.unwrap_err().contains("Another Murmur instance"));
        for (path, bytes) in &protected {
            assert_eq!(fs::read(path).unwrap(), *bytes);
        }
        assert_eq!(
            fs::read(root.path().join(DATABASE_NAME)).unwrap(),
            original_main
        );
        assert_eq!(fs::read(&backup).unwrap(), original_backup);
        assert!(!root.path().join("quarantine").exists());
        child.kill().unwrap();
        child.wait().unwrap();
        drop(input);
        assert!(
            candidate.exists(),
            "Hard termination must leave the disposable candidate for startup cleanup"
        );

        let (recovered, outcome) =
            MeetingRepository::initialize(root.path().to_path_buf()).unwrap();
        assert_eq!(outcome, InitializationOutcome::Recovered);
        for (path, _) in protected {
            assert!(!path.exists());
        }
        assert_eq!(
            serde_json::to_value(recovered.workspace("recovered-meeting").unwrap()).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
        assert_eq!(fs::read(&backup).unwrap(), original_backup);
    }

    #[test]
    fn recovery_does_not_treat_disk_full_as_an_invalid_backup() {
        let root = TempDir::new().unwrap();
        let backup = root.path().join("v3-full.sqlite3");
        v3_backup_fixture(&backup);
        let connection = Connection::open(&backup).unwrap();
        connection.execute_batch("VACUUM").unwrap();
        let pages: u32 = connection
            .pragma_query_value(None, "page_count", |row| row.get(0))
            .unwrap();
        let free: u32 = connection
            .pragma_query_value(None, "freelist_count", |row| row.get(0))
            .unwrap();
        assert_eq!(free, 0);
        connection
            .pragma_update(None, "max_page_count", pages)
            .unwrap();
        let original = fs::read(&backup).unwrap();

        let error = migrations::migrate(&connection).unwrap_err();

        assert!(
            matches!(&error, migrations::MeetingDatabaseError::Sqlite(rusqlite::Error::SqliteFailure(cause, _), _) if cause.code == rusqlite::ErrorCode::DiskFull)
        );
        assert!(!error.is_invalid_backup());
        drop(connection);
        assert_eq!(fs::read(&backup).unwrap(), original);
    }

    #[test]
    fn recovery_rejects_invalid_newer_backups_and_keeps_the_healthy_source() {
        for fault in [
            "future",
            "missing_column",
            "migration_conflict",
            "foreign_key",
            "corrupt",
        ] {
            let root = TempDir::new().unwrap();
            fs::create_dir(root.path().join("backups")).unwrap();
            let good = root.path().join("backups/meetings-v3-good.sqlite3");
            let expected = v3_backup_fixture(&good);
            let bad = root.path().join("backups/meetings-v3-newer.sqlite3");
            fs::copy(&good, &bad).unwrap();
            if fault == "corrupt" {
                fs::write(&bad, b"corrupt backup").unwrap();
            } else {
                let connection = Connection::open(&bad).unwrap();
                let sql = match fault {
                    "future" => "PRAGMA user_version=5",
                    "missing_column" => "ALTER TABLE meeting_reviews DROP COLUMN me_label",
                    "migration_conflict" => {
                        "CREATE TABLE meeting_remote_speakers(unexpected INTEGER)"
                    }
                    "foreign_key" => {
                        "PRAGMA foreign_keys=OFF; UPDATE meeting_segments SET session_id='missing'"
                    }
                    _ => unreachable!(),
                };
                connection.execute_batch(sql).unwrap();
            }
            fs::File::open(&good)
                .unwrap()
                .set_modified(UNIX_EPOCH + std::time::Duration::from_secs(60))
                .unwrap();
            fs::File::open(&bad)
                .unwrap()
                .set_modified(UNIX_EPOCH + std::time::Duration::from_secs(120))
                .unwrap();
            let good_bytes = fs::read(&good).unwrap();
            let bad_bytes = fs::read(&bad).unwrap();
            fs::write(root.path().join(DATABASE_NAME), b"corrupt live database").unwrap();

            let (recovered, outcome) =
                MeetingRepository::initialize(root.path().to_path_buf()).unwrap();

            assert_eq!(outcome, InitializationOutcome::Recovered, "{fault}");
            assert_eq!(
                serde_json::to_value(recovered.workspace("recovered-meeting").unwrap()).unwrap(),
                serde_json::to_value(expected).unwrap(),
                "{fault}"
            );
            assert_eq!(fs::read(&good).unwrap(), good_bytes, "{fault}");
            assert_eq!(fs::read(&bad).unwrap(), bad_bytes, "{fault}");
            assert!(
                !fs::read_dir(root.path())
                    .unwrap()
                    .flatten()
                    .any(|entry| entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with(".meeting-recovery-")),
                "{fault}"
            );
        }
    }

    #[test]
    fn recovery_retries_backup_when_main_is_missing_after_an_interrupted_publish() {
        let root = TempDir::new().unwrap();
        fs::create_dir(root.path().join("backups")).unwrap();
        let backup = root.path().join("backups/meetings-v3-fixture.sqlite3");
        let expected = v3_backup_fixture(&backup);
        let abandoned = root.path().join(format!(
            ".meeting-recovery-{}.sqlite3",
            uuid::Uuid::new_v4()
        ));
        for suffix in ["", "-wal", "-shm"] {
            fs::write(
                format!("{}{suffix}", abandoned.display()),
                b"abandoned staging data",
            )
            .unwrap();
        }
        let unrelated = root.path().join(".meeting-recovery-user.sqlite3");
        fs::write(&unrelated, b"preserve unrelated file").unwrap();
        let (recovered, outcome) =
            MeetingRepository::initialize(root.path().to_path_buf()).unwrap();
        assert_eq!(outcome, InitializationOutcome::Recovered);
        for suffix in ["", "-wal", "-shm"] {
            assert!(!PathBuf::from(format!("{}{suffix}", abandoned.display())).exists());
        }
        assert_eq!(fs::read(unrelated).unwrap(), b"preserve unrelated file");
        assert_eq!(
            serde_json::to_value(recovered.workspace("recovered-meeting").unwrap()).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
    }

    #[test]
    fn quarantine_preserves_main_and_sidecar_evidence() {
        let (root, repository) = repository();
        for (suffix, bytes) in [
            ("", b"main evidence".as_slice()),
            ("-wal", b"wal evidence"),
            ("-shm", b"shm evidence"),
        ] {
            fs::write(format!("{}{suffix}", repository.db_path.display()), bytes).unwrap();
        }
        let lease = InitializationLease::acquire(repository.root()).unwrap();
        repository.quarantine_live_database(&lease).unwrap();
        let files = directory_files_newest_first(&root.path().join("quarantine")).unwrap();
        assert_eq!(files.len(), 3);
        for (suffix, bytes) in [
            (".sqlite3", b"main evidence".as_slice()),
            (".sqlite3-wal", b"wal evidence"),
            (".sqlite3-shm", b"shm evidence"),
        ] {
            let path = files
                .iter()
                .find(|path| path.to_string_lossy().ends_with(suffix))
                .unwrap();
            assert_eq!(fs::read(path).unwrap(), bytes);
        }
        assert!(!repository.db_path.exists());
    }

    #[test]
    fn backup_retention_counts_standalone_snapshots_not_sidecars() {
        let (root, repository) = repository();
        let connection = repository.open_checked().unwrap();
        for _ in 0..5 {
            repository
                .create_backup(&connection, MEETING_STORE_SCHEMA_VERSION)
                .unwrap();
        }
        let files = directory_files_newest_first(&root.path().join("backups")).unwrap();
        assert_eq!(files.len(), MAX_BACKUPS);
        for path in files {
            assert_eq!(path.extension().unwrap(), "sqlite3");
            let check =
                Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
            migrations::quick_check(&check).unwrap();
            migrations::validate_schema(&check).unwrap();
            let journal: String = check
                .pragma_query_value(None, "journal_mode", |row| row.get(0))
                .unwrap();
            assert_eq!(journal, "delete");
        }
    }

    #[test]
    fn pending_finalize_search_and_delete_are_durable() {
        let (_temp, repository) = repository();
        repository
            .create_session("session-1", "base.en", "en", true, false)
            .unwrap();
        let audio = repository.root().join("audio/session-1/me-0.wav");
        fs::create_dir_all(audio.parent().unwrap()).unwrap();
        fs::write(&audio, b"wav").unwrap();
        let segment = repository
            .insert_pending_segment(
                "session-1",
                MeetingSpeaker::Me,
                0,
                10,
                20,
                "audio/session-1/me-0.wav",
            )
            .unwrap();
        repository
            .finalize_segment(segment, "private sentinel transcript", false)
            .unwrap();
        fs::remove_file(audio).unwrap();
        let page = repository.list_sessions(Some("sentinel"), 0, 20).unwrap();
        assert_eq!(page.total, 1);
        let detail = repository.detail("session-1").unwrap();
        assert_eq!(detail.segments[0].text, "private sentinel transcript");
        assert!(!detail.segments[0].audio_available);
        repository.delete_session("session-1").unwrap();
        assert_eq!(repository.list_sessions(None, 0, 20).unwrap().total, 0);
    }

    #[test]
    fn startup_marks_active_sessions_interrupted_and_keeps_pending_audio() {
        let (temp, repository) = repository();
        repository
            .create_session("session-2", "base.en", "en", true, true)
            .unwrap();
        fs::create_dir_all(repository.root().join("audio/session-2")).unwrap();
        fs::write(repository.root().join("audio/session-2/them-0.wav"), b"wav").unwrap();
        repository
            .insert_pending_segment(
                "session-2",
                MeetingSpeaker::Them,
                0,
                0,
                100,
                "audio/session-2/them-0.wav",
            )
            .unwrap();
        drop(repository);
        let (reopened, _) = MeetingRepository::initialize(temp.path().to_path_buf()).unwrap();
        assert_eq!(
            reopened.get_session("session-2").unwrap().status,
            MeetingSessionStatus::Interrupted
        );
        assert!(reopened.next_pending_segment().unwrap().is_some());
    }

    #[test]
    fn orphan_audio_is_swept_without_deleting_owned_pending_chunks() {
        let (temp, repository) = repository();
        repository
            .create_session("session-3", "base.en", "en", true, true)
            .unwrap();
        fs::create_dir_all(repository.root().join("audio/session-3")).unwrap();
        let owned = repository.root().join("audio/session-3/me-0.wav");
        let orphan = repository.root().join("audio/session-3/orphan.wav");
        fs::write(&owned, b"wav").unwrap();
        fs::write(&orphan, b"wav").unwrap();
        repository
            .insert_pending_segment(
                "session-3",
                MeetingSpeaker::Me,
                0,
                0,
                10,
                "audio/session-3/me-0.wav",
            )
            .unwrap();
        drop(repository);
        let (_reopened, _) = MeetingRepository::initialize(temp.path().to_path_buf()).unwrap();
        assert!(owned.exists());
        assert!(!orphan.exists());
    }

    #[test]
    fn list_is_sql_bounded_and_audio_paths_cannot_escape_the_store() {
        let (_temp, repository) = repository();
        for id in ["session-a", "session-b", "session-c"] {
            repository
                .create_session(id, "base.en", "en", true, false)
                .unwrap();
        }
        let page = repository.list_sessions(None, 1, 2).unwrap();
        assert_eq!(page.total, 3);
        assert_eq!(page.sessions.len(), 2);
        assert!(repository.audio_path("audio/session-a/me-0.wav").is_ok());
        assert!(repository.audio_path("audio/../outside.wav").is_err());
        assert!(repository.audio_path("../outside.wav").is_err());
        assert!(repository
            .insert_pending_segment(
                "session-a",
                MeetingSpeaker::Me,
                0,
                0,
                10,
                "audio/session-a/../../outside.wav",
            )
            .is_err());
    }

    #[test]
    fn regeneration_preserves_review_and_restore_is_explicit() {
        let (_temp, repository) = repository();
        repository
            .create_session("review-session", "base.en", "en", true, false)
            .unwrap();
        let audio = repository.root().join("audio/review-session/me-0.wav");
        fs::create_dir_all(audio.parent().unwrap()).unwrap();
        fs::write(&audio, b"wav").unwrap();
        let segment = repository
            .insert_pending_segment(
                "review-session",
                MeetingSpeaker::Me,
                0,
                0,
                1_000,
                "audio/review-session/me-0.wav",
            )
            .unwrap();
        repository
            .finalize_segment(segment, "Evidence", false)
            .unwrap();
        fs::remove_file(audio).unwrap();
        let artifact = crate::meeting_artifact::MeetingArtifactV1 {
            schema: crate::meeting_artifact::MEETING_ARTIFACT_SCHEMA.into(),
            summary: crate::meeting_artifact::SourcedMeetingText {
                text: "Generated one".into(),
                source_segment_ids: vec![segment],
            },
            decisions: Vec::new(),
            action_items: Vec::new(),
            open_questions: Vec::new(),
        };
        repository
            .save_artifact("review-session", &artifact, 1, 2)
            .unwrap();
        let workspace = repository.workspace("review-session").unwrap();
        let generated_revision = workspace.generated.as_ref().unwrap().revision;
        let saved_review = repository
            .save_review(SaveMeetingReviewRequest {
                session_id: "review-session".into(),
                expected_review_revision: None,
                base: ReviewEditBase::Generated { generated_revision },
                labels: MeetingSpeakerLabels {
                    me: "George".into(),
                    them: "Team".into(),
                },
                document: Some(crate::meeting_review::EditableReviewDocument {
                    summary: crate::meeting_review::EditableReviewText {
                        key: "summary".into(),
                        text: "Reviewed wording".into(),
                    },
                    decisions: Vec::new(),
                    action_items: Vec::new(),
                    open_questions: Vec::new(),
                }),
            })
            .unwrap();
        let saved_revision = saved_review.review.as_ref().unwrap().revision;
        let labels_only = repository
            .save_review(SaveMeetingReviewRequest {
                session_id: "review-session".into(),
                expected_review_revision: Some(saved_revision),
                base: ReviewEditBase::LabelsOnly,
                labels: MeetingSpeakerLabels {
                    me: "George N.".into(),
                    them: "Team".into(),
                },
                document: None,
            })
            .unwrap();
        assert_eq!(
            labels_only.active_document.as_ref().unwrap().summary.text,
            "Reviewed wording"
        );
        assert_eq!(labels_only.labels.me, "George N.");

        let mut replacement = artifact.clone();
        replacement.summary.text = "Generated two".into();
        repository
            .save_artifact("review-session", &replacement, 3, 4)
            .unwrap();
        let after_regeneration = repository.workspace("review-session").unwrap();
        assert_eq!(
            after_regeneration.active_document.unwrap().summary.text,
            "Reviewed wording"
        );
        assert_eq!(after_regeneration.labels.me, "George N.");

        let generated_revision = after_regeneration.generated.as_ref().unwrap().revision;
        let review_revision = after_regeneration.review.as_ref().unwrap().revision;
        let restored = repository
            .restore_review_from_generated(crate::meeting_review::RestoreMeetingReviewRequest {
                session_id: "review-session".into(),
                generated_revision,
                expected_review_revision: Some(review_revision),
            })
            .unwrap();
        assert_eq!(
            restored.active_document.unwrap().summary.text,
            "Generated two"
        );
    }

    #[test]
    fn remote_speaker_assignments_are_atomic_bounded_and_session_scoped() {
        let (_temp, repository) = repository();
        for session_id in ["first", "second"] {
            repository
                .create_session(session_id, "base.en", "en", true, false)
                .unwrap();
        }
        let first_remote = final_segment(
            &repository,
            "first",
            MeetingSpeaker::Them,
            0,
            "Remote evidence",
        );
        let first_mic = final_segment(&repository, "first", MeetingSpeaker::Me, 0, "Mic evidence");
        let second_remote = final_segment(
            &repository,
            "second",
            MeetingSpeaker::Them,
            0,
            "Other session evidence",
        );
        assert!(repository
            .apply_remote_speakers("first", &[(first_remote, 1)])
            .is_err());
        for session_id in ["first", "second"] {
            repository
                .finish_session(session_id, MeetingSessionStatus::Complete, None)
                .unwrap();
        }

        assert!(repository
            .apply_remote_speakers("first", &[(first_remote, 1), (first_remote, 2)])
            .is_err());
        assert!(repository
            .apply_remote_speakers("first", &[(first_remote, 1), (first_mic, 2)])
            .is_err());
        let rolled_back = repository.workspace("first").unwrap();
        assert!(rolled_back.remote_speakers.is_empty());
        assert!(rolled_back
            .segments
            .iter()
            .all(|segment| segment.remote_speaker_id.is_none()));

        repository
            .apply_remote_speakers("first", &[(first_remote, 1)])
            .unwrap();
        repository
            .apply_remote_speakers("second", &[(second_remote, 1)])
            .unwrap();
        assert!(repository
            .apply_remote_speakers("first", &[(first_remote, 2)])
            .is_err());
        assert!(repository
            .apply_remote_speakers("second", &[(second_remote, 0)])
            .is_err());

        let renamed = repository
            .rename_remote_speaker("first", 1, "  Casey  ")
            .unwrap();
        assert_eq!(renamed.remote_speakers[0].label, "Casey");
        assert_eq!(renamed.segments[0].text, "Remote evidence");
        assert_eq!(renamed.segments[0].remote_speaker_id, Some(1));
        assert_eq!(renamed.segments[1].text, "Mic evidence");
        assert_eq!(renamed.segments[1].remote_speaker_id, None);
        assert_eq!(
            repository.workspace("second").unwrap().remote_speakers[0].label,
            "Speaker 1"
        );
    }

    #[test]
    fn remote_speaker_foreign_keys_reject_cross_session_and_cascade_on_delete() {
        let (_temp, repository) = repository();
        for session_id in ["owner", "other"] {
            repository
                .create_session(session_id, "base.en", "en", true, false)
                .unwrap();
        }
        let owner_remote = final_segment(
            &repository,
            "owner",
            MeetingSpeaker::Them,
            0,
            "Owner remote",
        );
        let other_remote = final_segment(
            &repository,
            "other",
            MeetingSpeaker::Them,
            0,
            "Other remote",
        );
        for session_id in ["owner", "other"] {
            repository
                .finish_session(session_id, MeetingSessionStatus::Complete, None)
                .unwrap();
        }
        repository
            .apply_remote_speakers("owner", &[(owner_remote, 1)])
            .unwrap();

        let connection = repository.open_checked().unwrap();
        assert!(connection
            .execute(
                "UPDATE meeting_segments SET remote_speaker_id=1 WHERE id=?",
                [other_remote],
            )
            .is_err());
        drop(connection);

        repository.delete_session("owner").unwrap();
        let connection = repository.open_checked().unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM meeting_remote_speakers WHERE session_id='owner'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            0
        );
    }
}
