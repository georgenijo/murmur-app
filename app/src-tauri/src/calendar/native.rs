use super::*;
use crate::MutexExt;
use block2::RcBlock;
use objc2::exception::catch;
use objc2::msg_send;
use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::Bool;
use objc2_event_kit::{EKEntityType, EKEvent, EKEventStatus, EKEventStore};
use objc2_foundation::{NSDate, NSError, NSString};
use std::panic::AssertUnwindSafe;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const QUERY_TIMEOUT: Duration = Duration::from_secs(15);

thread_local! {
    static SUGGESTION_OBSERVER_STORE: std::cell::RefCell<Option<Retained<EKEventStore>>> = const { std::cell::RefCell::new(None) };
}

pub(crate) async fn set_suggestion_observation(
    app: &tauri::AppHandle,
    enabled: bool,
) -> Result<(), String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let result = catch(AssertUnwindSafe(|| {
            SUGGESTION_OBSERVER_STORE.with(|slot| {
                let mut slot = slot.borrow_mut();
                if enabled {
                    permission_status().require_read_access()?;
                    if slot.is_none() {
                        *slot = Some(unsafe { EKEventStore::new() });
                    }
                } else {
                    *slot = None;
                }
                Ok(())
            })
        }))
        .map_err(|_| UNAVAILABLE.to_string())
        .and_then(|result| result);
        let _ = sender.send(result);
    })
    .map_err(|_| UNAVAILABLE.to_string())?;
    receiver.await.map_err(|_| UNAVAILABLE.to_string())?
}

pub(crate) fn permission_status() -> CalendarPermissionStatus {
    catch(|| unsafe {
        CalendarPermissionStatus::from_native(
            EKEventStore::authorizationStatusForEntityType(EKEntityType::Event).0,
        )
    })
    .unwrap_or(CalendarPermissionStatus::Unsupported)
}

pub(crate) async fn request_permission(
    app: &tauri::AppHandle,
) -> Result<CalendarPermissionStatus, String> {
    let status = permission_status();
    if status != CalendarPermissionStatus::NotDetermined {
        return Ok(status);
    }
    let operation = CalendarOperation::acquire()?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let _ = catch(AssertUnwindSafe(|| unsafe {
            let store = EKEventStore::new();
            // EventKit may complete on any queue. The completion owns the store
            // and admission guard until it has delivered its single result.
            let pending = Mutex::new(Some((store.clone(), operation, sender)));
            let completion = RcBlock::new(move |_granted: Bool, error: *mut NSError| {
                if let Some((_store, _operation, sender)) = pending.lock_or_recover().take() {
                    let result = if error.is_null() {
                        Ok(permission_status())
                    } else {
                        Err(UNAVAILABLE.to_string())
                    };
                    let _ = sender.send(result);
                }
            });
            store.requestFullAccessToEventsWithCompletion(RcBlock::as_ptr(&completion));
        }));
    })
    .map_err(|_| UNAVAILABLE.to_string())?;
    receiver.await.map_err(|_| UNAVAILABLE.to_string())?
}

pub(crate) async fn query_events(
    session_id: String,
    window: CalendarWindow,
) -> Result<Vec<CalendarEventCandidate>, String> {
    permission_status().require_read_access()?;
    let operation = CalendarOperation::acquire()?;
    let task = tauri::async_runtime::spawn_blocking(move || {
        // A timed-out native query keeps ownership until its worker returns.
        // Repeated UI requests therefore cannot accumulate blocked workers.
        let _operation = operation;
        let events = read_events(window, false, None)?;
        permission_status().require_read_access()?;
        candidates(&session_id, window, events)
    });
    tokio::time::timeout(QUERY_TIMEOUT, task)
        .await
        .map_err(|_| UNAVAILABLE.to_string())?
        .map_err(|_| UNAVAILABLE.to_string())?
}

pub(crate) async fn query_suggestion_events(
    window: CalendarWindow,
    permit: Arc<AtomicBool>,
) -> Result<Vec<SuggestionEvent>, String> {
    let operation = CalendarOperation::acquire()?;
    let task = tauri::async_runtime::spawn_blocking(move || {
        let _operation = operation;
        if !permit.load(Ordering::Acquire) {
            return Err(UNAVAILABLE.into());
        }
        let events = read_events(window, true, Some(permit.clone()))?;
        if !permit.load(Ordering::Acquire) {
            return Err(UNAVAILABLE.into());
        }
        permission_status().require_read_access()?;
        suggestion_events(window, events)
    });
    tokio::time::timeout(QUERY_TIMEOUT, task)
        .await
        .map_err(|_| UNAVAILABLE.to_string())?
        .map_err(|_| UNAVAILABLE.to_string())?
}

