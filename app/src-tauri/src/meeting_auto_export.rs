//! Optional hand-off of completed meetings to a local folder.
//!
//! When enabled at meeting start, a completed session is rendered with the
//! existing Markdown review export, prefixed with YAML front matter, and
//! published atomically into the chosen folder so a local watcher can pick it
//! up. It runs once per session, after remote speaker labels settle, on its own
//! thread. Nothing here touches the network, and logs carry only stable codes.

use crate::meeting_review::{render_export, MeetingReviewExportFormat};
use crate::meeting_store::{
    MeetingRepository, MeetingSession, MeetingSessionStatus, MeetingTitleSource,
};
use chrono::{DateTime, SecondsFormat, TimeZone};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

const MARKER_DIRECTORY: &str = "auto-export";
const MAX_TITLE_CHARS: usize = 80;
const MAX_NAME_ATTEMPTS: u32 = 100;
const UNTITLED: &str = "Untitled";

/// Default destination, relative to the home directory.
const DEFAULT_RELATIVE_DIRECTORY: [&str; 2] = ["Meetings", "inbox"];

/// Calendar event that overlapped an untitled meeting. Never persisted.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct CalendarMatch {
    pub title: String,
    pub attendees: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ExportOutcome {
    Written(PathBuf),
    AlreadyExported,
    NotComplete,
}

/// Resolve the configured folder once at meeting start. Empty selects
/// `~/Meetings/inbox`; a leading `~/` expands to the home directory. Relative
/// paths are refused so a meeting never writes somewhere unexpected.
pub(crate) fn resolve_directory(configured: &str) -> Option<PathBuf> {
    resolve_directory_with_home(configured, dirs::home_dir())
}

fn resolve_directory_with_home(configured: &str, home: Option<PathBuf>) -> Option<PathBuf> {
    let configured = configured.trim();
    if configured.is_empty() {
        return home.map(|home| {
            DEFAULT_RELATIVE_DIRECTORY
                .iter()
                .fold(home, |path, part| path.join(part))
        });
    }
    if configured == "~" {
        return home;
    }
    if let Some(rest) = configured.strip_prefix("~/") {
        return home.map(|home| home.join(rest));
    }
    let path = PathBuf::from(configured);
    path.is_absolute().then_some(path)
}

/// Replace characters that are unsafe in file names on macOS and common sync
/// targets, collapse whitespace, and bound the length.
pub(crate) fn sanitize_title(title: &str) -> String {
    let replaced: String = title
        .chars()
        .map(|character| match character {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => ' ',
            character if character.is_control() => ' ',
            character => character,
        })
        .collect();
    let collapsed = replaced.split_whitespace().collect::<Vec<_>>().join(" ");
    let bounded: String = collapsed.chars().take(MAX_TITLE_CHARS).collect();
    let trimmed =
        bounded.trim_matches(|character: char| character == '.' || character.is_whitespace());
    if trimmed.is_empty() {
        UNTITLED.to_string()
    } else {
        trimmed.to_string()
    }
}

fn local_time<Tz: TimeZone>(ms: u64, zone: &Tz) -> Option<DateTime<Tz>> {
    zone.timestamp_millis_opt(i64::try_from(ms).ok()?).single()
}

/// `YYYY-MM-DD HHmm <title>` without an extension.
pub(crate) fn file_stem<Tz: TimeZone>(started_at_ms: u64, title: Option<&str>, zone: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let stamp = local_time(started_at_ms, zone)
        .map(|time| time.format("%Y-%m-%d %H%M").to_string())
        .unwrap_or_else(|| "0000-00-00 0000".to_string());
    format!("{stamp} {}", sanitize_title(title.unwrap_or(UNTITLED)))
}

/// JSON string literals are valid YAML double-quoted scalars.
fn yaml_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}

fn yaml_optional(value: Option<&str>) -> String {
    value.map(yaml_string).unwrap_or_else(|| "null".to_string())
}

fn yaml_list(key: &str, values: &[String]) -> String {
    if values.is_empty() {
        return format!("{key}: []\n");
    }
    let mut output = format!("{key}:\n");
    for value in values {
        output.push_str(&format!("  - {}\n", yaml_string(value)));
    }
    output
}

fn title_source(source: Option<MeetingTitleSource>) -> &'static str {
    match source {
        Some(MeetingTitleSource::Manual) => "manual",
        Some(MeetingTitleSource::Calendar) => "calendar",
        Some(MeetingTitleSource::Generated) => "generated",
        None => "null",
    }
}

