use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OpenFlags, OptionalExtension, MAIN_DB};

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

impl MeetingRepository {
    pub fn initialize(root: PathBuf) -> Result<(Self, InitializationOutcome), String> {
        fs::create_dir_all(root.join("audio")).map_err(|_| storage_error())?;
        fs::create_dir_all(root.join("backups")).map_err(|_| storage_error())?;
        fs::create_dir_all(root.join("quarantine")).map_err(|_| storage_error())?;
        let repository = Self {
            db_path: root.join(DATABASE_NAME),
            root,
        };
        let mut outcome = InitializationOutcome::Opened;
        if repository.db_path.exists() {
            let valid = repository
                .open_raw()
                .and_then(|connection| {
                    configure_connection(&connection)?;
                    migrations::quick_check(&connection)
                })
                .is_ok();
            if !valid {
                outcome = repository.recover_corrupt_database()?;
            }
        }
        let connection = repository.open_raw()?;
        configure_connection(&connection)?;
        let old_version = migrations::schema_version(&connection)?;
        if old_version > 0 && old_version < MEETING_STORE_SCHEMA_VERSION {
            repository.create_backup(&connection, old_version)?;
        }
        migrations::migrate(&connection)?;
        migrations::quick_check(&connection)?;
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
                "SELECT id, session_id, speaker, sequence, start_ms, end_ms, status, text, audio_relative_path IS NOT NULL, error_code
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
        Ok(MeetingWorkspace {
            session: detail.session,
            segments: detail.segments,
            labels,
            generated,
            review,
            active_document,
            active_origin,
        })
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
        configure_connection(&connection)?;
        migrations::quick_check(&connection)?;
        migrations::validate_schema(&connection)?;
        Ok(connection)
    }

    fn create_backup(&self, source: &Connection, version: u32) -> Result<(), String> {
        let path = self
            .root
            .join("backups")
            .join(format!("meetings-v{version}-{}.sqlite3", now_ms()));
        source.backup(MAIN_DB, &path, None).map_err(db_error)?;
        let check = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|_| storage_error())?;
        migrations::quick_check(&check)?;
        let mut backups = directory_files_newest_first(&self.root.join("backups"))?;
        for old in backups.drain(MAX_BACKUPS..) {
            let _ = fs::remove_file(old);
        }
        Ok(())
    }

    fn recover_corrupt_database(&self) -> Result<InitializationOutcome, String> {
        let quarantine = self
            .root
            .join("quarantine")
            .join(format!("meetings-corrupt-{}.sqlite3", now_ms()));
        fs::rename(&self.db_path, quarantine).map_err(|_| storage_error())?;
        remove_sidecars(&self.db_path);
        for backup in directory_files_newest_first(&self.root.join("backups"))? {
            let valid = Connection::open_with_flags(&backup, OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(|_| storage_error())
                .and_then(|connection| {
                    migrations::quick_check(&connection)?;
                    migrations::validate_schema(&connection)
                })
                .is_ok();
            if valid {
                fs::copy(backup, &self.db_path).map_err(|_| storage_error())?;
                return Ok(InitializationOutcome::Recovered);
            }
        }
        Ok(InitializationOutcome::Reinitialized)
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

fn configure_connection(connection: &Connection) -> Result<(), String> {
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .map_err(db_error)?;
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .map_err(db_error)?;
    connection
        .pragma_update(None, "synchronous", "FULL")
        .map_err(db_error)?;
    connection
        .pragma_update(None, "secure_delete", "ON")
        .map_err(db_error)?;
    connection
        .busy_timeout(std::time::Duration::from_secs(2))
        .map_err(db_error)
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
        sequence: to_u64(row.get(3)?)?,
        start_ms: to_u64(row.get(4)?)?,
        end_ms: to_u64(row.get(5)?)?,
        status: MeetingSegmentStatus::from_db(&row.get::<_, String>(6)?)?,
        text: row.get(7)?,
        audio_available: row.get(8)?,
        error_code: row.get(9)?,
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
}
