//! Calendar access, naming and meeting-suggestion adapters over the shared
//! Calendar commands. The Meetings, Settings and overlay hosts share these so a
//! first-use Calendar prompt, its failure explanations and the suggestion
//! opt-in behave the same on every entry point. Messages mirror shipped Murmur.
use super::{backend::Backend, os::Live};
use crate::{
    meetings::model::{Calendar, Candidate},
    settings::model::Problem,
};
use serde_json::{Value, json};
use std::{future::Future, time::Duration};

const UNSUPPORTED: &str =
    "Calendar lookup is unavailable on this Mac. You can name this meeting manually.";
const NOT_ALLOWED: &str =
    "Calendar access is not allowed. Enable it in System Settings, or name this meeting manually.";
const UNREADABLE: &str =
    "Calendar events could not be read. Try again, or name this meeting manually.";
const CHANGED: &str =
    "The event may have changed. Look it up again, or name this meeting manually.";
const NAMED: &str = "Meeting named from calendar.";
const RESET_DONE: &str =
    "Calendar access reset. Choose Name from calendar to request access again.";
const RESET_FAILED: &str =
    "Calendar access could not be reset. Check System Settings, or keep naming meetings manually.";
const SETTINGS_FAILED: &str = "Calendar settings could not be opened. Open System Settings, then Privacy & Security, then Calendars.";
pub(super) const SUGGESTIONS_ACCESS: &str = "Meeting suggestions need Calendar access. Enable Murmur under Privacy & Security → Calendars, or keep naming meetings manually.";
pub(super) const SUGGESTIONS_CHECK: &str = "Calendar access could not be checked. Keep naming meetings manually, or review Calendar access in System Settings.";
pub(super) const SUGGESTIONS_CONFIG: &str =
    "Meeting suggestions could not be configured. The previous setting was restored.";
pub(super) const ACCEPT_FAILED: &str =
    "Notetaker could not start from that Calendar suggestion. Start it manually to continue.";

/// The shared service answers a first-use request as soon as the native prompt
/// is on screen (onboarding must not wait on it), so `notDetermined` can still
/// be returned while the user decides. Poll the live status until it settles.
const PROMPT_POLL: Duration = Duration::from_millis(250);
const PROMPT_POLLS: u32 = 2_400;

fn status(value: Value) -> Result<String, String> {
    value
        .as_str()
        .map(String::from)
        .ok_or_else(|| "Calendar permission unavailable".into())
}

/// Resolve a request answer that may still be pending. Stops early when the
/// presentation that asked is gone, and gives up after `polls` reads.
pub(super) async fn settle<F, Fut>(
    first: String,
    mut read: F,
    live: &Live,
    interval: Duration,
    polls: u32,
) -> Result<String, String>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<String, String>>,
{
    let mut current = first;
    let mut remaining = polls;
    while current == "notDetermined" && remaining > 0 {
        if !live() {
            return Err("Presentation changed".into());
        }
        tokio::time::sleep(interval).await;
        current = read().await?;
        remaining -= 1;
    }
    Ok(current)
}

/// Check Calendar access, request it only from `notDetermined`, and wait for
/// the user's answer so a grant continues the caller's operation.
pub(super) async fn calendar_access(b: &Backend, live: &Live) -> Result<String, String> {
    let current = status(
        b.effect("get_calendar_permission_status", json!({}), live.clone())
            .await?,
    )?;
    if current != "notDetermined" {
        return Ok(current);
    }
    let requested = status(
        b.effect("request_calendar_permission", json!({}), live.clone())
            .await?,
    )?;
    settle(
        requested,
        move || async move {
            status(
                b.effect("get_calendar_permission_status", json!({}), live.clone())
                    .await?,
            )
        },
        live,
        PROMPT_POLL,
        PROMPT_POLLS,
    )
    .await
}

pub(super) fn unavailable(permission: Option<&str>) -> Calendar {
    let (message, denied, unsupported) = match permission {
        Some("unsupported") => (UNSUPPORTED, false, true),
        Some("denied") => (NOT_ALLOWED, true, false),
        Some(_) => (NOT_ALLOWED, false, false),
        None => (UNREADABLE, false, false),
    };
    Calendar::Unavailable {
        message,
        denied,
        unsupported,
    }
}

fn text(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or_default().into()
}

pub(super) fn choices(events: &Value) -> Vec<Candidate> {
    events
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| Candidate {
            token: text(c, "selectionToken"),
            title: text(c, "title"),
            attendees: c["attendees"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect(),
            start_ms: c["startMs"].as_u64().unwrap_or(0) as i64,
            end_ms: c["endMs"].as_u64().unwrap_or(0) as i64,
        })
        .collect()
}

