use crate::calendar::{
    self, CalendarEventCandidate, CalendarOperation, CalendarPermissionStatus, CalendarWindow,
};
use crate::meeting_review::MeetingWorkspace;
use crate::meeting_store::{MeetingRepository, SaveMeetingMetadataRequest};
use crate::State;

fn require_main_window(label: &str) -> Result<(), String> {
    if label == "main" {
        Ok(())
    } else {
        Err("Calendar actions are only available from the main window.".into())
    }
}

fn validate_session_id(value: &str) -> Result<(), String> {
    if !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|value| value.is_ascii_alphanumeric() || value == b'-' || value == b'_')
    {
        Ok(())
    } else {
        Err("The meeting calendar request is invalid.".into())
    }
}

#[tauri::command]
pub fn get_calendar_permission_status(
    window: tauri::WebviewWindow,
) -> Result<CalendarPermissionStatus, String> {
    require_main_window(window.label())?;
    Ok(calendar::permission_status())
}

#[tauri::command]
pub async fn request_calendar_permission(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<CalendarPermissionStatus, String> {
    require_main_window(window.label())?;
    calendar::request_permission(&app).await
}

#[tauri::command]
pub async fn reset_calendar_permission(window: tauri::WebviewWindow) -> Result<(), String> {
    require_main_window(window.label())?;
    let _operation = CalendarOperation::acquire()?;
    let bundle_id = super::permissions::current_bundle_identifier().ok_or(calendar::UNAVAILABLE)?;
    let mut command = tokio::process::Command::new("/usr/bin/tccutil");
    command
        .args(["reset", "Calendar", &bundle_id])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let status = tokio::time::timeout(std::time::Duration::from_secs(10), command.status())
        .await
        .map_err(|_| {
            "Calendar permission reset did not finish. Open System Settings to change access."
                .to_string()
        })?
        .map_err(|_| {
            "Calendar permission could not be reset. Open System Settings to change access."
                .to_string()
        })?;
    if !status.success() {
        return Err(
            "Calendar permission could not be reset. Open System Settings to change access.".into(),
        );
    }
    Ok(())
}

#[tauri::command]
pub fn open_calendar_preferences(window: tauri::WebviewWindow) -> Result<(), String> {
    require_main_window(window.label())?;
    super::permissions::open_system_preference_pane("Privacy_Calendars")
        .map_err(|_| "Open System Settings, then Privacy & Security, then Calendars.".into())
}

#[tauri::command]
pub async fn get_meeting_calendar_events(
    window: tauri::WebviewWindow,
    session_id: String,
    state: tauri::State<'_, State>,
) -> Result<Vec<CalendarEventCandidate>, String> {
    require_main_window(window.label())?;
    validate_session_id(&session_id)?;
    let session = state.meeting_store.repository()?.get_session(&session_id)?;
    let interval = CalendarWindow::for_session(&session)?;
    calendar::query_events(session_id, interval).await
}

#[tauri::command]
pub async fn apply_meeting_calendar_event(
    window: tauri::WebviewWindow,
    session_id: String,
    selection_token: String,
    state: tauri::State<'_, State>,
) -> Result<MeetingWorkspace, String> {
    require_main_window(window.label())?;
    validate_session_id(&session_id)?;
    calendar::validate_selection_token(&selection_token)?;
    let repository = state.meeting_store.repository()?;
    let session = repository.get_session(&session_id)?;
    let interval = CalendarWindow::for_session(&session)?;
    let candidates = calendar::query_events(session_id.clone(), interval).await?;
    save_selection(&repository, session_id, &selection_token, candidates)
}

fn save_selection(
    repository: &MeetingRepository,
    session_id: String,
    selection_token: &str,
    candidates: Vec<CalendarEventCandidate>,
) -> Result<MeetingWorkspace, String> {
    let candidate = candidates
        .into_iter()
        .find(|candidate| candidate.selection_token == selection_token)
        .ok_or(calendar::STALE_SELECTION)?;
    repository.save_calendar_metadata(SaveMeetingMetadataRequest {
        session_id,
        title: Some(candidate.title),
        attendees: candidate.attendees,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meeting_store::MeetingTitleSource;

    #[test]
    fn calendar_commands_reject_other_windows_and_unbounded_identifiers() {
        assert!(require_main_window("main").is_ok());
        for label in ["overlay", "log-viewer", "transform-review", ""] {
            assert!(require_main_window(label).is_err());
        }
        assert!(validate_session_id("valid-session_1").is_ok());
        for value in ["", "../private", "session\n", &"a".repeat(65)] {
            assert!(validate_session_id(value).is_err());
        }
    }

    #[test]
    fn calendar_apply_requires_a_current_selection_and_only_saves_metadata() {
        let root = tempfile::tempdir().unwrap();
        let (repository, _) = MeetingRepository::initialize(root.path().to_path_buf()).unwrap();
        repository
            .create_session("calendar-session", "base.en", "en", true, false)
            .unwrap();
        assert!(
            CalendarWindow::for_session(&repository.get_session("calendar-session").unwrap())
                .is_err()
        );
        repository
            .finish_session(
                "calendar-session",
                crate::meeting_store::MeetingSessionStatus::Complete,
                None,
            )
            .unwrap();
        let initial = repository.workspace("calendar-session").unwrap();
        assert!(CalendarWindow::for_session(&initial.session).is_ok());
        let candidate = CalendarEventCandidate {
            selection_token: "1".repeat(64),
            title: "Confirmed calendar title".into(),
            attendees: vec!["Alex".into()],
            start_ms: 1_000,
            end_ms: 2_000,
        };
        assert!(save_selection(
            &repository,
            "calendar-session".into(),
            &"0".repeat(64),
            vec![candidate.clone()]
        )
        .is_err());
        assert_eq!(
            repository.workspace("calendar-session").unwrap().session,
            initial.session
        );
        assert!(save_selection(
            &repository,
            "calendar-session".into(),
            &candidate.selection_token,
            vec![]
        )
        .is_err());
        let saved = save_selection(
            &repository,
            "calendar-session".into(),
            &candidate.selection_token.clone(),
            vec![candidate],
        )
        .unwrap();
        assert_eq!(
            saved.session.title.as_deref(),
            Some("Confirmed calendar title")
        );
        assert_eq!(
            saved.session.title_source,
            Some(MeetingTitleSource::Calendar)
        );
        assert_eq!(saved.session.attendees, ["Alex"]);
        assert_eq!(saved.segments, initial.segments);
        assert_eq!(
            repository
                .list_sessions(Some("Confirmed calendar"), 0, 10)
                .unwrap()
                .total,
            1
        );
        let manual = repository
            .save_metadata(SaveMeetingMetadataRequest {
                session_id: "calendar-session".into(),
                title: Some("Manually renamed".into()),
                attendees: vec![],
            })
            .unwrap();
        assert_eq!(
            manual.session.title_source,
            Some(MeetingTitleSource::Manual)
        );
        let connection = rusqlite::Connection::open(root.path().join("meetings.sqlite3")).unwrap();
        let columns = connection
            .prepare("PRAGMA table_info(meeting_sessions)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(!columns.iter().any(|column| column.contains("calendar")
            || column.contains("token")
            || column.contains("event")));
    }
}