/// Calendar details for the front matter: a calendar-sourced session title
/// wins; otherwise an overlapping event found at export time.
fn calendar_details<'a>(
    session: &'a MeetingSession,
    calendar: Option<&'a CalendarMatch>,
) -> Option<(&'a str, &'a [String])> {
    if session.title_source == Some(MeetingTitleSource::Calendar) {
        if let Some(title) = session.title.as_deref() {
            return Some((title, session.attendees.as_slice()));
        }
    }
    calendar.map(|event| (event.title.as_str(), event.attendees.as_slice()))
}

/// Title used for the file name: the session title, then a matching calendar
/// event, then `Untitled`.
pub(crate) fn effective_title<'a>(
    session: &'a MeetingSession,
    calendar: Option<&'a CalendarMatch>,
) -> Option<&'a str> {
    session
        .title
        .as_deref()
        .or_else(|| calendar_details(session, calendar).map(|(title, _)| title))
}

pub(crate) fn render_front_matter<Tz: TimeZone>(
    session: &MeetingSession,
    calendar: Option<&CalendarMatch>,
    zone: &Tz,
) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let iso =
        |ms: u64| local_time(ms, zone).map(|time| time.to_rfc3339_opts(SecondsFormat::Secs, false));
    let started_at = iso(session.started_at_ms);
    let ended_at = session.ended_at_ms.and_then(iso);
    let duration_seconds = session
        .ended_at_ms
        .map(|end| end.saturating_sub(session.started_at_ms) / 1_000)
        .unwrap_or(session.duration_ms / 1_000);
    let details = calendar_details(session, calendar);
    let mut output = String::from("---\n");
    output.push_str(&format!(
        "murmur_session_id: {}\n",
        yaml_string(&session.id)
    ));
    output.push_str(&format!(
        "started_at: {}\n",
        yaml_optional(started_at.as_deref())
    ));
    output.push_str(&format!(
        "ended_at: {}\n",
        yaml_optional(ended_at.as_deref())
    ));
    output.push_str(&format!("duration_seconds: {duration_seconds}\n"));
    output.push_str(&format!(
        "title: {}\n",
        yaml_optional(session.title.as_deref())
    ));
    output.push_str(&format!(
        "title_source: {}\n",
        title_source(session.title_source)
    ));
    output.push_str(&yaml_list("attendees", &session.attendees));
    output.push_str(&format!(
        "calendar_event_title: {}\n",
        yaml_optional(details.map(|(title, _)| title))
    ));
    output.push_str(&yaml_list(
        "calendar_event_attendees",
        details.map(|(_, attendees)| attendees).unwrap_or(&[]),
    ));
    output.push_str("---\n\n");
    output
}

fn marker_path(repository: &MeetingRepository, session_id: &str) -> Option<PathBuf> {
    uuid::Uuid::parse_str(session_id).ok()?;
    Some(repository.root().join(MARKER_DIRECTORY).join(session_id))
}

/// Claim the session's one export. The marker lives in the meeting store so a
/// watcher that moves files out of the folder cannot cause a second write.
fn claim(marker: &Path) -> Result<bool, &'static str> {
    let parent = marker.parent().ok_or("marker_unavailable")?;
    std::fs::create_dir_all(parent).map_err(|_| "marker_unavailable")?;
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(marker)
    {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => Ok(false),
        Err(_) => Err("marker_unavailable"),
    }
}

/// Choose a free `<stem>.md`, appending ` (2)`, ` (3)`, … on collisions. An
/// existing file carrying this session's id means it was already exported.
fn destination(
    directory: &Path,
    stem: &str,
    id_line: &str,
) -> Result<Option<PathBuf>, &'static str> {
    for attempt in 1..=MAX_NAME_ATTEMPTS {
        let name = if attempt == 1 {
            format!("{stem}.md")
        } else {
            format!("{stem} ({attempt}).md")
        };
        let path = directory.join(name);
        if !path.exists() {
            return Ok(Some(path));
        }
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        if existing.lines().take(4).any(|line| line == id_line) {
            return Ok(None);
        }
    }
    Err("name_unavailable")
}

