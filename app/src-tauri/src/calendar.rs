use crate::meeting_store::{
    MeetingSession, MeetingSessionStatus, MAX_MEETING_ATTENDEES, MAX_MEETING_ATTENDEE_CHARS,
    MAX_MEETING_TITLE_CHARS,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};

mod native;

pub(crate) use native::{
    permission_status, query_events, query_suggestion_events, request_permission,
    set_suggestion_observation,
};

pub(crate) const MAX_CALENDAR_EVENTS: usize = 100;
const MAX_LOOKUP_WINDOW_MS: u64 = 7 * 24 * 60 * 60 * 1_000;
const MAX_TIMESTAMP_MS: u64 = 253_402_300_799_999;
const MAX_EVENT_IDENTIFIER_CHARS: usize = 2_048;
pub(crate) const UNAVAILABLE: &str =
    "Calendar lookup is unavailable. You can name this meeting manually.";
pub(crate) const ACCESS_REQUIRED: &str =
    "Allow Calendar access in System Settings, or name this meeting manually.";
const INVALID_EVENT: &str =
    "Calendar details exceed the supported limits. You can name this meeting manually.";
pub(crate) const STALE_SELECTION: &str =
    "This calendar event changed or is no longer available. Choose it again before applying.";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CalendarPermissionStatus {
    NotDetermined,
    Granted,
    Denied,
    Restricted,
    Unsupported,
}

impl CalendarPermissionStatus {
    fn from_native(value: isize) -> Self {
        match value {
            0 => Self::NotDetermined,
            1 => Self::Restricted,
            2 | 4 => Self::Denied,
            3 => Self::Granted,
            _ => Self::Unsupported,
        }
    }