fn read_events(
    window: CalendarWindow,
    only_video: bool,
    permit: Option<Arc<AtomicBool>>,
) -> Result<Vec<CalendarEvent>, String> {
    if permit
        .as_ref()
        .is_some_and(|permit| !permit.load(Ordering::Acquire))
    {
        return Err(UNAVAILABLE.into());
    }
    permission_status().require_read_access()?;
    autoreleasepool(|_| {
        catch(AssertUnwindSafe(|| unsafe {
            if permit
                .as_ref()
                .is_some_and(|permit| !permit.load(Ordering::Acquire))
            {
                return Err(UNAVAILABLE.into());
            }
            let store = EKEventStore::new();
            let start = NSDate::dateWithTimeIntervalSince1970(window.start_ms as f64 / 1_000.0);
            let end = NSDate::dateWithTimeIntervalSince1970(window.end_ms as f64 / 1_000.0);
            let predicate =
                store.predicateForEventsWithStartDate_endDate_calendars(&start, &end, None);
            let output = Arc::new(Mutex::new(Ok(Vec::new())));
            let result = output.clone();
            let started = Instant::now();
            let callback = RcBlock::new(move |event: NonNull<EKEvent>, mut stop: NonNull<Bool>| {
                let mut result = result.lock_or_recover();
                if started.elapsed() >= QUERY_TIMEOUT
                    || permit
                        .as_ref()
                        .is_some_and(|permit| !permit.load(Ordering::Acquire))
                {
                    *result = Err(UNAVAILABLE.to_string());
                } else if let Ok(events) = result.as_mut() {
                    if events.len() >= MAX_CALENDAR_EVENTS {
                        *result = Err(INVALID_EVENT.to_string());
                    } else {
                        let converted = catch(AssertUnwindSafe(|| {
                            if only_video && !event_has_video_link(event.as_ref()) {
                                return Ok(None);
                            }
                            extract_event(event.as_ref(), window)
                        }))
                        .map_err(|_| UNAVAILABLE.to_string())
                        .and_then(|event| event);
                        match converted {
                            Ok(Some(event)) => events.push(event),
                            Ok(None) => {}
                            Err(error) => *result = Err(error),
                        }
                    }
                }
                if result.is_err() {
                    *stop.as_mut() = Bool::YES;
                }
            });
            store.enumerateEventsMatchingPredicate_usingBlock(
                &predicate,
                RcBlock::as_ptr(&callback),
            );
            drop(callback);
            let mut output = output.lock_or_recover();
            std::mem::replace(&mut *output, Ok(Vec::new()))
        }))
        .map_err(|_| UNAVAILABLE.to_string())?
    })
}

unsafe fn event_has_video_link(event: &EKEvent) -> bool {
    if event.isAllDay() {
        return false;
    }
    if let Some(url) = event.URL().and_then(|url| url.absoluteString()) {
        if url.length() <= 2_048 && contains_video_link(&url.to_string()) {
            return true;
        }
    }
    for value in [event.location(), event.notes()].into_iter().flatten() {
        if value.length() <= 16_384 {
            let text = value.to_string();
            if text.len() <= 16_384 && contains_video_link(&text) {
                return true;
            }
        }
    }
    false
}

fn date_ms(date: &NSDate) -> Result<u64, String> {
    let milliseconds = date.timeIntervalSince1970() * 1_000.0;
    if !milliseconds.is_finite() || milliseconds < 0.0 || milliseconds > MAX_TIMESTAMP_MS as f64 {
        return Err(INVALID_EVENT.to_string());
    }
    Ok(milliseconds.round() as u64)
}

fn text(value: &NSString, maximum: usize) -> Result<String, String> {
    if value.length() > maximum * 2 {
        return Err(INVALID_EVENT.to_string());
    }
    bounded_text(value.to_string(), maximum)
}

unsafe fn extract_event(
    event: &EKEvent,
    window: CalendarWindow,
) -> Result<Option<CalendarEvent>, String> {
    if event.status() == EKEventStatus::Canceled {
        return Ok(None);
    }
    // Apple's older null-unspecified declarations allow nil. Receive optional
    // retained values instead of trusting the generated nonnull getter types.
    let start: Option<Retained<NSDate>> = msg_send![event, startDate];
    let end: Option<Retained<NSDate>> = msg_send![event, endDate];
    let (Some(start), Some(end)) = (start, end) else {
        return Err(INVALID_EVENT.into());
    };
    let start_ms = date_ms(&start)?;
    let end_ms = date_ms(&end)?;
    if !window.overlaps(start_ms, end_ms) {
        return Ok(None);
    }
    let identifier = event.eventIdentifier().ok_or(INVALID_EVENT)?;
    let identifier = text(&identifier, MAX_EVENT_IDENTIFIER_CHARS)?;
    let title: Option<Retained<NSString>> = msg_send![event, title];
    let title = match title {
        Some(title) if title.length() > 0 => text(&title, MAX_MEETING_TITLE_CHARS)?,
        _ => "Untitled calendar event".into(),
    };
    let occurrence_ms = event
        .occurrenceDate()
        .as_deref()
        .map(date_ms)
        .transpose()?
        .unwrap_or(start_ms);
    let mut attendees = Vec::new();
    if let Some(participants) = event.attendees() {
        if participants.count() > MAX_MEETING_ATTENDEES {
            return Err(INVALID_EVENT.into());
        }
        for participant in &participants {
            if let Some(name) = participant.name() {
                if name.length() > 0 {
                    attendees.push(text(&name, MAX_MEETING_ATTENDEE_CHARS)?);
                }
            }
        }
    }
    Ok(Some(CalendarEvent {
        identifier,
        occurrence_ms,
        title,
        attendees,
        start_ms,
        end_ms,
    }))
}