/// Render and publish one completed session. Returns without writing when the
/// session is not complete or has already been exported.
pub(crate) fn export_session<Tz: TimeZone>(
    repository: &MeetingRepository,
    session_id: &str,
    directory: &Path,
    calendar: Option<&CalendarMatch>,
    zone: &Tz,
) -> Result<ExportOutcome, &'static str>
where
    Tz::Offset: std::fmt::Display,
{
    let workspace = repository
        .workspace(session_id)
        .map_err(|_| "session_unavailable")?;
    if workspace.session.status != MeetingSessionStatus::Complete {
        return Ok(ExportOutcome::NotComplete);
    }
    let marker = marker_path(repository, session_id).ok_or("invalid_session")?;
    if !claim(&marker)? {
        return Ok(ExportOutcome::AlreadyExported);
    }
    let result = (|| {
        let body = render_export(&workspace, MeetingReviewExportFormat::Markdown)
            .map_err(|_| "render_failed")?;
        let front_matter = render_front_matter(&workspace.session, calendar, zone);
        std::fs::create_dir_all(directory).map_err(|_| "directory_unavailable")?;
        let stem = file_stem(
            workspace.session.started_at_ms,
            effective_title(&workspace.session, calendar),
            zone,
        );
        let id_line = format!("murmur_session_id: {}", yaml_string(session_id));
        let Some(path) = destination(directory, &stem, &id_line)? else {
            return Ok(ExportOutcome::AlreadyExported);
        };
        crate::commands::export::write_text_export(&path, &format!("{front_matter}{body}"))
            .map_err(|_| "write_failed")?;
        Ok(ExportOutcome::Written(path))
    })();
    if result.is_err() {
        // Release the claim so a later attempt is not refused.
        let _ = std::fs::remove_file(&marker);
    }
    result
}

/// Best-effort, local-only calendar lookup for an untitled completed meeting.
/// Uses existing Calendar access only; it never prompts.
fn overlapping_event(repository: &MeetingRepository, session_id: &str) -> Option<CalendarMatch> {
    let session = repository.get_session(session_id).ok()?;
    if session.title.is_some()
        || crate::calendar::permission_status()
            != crate::calendar::CalendarPermissionStatus::Granted
    {
        return None;
    }
    let window = crate::calendar::CalendarWindow::for_session(&session).ok()?;
    let candidates = tauri::async_runtime::block_on(crate::calendar::query_events(
        session_id.to_string(),
        window,
    ))
    .ok()?;
    let overlap = |start: u64, end: u64| {
        end.min(window.end_ms)
            .saturating_sub(start.max(window.start_ms))
    };
    candidates
        .into_iter()
        .max_by_key(|candidate| overlap(candidate.start_ms, candidate.end_ms))
        .map(|candidate| CalendarMatch {
            title: candidate.title,
            attendees: candidate.attendees,
        })
}