async fn candidates(b: &Backend, id: &str, live: &Live) -> Result<Calendar, String> {
    let access = calendar_access(b, live).await?;
    if access != "granted" {
        return Ok(unavailable(Some(&access)));
    }
    let events = b
        .effect(
            "get_meeting_calendar_events",
            json!({"sessionId":id}),
            live.clone(),
        )
        .await?;
    Ok(Calendar::Choosing(choices(&events)))
}

/// Every outcome is a visible state: choices, an access explanation, or a
/// retryable read failure. Manual naming stays available in each.
pub(super) async fn lookup_calendar(b: &Backend, id: &str, live: &Live) -> Calendar {
    candidates(b, id, live)
        .await
        .unwrap_or_else(|_| unavailable(None))
}

/// The picker closes with a confirmation when the event applied; a refusal
/// (stale token, changed event, native failure) explains itself.
pub(super) fn apply_outcome(applied: bool) -> (Calendar, Option<String>) {
    if applied {
        (Calendar::Closed, Some(NAMED.into()))
    } else {
        (
            Calendar::Unavailable {
                message: CHANGED,
                denied: false,
                unsupported: false,
            },
            None,
        )
    }
}

/// A reset closes the picker with guidance to ask again; a failed reset keeps
/// the denied explanation with its own message. Neither opens manual naming.
pub(super) fn reset_outcome(reset: bool) -> (Calendar, Option<String>) {
    if reset {
        (Calendar::Closed, Some(RESET_DONE.into()))
    } else {
        (
            Calendar::Unavailable {
                message: RESET_FAILED,
                denied: true,
                unsupported: false,
            },
            None,
        )
    }
}

/// Opening System Settings only reports a failure, with where to go by hand.
pub(super) fn settings_notice(opened: bool) -> Option<String> {
    (!opened).then(|| SETTINGS_FAILED.into())
}

/// Same request `Action::StartSuggested` builds in `meetings.rs`; only the
/// accepted opaque token carries the suggestion.
pub(super) fn suggested_start_request(settings: &Value, token: &str) -> Value {
    json!({"deviceName":settings["microphone"].as_str().filter(|s|!s.is_empty()&&*s!="system_default"),"smartAuto":if settings["smartAutoMicrophoneEnabled"]==true{json!({"approvedDeviceIds":settings["smartAutoApprovedDeviceIds"],"preferredDeviceIds":settings["smartAutoPreferredDeviceIds"],"allowContinuity":settings["smartAutoAllowContinuity"],"requireRecentSignal":settings["smartAutoProbeEnabled"]})}else{Value::Null},"retainAudio":settings["meetingRetainAudio"],"retentionDays":settings["meetingRetentionDays"].as_u64().filter(|n|*n>0),"maxSessions":settings["meetingMaxSessions"],"echoCancellation":settings["meetingEchoCancellationEnabled"],"diarization":settings["meetingDiarization"],"suggestionToken":token})
}

/// Admission samples the frontmost app, so this must run before Murmur
/// activates its own window. Shipped Murmur likewise starts in the background
/// and only surfaces the window to explain a failure.
pub(super) async fn accept_suggestion(b: &Backend, token: &str) -> Result<(), String> {
    if token.is_empty() || token.len() > 4096 {
        return Err("Invalid suggestion".into());
    }
    let settings = super::projections::settings(b).await?;
    b.call(
        "start_meeting",
        json!({"request":suggested_start_request(&settings,token)}),
    )
    .await
    .map(|_| ())
}

async fn configure(b: &Backend, enabled: bool) -> Result<(), String> {
    b.call("configure_meeting_suggestions", json!({"enabled":enabled}))
        .await
        .map(|_| ())
        .map_err(|_| SUGGESTIONS_CONFIG.into())
}

/// Turning suggestions on asks for Calendar access first and saves only after
/// the observer is configured; any failure leaves the previous setting.
pub(super) async fn enable_suggestions(b: &Backend, live: &Live) -> Result<(), String> {
    let access = calendar_access(b, live)
        .await
        .map_err(|_| String::from(SUGGESTIONS_CHECK))?;
    if access != "granted" {
        return Err(SUGGESTIONS_ACCESS.into());
    }
    let (owner, undo) = (b.clone(), b.clone());
    // The patch merges one key, so the current revision cannot clobber edits
    // made while the native prompt was open.
    b.services
        .accept_settings_effect(
            json!({"meetingSuggestionsEnabled":true}),
            b.services.settings_revision(),
            false,
            async move { configure(&owner, true).await },
            async move {
                let _ = configure(&undo, false).await;
            },
        )?
        .await
        .map_err(|_| String::from("Settings owner unavailable"))??;
    Ok(())
}