    fn require_read_access(self) -> Result<(), String> {
        if self == Self::Granted {
            Ok(())
        } else {
            Err(ACCESS_REQUIRED.into())
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct CalendarWindow {
    pub start_ms: u64,
    pub end_ms: u64,
}

impl CalendarWindow {
    pub(crate) fn for_session(session: &MeetingSession) -> Result<Self, String> {
        if session.status == MeetingSessionStatus::Active {
            return Err("Stop this meeting before naming it from calendar.".into());
        }
        let mut end_ms = session.ended_at_ms.ok_or(UNAVAILABLE)?;
        if end_ms == session.started_at_ms {
            end_ms = end_ms.checked_add(1).ok_or(UNAVAILABLE)?;
        }
        Self::new(session.started_at_ms, end_ms)
    }

    pub(crate) fn new(start_ms: u64, end_ms: u64) -> Result<Self, String> {
        if start_ms >= end_ms
            || end_ms > MAX_TIMESTAMP_MS
            || end_ms - start_ms > MAX_LOOKUP_WINDOW_MS
        {
            return Err("Calendar lookup supports completed meeting windows up to seven days. You can name this meeting manually.".into());
        }
        Ok(Self { start_ms, end_ms })
    }

    fn overlaps(self, start_ms: u64, end_ms: u64) -> bool {
        start_ms < self.end_ms && end_ms > self.start_ms && start_ms < end_ms
    }
}

// Private calendar payloads deliberately have no Debug implementation.
pub(crate) struct CalendarEvent {
    identifier: String,
    occurrence_ms: u64,
    title: String,
    attendees: Vec<String>,
    start_ms: u64,
    end_ms: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarEventCandidate {
    pub selection_token: String,
    pub title: String,
    pub attendees: Vec<String>,
    pub start_ms: u64,
    pub end_ms: u64,
}

#[derive(Clone)]
pub(crate) struct SuggestionEvent {
    pub occurrence_key: String,
    pub title: String,
    pub attendees: Vec<String>,
    pub start_ms: u64,
    pub end_ms: u64,
}

fn suggestion_events(
    window: CalendarWindow,
    events: Vec<CalendarEvent>,
) -> Result<Vec<SuggestionEvent>, String> {
    events
        .into_iter()
        .map(|event| {
            let mut digest = Sha256::new();
            digest.update(event.identifier.as_bytes());
            digest.update(event.occurrence_ms.to_le_bytes());
            let occurrence_key = format!("{:x}", digest.finalize());
            let candidate = candidates("suggestion", window, vec![event])?
                .pop()
                .ok_or(UNAVAILABLE)?;
            Ok(SuggestionEvent {
                occurrence_key,
                title: candidate.title,
                attendees: candidate.attendees,
                start_ms: candidate.start_ms,
                end_ms: candidate.end_ms,
            })
        })
        .collect()
}

fn contains_video_link(text: &str) -> bool {
    text.split(|character: char| {
        character.is_whitespace()
            || matches!(character, '<' | '>' | '"' | '\'' | '(' | ')' | '[' | ']')
    })
    .any(|part| {
        let Ok(url) = reqwest::Url::parse(part.trim_end_matches(['.', ',', ';'])) else {
            return false;
        };
        if !matches!(url.scheme(), "https" | "http")
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return false;
        }
        let Some(host) = url.host_str() else {
            return false;
        };
        [
            "zoom.us",
            "teams.microsoft.com",
            "teams.live.com",
            "meet.google.com",
            "webex.com",
            "facetime.apple.com",
        ]
        .iter()
        .any(|provider| host == *provider || host.ends_with(&format!(".{provider}")))
            || ((host == "slack.com" || host.ends_with(".slack.com"))
                && url.path().split('/').any(|part| part == "huddle"))
    })
}

fn bounded_text(value: String, maximum: usize) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > maximum || value.chars().any(char::is_control) {
        return Err(INVALID_EVENT.into());
    }
    Ok(value.to_string())
}

pub(crate) fn candidates(
    session_id: &str,
    window: CalendarWindow,
    events: Vec<CalendarEvent>,
) -> Result<Vec<CalendarEventCandidate>, String> {
    if events.len() > MAX_CALENDAR_EVENTS {
        return Err(INVALID_EVENT.into());
    }
    let mut candidates = Vec::with_capacity(events.len());
    for event in events {
        if !window.overlaps(event.start_ms, event.end_ms) {
            continue;
        }
        if event.end_ms > MAX_TIMESTAMP_MS
            || event.occurrence_ms > MAX_TIMESTAMP_MS
            || event.attendees.len() > MAX_MEETING_ATTENDEES
        {
            return Err(INVALID_EVENT.into());
        }
        let identifier = bounded_text(event.identifier, MAX_EVENT_IDENTIFIER_CHARS)?;
        let title = bounded_text(event.title, MAX_MEETING_TITLE_CHARS)?;
        let mut attendees = event
            .attendees
            .into_iter()
            .map(|value| bounded_text(value, MAX_MEETING_ATTENDEE_CHARS))
            .collect::<Result<Vec<_>, _>>()?;
        attendees.sort();
        attendees.dedup();
        let mut digest = Sha256::new();
        for part in [session_id, &identifier, &title] {
            digest.update((part.len() as u64).to_le_bytes());
            digest.update(part.as_bytes());
        }
        for value in [
            window.start_ms,
            window.end_ms,
            event.start_ms,
            event.end_ms,
            event.occurrence_ms,
        ] {
            digest.update(value.to_le_bytes());
        }
        for attendee in &attendees {
            digest.update((attendee.len() as u64).to_le_bytes());
            digest.update(attendee.as_bytes());
        }
        candidates.push(CalendarEventCandidate {
            selection_token: format!("{:x}", digest.finalize()),
            title,
            attendees,
            start_ms: event.start_ms,
            end_ms: event.end_ms,
        });
    }
    candidates.sort_by(|a, b| {
        a.start_ms
            .cmp(&b.start_ms)
            .then_with(|| a.title.cmp(&b.title))
            .then_with(|| a.selection_token.cmp(&b.selection_token))
    });
    candidates.dedup_by(|a, b| a.selection_token == b.selection_token);
    Ok(candidates)
}

pub(crate) fn validate_selection_token(value: &str) -> Result<(), String> {
    if value.len() == 64
        && value
            .bytes()
            .all(|value| value.is_ascii_digit() || (b'a'..=b'f').contains(&value))
    {
        Ok(())
    } else {
        Err(STALE_SELECTION.into())
    }
}

static OPERATION_ACTIVE: AtomicBool = AtomicBool::new(false);
pub(crate) static OPERATION_FINISHED: tokio::sync::Notify = tokio::sync::Notify::const_new();
pub(crate) const OPERATION_BUSY: &str =
    "A Calendar request is already in progress. Try again when it finishes.";

pub(crate) fn operation_in_progress() -> bool {
    OPERATION_ACTIVE.load(Ordering::Acquire)
}

pub(crate) struct CalendarOperation;

impl CalendarOperation {
    pub(crate) fn acquire() -> Result<Self, String> {
        OPERATION_ACTIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| Self)
            .map_err(|_| OPERATION_BUSY.into())
    }
}

impl Drop for CalendarOperation {
    fn drop(&mut self) {
        OPERATION_ACTIVE.store(false, Ordering::Release);
        OPERATION_FINISHED.notify_waiters();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event() -> CalendarEvent {
        CalendarEvent {
            identifier: "synthetic-calendar-event".into(),
            occurrence_ms: 1_000,
            title: "Project meeting".into(),
            attendees: vec!["Taylor".into(), "Alex".into()],
            start_ms: 1_000,
            end_ms: 3_000,
        }
    }

    #[test]
    fn calendar_permission_fails_closed_except_full_access() {
        for (value, expected) in [
            (0, CalendarPermissionStatus::NotDetermined),
            (1, CalendarPermissionStatus::Restricted),
            (2, CalendarPermissionStatus::Denied),
            (3, CalendarPermissionStatus::Granted),
            (4, CalendarPermissionStatus::Denied),
            (99, CalendarPermissionStatus::Unsupported),
        ] {
            let status = CalendarPermissionStatus::from_native(value);
            assert_eq!(status, expected);
            assert_eq!(status.require_read_access().is_ok(), value == 3);
        }
        assert_eq!(
            serde_json::to_string(&CalendarPermissionStatus::NotDetermined).unwrap(),
            "\"notDetermined\""
        );
    }

    #[test]
    fn calendar_video_links_require_parsed_provider_hosts() {
        for link in [
            "https://meet.google.com/abc-defg-hij",
            "Join: https://example.zoom.us/j/123.",
            "https://teams.microsoft.com/l/meetup-join/123",
            "https://app.slack.com/huddle/T1/C1",
            "https://facetime.apple.com/join#value",
        ] {
            assert!(contains_video_link(link), "{link}");
        }
        for link in [
            "https://zoom.us.evil.invalid/j/123",
            "https://zoom.us@evil.invalid/j/123",
            "https://evil.invalid/?next=https://zoom.us",
            "https://slack.com/help",
            "file://meet.google.com/call",
            "Meet in room 4",
        ] {
            assert!(!contains_video_link(link), "{link}");
        }
    }

    #[test]
    fn calendar_occurrence_keys_survive_title_changes_but_distinguish_recurrence() {
        let window = CalendarWindow::new(1_000, 10_000).unwrap();
        let first = suggestion_events(window, vec![event()]).unwrap().remove(0);
        let mut renamed = event();
        renamed.title = "Changed title".into();
        let renamed = suggestion_events(window, vec![renamed]).unwrap().remove(0);
        assert_eq!(first.occurrence_key, renamed.occurrence_key);
        let mut next = event();
        next.occurrence_ms += 5_000;
        let next = suggestion_events(window, vec![next]).unwrap().remove(0);
        assert_ne!(first.occurrence_key, next.occurrence_key);
    }

    #[test]
    fn calendar_operation_admission_refuses_queued_work_and_releases_on_drop() {
        let operation = CalendarOperation::acquire().unwrap();
        for _ in 0..100 {
            assert!(CalendarOperation::acquire().is_err());
        }
        drop(operation);
        assert!(CalendarOperation::acquire().is_ok());
    }

    #[test]
    fn calendar_overlap_is_strict_and_includes_enclosing_events() {
        let window = CalendarWindow::new(1_500, 2_500).unwrap();
        assert!(window.overlaps(1_000, 3_000));
        assert!(window.overlaps(2_000, 2_100));
        assert!(!window.overlaps(0, 1_500));
        assert!(!window.overlaps(2_500, 3_000));
        assert!(!window.overlaps(2_000, 2_000));
        assert!(CalendarWindow::new(10, 9).is_err());
        assert!(CalendarWindow::new(0, MAX_LOOKUP_WINDOW_MS + 1).is_err());
        assert!(CalendarWindow::new(MAX_TIMESTAMP_MS, MAX_TIMESTAMP_MS + 1).is_err());
    }

    #[test]
    fn calendar_candidates_are_bounded_and_errors_are_content_free() {
        let window = CalendarWindow::new(1_000, 3_000).unwrap();
        for field in ["title", "attendee", "count", "identifier"] {
            let mut input = event();
            match field {
                "title" => input.title = "SECRET".repeat(100),
                "attendee" => input.attendees = vec!["SECRET\nNAME".into()],
                "count" => input.attendees = vec!["SECRET".into(); MAX_MEETING_ATTENDEES + 1],
                _ => input.identifier = "SECRET".repeat(1_000),
            }
            let error = candidates("session", window, vec![input]).err().unwrap();
            assert!(!error.contains("SECRET"));
        }
        let too_many = (0..=MAX_CALENDAR_EVENTS).map(|_| event()).collect();
        assert!(candidates("session", window, too_many).is_err());
    }

    #[test]
    fn calendar_selection_binds_session_occurrence_and_displayed_metadata() {
        let window = CalendarWindow::new(1_000, 3_000).unwrap();
        let original = candidates("session", window, vec![event()])
            .unwrap()
            .remove(0);
        assert_eq!(original.attendees, ["Alex", "Taylor"]);
        assert!(validate_selection_token(&original.selection_token).is_ok());
        assert!(validate_selection_token("SECRET_EVENT_ID").is_err());
        for variant in 0..5 {
            let mut input = event();
            let mut session_id = "session";
            match variant {
                0 => input.title = "Renamed meeting".into(),
                1 => input.attendees.push("Jordan".into()),
                2 => input.occurrence_ms += 1,
                3 => input.end_ms += 1,
                _ => session_id = "different-session",
            }
            let changed = candidates(session_id, window, vec![input])
                .unwrap()
                .remove(0);
            assert_ne!(original.selection_token, changed.selection_token);
        }
        let mut second = event();
        second.identifier = "second-event".into();
        assert_eq!(
            candidates("session", window, vec![event(), second])
                .unwrap()
                .len(),
            2
        );
    }
}