/// Run the export on a dedicated thread. Failures are logged without paths,
/// titles, session ids, or transcript content.
pub(crate) fn spawn(
    repository: MeetingRepository,
    session_id: String,
    directory: PathBuf,
    generation: u64,
) {
    let spawned = std::thread::Builder::new()
        .name("murmur-meeting-export".into())
        .spawn(move || {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let calendar = overlapping_event(&repository, &session_id);
                export_session(
                    &repository,
                    &session_id,
                    &directory,
                    calendar.as_ref(),
                    &chrono::Local,
                )
            }))
            .unwrap_or(Err("export_panicked"));
            match outcome {
                Ok(ExportOutcome::Written(_)) => tracing::info!(
                    target: "meeting",
                    event_code = "meeting.auto_export_written",
                    generation,
                    "meeting auto-export written"
                ),
                Ok(ExportOutcome::AlreadyExported) => tracing::info!(
                    target: "meeting",
                    event_code = "meeting.auto_export_skipped",
                    generation,
                    error_code = "already_exported",
                    "meeting auto-export skipped"
                ),
                Ok(ExportOutcome::NotComplete) => tracing::info!(
                    target: "meeting",
                    event_code = "meeting.auto_export_skipped",
                    generation,
                    error_code = "not_complete",
                    "meeting auto-export skipped"
                ),
                Err(error_code) => tracing::warn!(
                    target: "meeting",
                    event_code = "meeting.auto_export_failed",
                    generation,
                    error_code,
                    "meeting auto-export failed"
                ),
            }
        });
    if spawned.is_err() {
        tracing::warn!(
            target: "meeting",
            event_code = "meeting.auto_export_failed",
            generation,
            error_code = "thread_unavailable",
            "meeting auto-export failed"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;
    use tempfile::TempDir;

    const SESSION_ID: &str = "3f8a2b1c-0d4e-4f6a-9b7c-1e2d3c4b5a69";
    // 2026-09-30T18:05:00Z
    const STARTED_AT_MS: u64 = 1_790_791_500_000;

    fn eastern() -> FixedOffset {
        FixedOffset::west_opt(4 * 3600).unwrap()
    }

    fn session() -> MeetingSession {
        MeetingSession {
            id: SESSION_ID.into(),
            title: None,
            title_source: None,
            attendees: vec![],
            started_at_ms: STARTED_AT_MS,
            ended_at_ms: Some(STARTED_AT_MS + 1_834_500),
            status: MeetingSessionStatus::Complete,
            model_name: "test".into(),
            language: "en".into(),
            smart_punctuation: true,
            retain_audio: false,
            duration_ms: 0,
            segment_count: 0,
            preview: String::new(),
            error_code: None,
        }
    }

    fn repository() -> (TempDir, MeetingRepository) {
        let temp = TempDir::new().unwrap();
        let (repository, _) =
            MeetingRepository::initialize(temp.path().canonicalize().unwrap()).unwrap();
        (temp, repository)
    }

    fn created(repository: &MeetingRepository, status: MeetingSessionStatus) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        repository
            .create_session(&id, "test", "en", true, false)
            .unwrap();
        repository.finish_session(&id, status, None).unwrap();
        id
    }

    fn markdown_files(directory: &Path) -> Vec<String> {
        let mut names = std::fs::read_dir(directory)
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    #[test]
    fn sanitizes_unsafe_titles_for_file_names() {
        assert_eq!(sanitize_title("Weekly sync"), "Weekly sync");
        assert_eq!(
            sanitize_title("Q3/Q4: plan?  \"final\""),
            "Q3 Q4 plan final"
        );
        assert_eq!(sanitize_title("a\\b*c<d>e|f\ng\th"), "a b c d e f g h");
        assert_eq!(sanitize_title("..hidden."), "hidden");
        assert_eq!(sanitize_title("   "), "Untitled");
        assert_eq!(sanitize_title("///"), "Untitled");
        assert_eq!(sanitize_title("Café ☕ sync"), "Café ☕ sync");
        let long = "x".repeat(200);
        assert_eq!(sanitize_title(&long).chars().count(), MAX_TITLE_CHARS);
    }

    #[test]
    fn file_stem_uses_local_start_time_and_title() {
        assert_eq!(
            file_stem(STARTED_AT_MS, Some("Design: review"), &eastern()),
            "2026-09-30 1405 Design review"
        );
        assert_eq!(
            file_stem(STARTED_AT_MS, None, &eastern()),
            "2026-09-30 1405 Untitled"
        );
    }

    #[test]
    fn resolves_default_home_relative_and_absolute_directories() {
        let home = Some(PathBuf::from("/Users/test"));
        assert_eq!(
            resolve_directory_with_home("", home.clone()),
            Some(PathBuf::from("/Users/test/Meetings/inbox"))
        );
        assert_eq!(
            resolve_directory_with_home("~/Notes", home.clone()),
            Some(PathBuf::from("/Users/test/Notes"))
        );
        assert_eq!(
            resolve_directory_with_home("/tmp/inbox", home.clone()),
            Some(PathBuf::from("/tmp/inbox"))
        );
        assert_eq!(resolve_directory_with_home("relative/dir", home), None);
        assert_eq!(resolve_directory_with_home("", None), None);
    }

    #[test]
    fn renders_front_matter_for_an_untitled_meeting() {
        assert_eq!(
            render_front_matter(&session(), None, &eastern()),
            "---\n\
             murmur_session_id: \"3f8a2b1c-0d4e-4f6a-9b7c-1e2d3c4b5a69\"\n\
             started_at: \"2026-09-30T14:05:00-04:00\"\n\
             ended_at: \"2026-09-30T14:35:34-04:00\"\n\
             duration_seconds: 1834\n\
             title: null\n\
             title_source: null\n\
             attendees: []\n\
             calendar_event_title: null\n\
             calendar_event_attendees: []\n\
             ---\n\n"
        );
    }

    #[test]
    fn front_matter_quotes_values_and_reports_calendar_details() {
        let mut titled = session();
        titled.title = Some("Plan: \"Q4\" \\ review".into());
        titled.title_source = Some(MeetingTitleSource::Calendar);
        titled.attendees = vec!["Alex".into(), "Taylor: PM".into()];
        let rendered = render_front_matter(&titled, None, &eastern());
        assert!(rendered.contains("title: \"Plan: \\\"Q4\\\" \\\\ review\"\n"));
        assert!(rendered.contains("title_source: calendar\n"));
        assert!(rendered.contains("attendees:\n  - \"Alex\"\n  - \"Taylor: PM\"\n"));
        assert!(rendered.contains("calendar_event_title: \"Plan: \\\"Q4\\\" \\\\ review\"\n"));
        assert!(rendered.contains("calendar_event_attendees:\n  - \"Alex\"\n  - \"Taylor: PM\"\n"));

        let event = CalendarMatch {
            title: "Standup".into(),
            attendees: vec!["Sam".into()],
        };
        let untitled = session();
        let rendered = render_front_matter(&untitled, Some(&event), &eastern());
        assert!(rendered.contains("title: null\n"));
        assert!(rendered.contains("attendees: []\n"));
        assert!(rendered.contains("calendar_event_title: \"Standup\"\n"));
        assert!(rendered.contains("calendar_event_attendees:\n  - \"Sam\"\n"));
        assert_eq!(effective_title(&untitled, Some(&event)), Some("Standup"));

        let mut manual = session();
        manual.title = Some("Manual".into());
        manual.title_source = Some(MeetingTitleSource::Manual);
        assert_eq!(effective_title(&manual, Some(&event)), Some("Manual"));
        assert!(
            render_front_matter(&manual, None, &eastern()).contains("calendar_event_title: null\n")
        );
    }

    #[test]
    fn exports_a_completed_session_exactly_once() {
        let (_store, repository) = repository();
        let output = TempDir::new().unwrap();
        let directory = output.path().join("inbox");
        let id = created(&repository, MeetingSessionStatus::Complete);

        let first = export_session(&repository, &id, &directory, None, &eastern()).unwrap();
        let ExportOutcome::Written(path) = first else {
            panic!("expected a written export, got {first:?}");
        };
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.starts_with(&format!("---\nmurmur_session_id: \"{id}\"\n")));
        assert!(contents.contains("---\n\n# Meeting review\n"));
        assert!(path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with(" Untitled.md"));

        assert_eq!(
            export_session(&repository, &id, &directory, None, &eastern()).unwrap(),
            ExportOutcome::AlreadyExported
        );
        // A watcher moving the file away must not cause a second write.
        std::fs::remove_file(&path).unwrap();
        assert_eq!(
            export_session(&repository, &id, &directory, None, &eastern()).unwrap(),
            ExportOutcome::AlreadyExported
        );
        assert!(markdown_files(&directory).is_empty());
    }

    #[test]
    fn skips_sessions_that_did_not_complete() {
        let (_store, repository) = repository();
        let output = TempDir::new().unwrap();
        for status in [
            MeetingSessionStatus::Failed,
            MeetingSessionStatus::Interrupted,
        ] {
            let id = created(&repository, status);
            assert_eq!(
                export_session(&repository, &id, output.path(), None, &eastern()).unwrap(),
                ExportOutcome::NotComplete
            );
        }
        assert!(markdown_files(output.path()).is_empty());
    }

    #[test]
    fn name_collisions_from_other_files_get_a_suffix() {
        let (_store, repository) = repository();
        let output = TempDir::new().unwrap();
        let id = created(&repository, MeetingSessionStatus::Complete);
        let started_at_ms = repository.get_session(&id).unwrap().started_at_ms;
        let stem = file_stem(started_at_ms, None, &eastern());
        let occupied = output.path().join(format!("{stem}.md"));
        std::fs::write(&occupied, "unrelated").unwrap();
        let ExportOutcome::Written(suffixed) =
            export_session(&repository, &id, output.path(), None, &eastern()).unwrap()
        else {
            panic!("expected a written export");
        };
        assert_eq!(
            suffixed.file_name().unwrap().to_string_lossy(),
            format!("{stem} (2).md")
        );
        assert_eq!(std::fs::read_to_string(&occupied).unwrap(), "unrelated");
        assert_eq!(markdown_files(output.path()).len(), 2);
    }

    #[test]
    fn a_failed_write_releases_the_claim() {
        let (_store, repository) = repository();
        let output = TempDir::new().unwrap();
        let blocker = output.path().join("not-a-directory");
        std::fs::write(&blocker, "file").unwrap();
        let id = created(&repository, MeetingSessionStatus::Complete);
        assert_eq!(
            export_session(&repository, &id, &blocker, None, &eastern()),
            Err("directory_unavailable")
        );
        let directory = output.path().join("inbox");
        assert!(matches!(
            export_session(&repository, &id, &directory, None, &eastern()),
            Ok(ExportOutcome::Written(_))
        ));
    }
}