pub(super) fn disable_suggestions(
    b: &Backend,
    revision: u64,
) -> Result<tokio::sync::oneshot::Receiver<Result<Value, String>>, String> {
    let (owner, undo) = (b.clone(), b.clone());
    b.services.accept_settings_effect(
        json!({"meetingSuggestionsEnabled":false}),
        revision,
        false,
        async move { configure(&owner, false).await },
        async move {
            let _ = configure(&undo, true).await;
        },
    )
}

/// Only these static explanations cross the presentation boundary.
pub(super) fn settings_problem(error: &str) -> Problem {
    [SUGGESTIONS_ACCESS, SUGGESTIONS_CHECK, SUGGESTIONS_CONFIG]
        .into_iter()
        .find(|known| *known == error)
        .map_or(Problem::Unavailable, Problem::Message)
}

/// A persisted opt-in the service refuses (for example after Calendar access
/// was revoked) is restored to off, never left saved and silently inert.
pub(super) fn restore_refused_opt_in(enabled: bool, configured: &Result<Value, String>) -> bool {
    enabled && configured.is_err()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicU32, AtomicUsize, Ordering},
    };

    fn run<T>(work: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
            .block_on(work)
    }
    fn alive() -> Live {
        Arc::new(|| true)
    }
    fn answers(
        sequence: &'static [&'static str],
    ) -> (
        impl FnMut() -> std::future::Ready<Result<String, String>>,
        Arc<AtomicUsize>,
    ) {
        let reads = Arc::new(AtomicUsize::new(0));
        let seen = reads.clone();
        (
            move || {
                let index = seen.fetch_add(1, Ordering::SeqCst);
                std::future::ready(
                    sequence
                        .get(index)
                        .map(|value| value.to_string())
                        .ok_or_else(|| "exhausted".to_string()),
                )
            },
            reads,
        )
    }

    #[test]
    fn a_pending_prompt_resumes_with_the_users_grant() {
        let (read, reads) = answers(&["notDetermined", "notDetermined", "granted"]);
        let result = run(settle(
            "notDetermined".into(),
            read,
            &alive(),
            Duration::from_millis(1),
            50,
        ));
        assert_eq!(result.as_deref(), Ok("granted"));
        assert_eq!(reads.load(Ordering::SeqCst), 3);
    }
    #[test]
    fn a_pending_prompt_resolves_to_a_denial_without_polling_further() {
        let (read, reads) = answers(&["denied", "granted"]);
        let result = run(settle(
            "notDetermined".into(),
            read,
            &alive(),
            Duration::from_millis(1),
            50,
        ));
        assert_eq!(result.as_deref(), Ok("denied"));
        assert_eq!(reads.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn an_answered_request_never_polls() {
        let (read, reads) = answers(&["denied"]);
        let result = run(settle(
            "granted".into(),
            read,
            &alive(),
            Duration::from_millis(1),
            50,
        ));
        assert_eq!(result.as_deref(), Ok("granted"));
        assert_eq!(reads.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn an_unanswered_prompt_is_bounded_and_stays_not_determined() {
        let calls = Arc::new(AtomicU32::new(0));
        let seen = calls.clone();
        let result = run(settle(
            "notDetermined".into(),
            move || {
                seen.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Ok("notDetermined".to_string()))
            },
            &alive(),
            Duration::from_millis(1),
            4,
        ));
        assert_eq!(result.as_deref(), Ok("notDetermined"));
        assert_eq!(calls.load(Ordering::SeqCst), 4);
    }
    #[test]
    fn a_closed_presentation_stops_waiting_for_the_prompt() {
        let (read, reads) = answers(&["granted"]);
        let gone: Live = Arc::new(|| false);
        let result = run(settle(
            "notDetermined".into(),
            read,
            &gone,
            Duration::from_millis(1),
            50,
        ));
        assert_eq!(result, Err("Presentation changed".into()));
        assert_eq!(reads.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn a_failed_status_read_surfaces_instead_of_looping() {
        let result = run(settle(
            "notDetermined".into(),
            || std::future::ready(Err("Native view is unavailable".to_string())),
            &alive(),
            Duration::from_millis(1),
            50,
        ));
        assert!(result.is_err());
    }

    fn parts(calendar: Calendar) -> (&'static str, bool, bool) {
        match calendar {
            Calendar::Unavailable {
                message,
                denied,
                unsupported,
            } => (message, denied, unsupported),
            _ => panic!("expected the unavailable state"),
        }
    }
    #[test]
    fn each_access_outcome_explains_itself_with_the_shipped_wording() {
        assert_eq!(
            parts(unavailable(Some("unsupported"))),
            (UNSUPPORTED, false, true)
        );
        assert_eq!(
            parts(unavailable(Some("denied"))),
            (NOT_ALLOWED, true, false)
        );
        assert_eq!(
            parts(unavailable(Some("restricted"))),
            (NOT_ALLOWED, false, false)
        );
        assert_eq!(
            parts(unavailable(Some("notDetermined"))),
            (NOT_ALLOWED, false, false)
        );
        assert_eq!(parts(unavailable(None)), (UNREADABLE, false, false));
    }
    #[test]
    fn choices_carry_the_service_events_with_no_invented_fields() {
        let events = json!([
            {"selectionToken":"token-a","title":"Synthetic planning","attendees":["Ada","Grace"],"startMs":1700000000000u64,"endMs":1700000900000u64},
            {"selectionToken":"token-b","title":"Synthetic review","attendees":[],"startMs":1700001000000u64,"endMs":1700001900000u64}
        ]);
        let found = choices(&events);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].token, "token-a");
        assert_eq!(found[0].attendees, ["Ada", "Grace"]);
        assert_eq!(found[0].start_ms, 1700000000000);
        assert_eq!(found[1].title, "Synthetic review");
        assert!(found[1].attendees.is_empty());
        assert!(choices(&json!(null)).is_empty());
    }
    #[test]
    fn applying_closes_the_picker_with_a_confirmation_and_a_refusal_explains() {
        let (calendar, notice) = apply_outcome(true);
        assert!(matches!(calendar, Calendar::Closed));
        assert_eq!(notice.as_deref(), Some("Meeting named from calendar."));
        let (calendar, notice) = apply_outcome(false);
        assert_eq!(parts(calendar), (CHANGED, false, false));
        assert_eq!(notice, None);
    }

    #[test]
    fn a_reset_closes_with_guidance_and_a_failed_reset_keeps_the_denied_state() {
        let (calendar, notice) = reset_outcome(true);
        assert!(matches!(calendar, Calendar::Closed));
        assert_eq!(
            notice.as_deref(),
            Some("Calendar access reset. Choose Name from calendar to request access again.")
        );
        let (calendar, notice) = reset_outcome(false);
        assert_eq!(parts(calendar), (RESET_FAILED, true, false));
        assert_eq!(notice, None);
    }
    #[test]
    fn a_settings_failure_explains_where_to_go_by_hand() {
        assert_eq!(settings_notice(true), None);
        assert_eq!(settings_notice(false).as_deref(), Some(SETTINGS_FAILED));
    }

    #[test]
    fn accepting_a_suggestion_carries_the_token_and_resolved_meeting_settings() {
        let settings = json!({
            "microphone":"system_default","smartAutoMicrophoneEnabled":false,
            "meetingRetainAudio":true,"meetingRetentionDays":0,"meetingMaxSessions":25,
            "meetingEchoCancellationEnabled":true,"meetingDiarization":false
        });
        let request = suggested_start_request(&settings, "synthetic-token");
        assert_eq!(request["suggestionToken"], "synthetic-token");
        assert_eq!(request["deviceName"], Value::Null);
        assert_eq!(request["smartAuto"], Value::Null);
        assert_eq!(request["retentionDays"], Value::Null);
        assert_eq!(request["retainAudio"], true);
        assert_eq!(request["maxSessions"], 25);
    }

    #[test]
    fn suggestion_failures_reach_the_settings_surface_with_their_own_explanation() {
        for known in [SUGGESTIONS_ACCESS, SUGGESTIONS_CHECK, SUGGESTIONS_CONFIG] {
            assert_eq!(settings_problem(known), Problem::Message(known));
        }
        assert_eq!(
            settings_problem("Native path /private"),
            Problem::Unavailable
        );
        assert_eq!(
            settings_problem("Settings changed elsewhere"),
            Problem::Unavailable
        );
    }
    #[test]
    fn a_refused_persisted_opt_in_is_restored_to_off() {
        assert!(restore_refused_opt_in(true, &Err("refused".into())));
        assert!(!restore_refused_opt_in(true, &Ok(Value::Null)));
        assert!(!restore_refused_opt_in(false, &Err("refused".into())));
    }
}
