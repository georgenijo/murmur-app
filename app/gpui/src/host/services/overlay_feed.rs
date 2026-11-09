//! What the production host feeds the overlay beyond lifecycle events: the
//! Smart Auto state read from the service, the result of an overlay delivery
//! retry, and the rule that keeps a slow read from overwriting a newer one.
//! Everything here is independent of the window system, so the same code the
//! host runs is exercised by the tests below.
use super::{
    delivery_feedback,
    qa_faults::{Fault, Faults},
    smart_auto_summary::{self as summary, Projection},
};
use crate::overlay::{
    model::{Delivery, Event},
    smart_auto::SmartAuto,
};
use serde_json::{Value, json};
use std::{future::Future, sync::Mutex};

/// Newest companion hydration requested. Reads finish in any order, but the
/// overlay accepts any snapshot with a higher revision and revisions are
/// assigned at publication, so a slow older read would otherwise land last and
/// win. Only the newest request may publish.
#[derive(Default)]
pub struct Hydrations(Mutex<u64>);

impl Hydrations {
    /// Numbers a request. Call on the thread that decides to rehydrate, before
    /// the reads start, so a later request is always the newer one.
    pub fn begin(&self) -> u64 {
        let mut newest = self.0.lock().unwrap_or_else(|p| p.into_inner());
        *newest += 1;
        *newest
    }
    pub fn is_current(&self, request: u64) -> bool {
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) == request
    }
    /// Run `publish` only if `request` is still the newest. The check and the
    /// publication share one lock, so a newer request cannot start between
    /// them. Returns whether it published.
    pub fn publish(&self, request: u64, publish: impl FnOnce()) -> bool {
        let newest = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if *newest != request {
            return false;
        }
        publish();
        true
    }
}

/// Wait `wait_ms`, then call `expired` unless a newer hydration has already
/// replaced `request`.
pub async fn refresh_after(
    hydrations: &Hydrations,
    request: u64,
    wait_ms: u64,
    expired: impl FnOnce(),
) {
    tokio::time::sleep(std::time::Duration::from_millis(wait_ms)).await;
    if hydrations.is_current(request) {
        expired();
    }
}

/// Production Smart Auto overlay state, or `None` when Smart Auto is off.
///
/// The shipped hook's rules: elapsed read time counts against the service's
/// deadline, and a read that is already spent by the time it returns is read
/// once more, then treated as unavailable. `now_ms` is the overlay clock,
/// `fail_status` consumes a QA fault, and `call` is the production command.
pub async fn read_smart_auto<F, Fut>(
    settings: &Value,
    now_ms: impl Fn() -> u64,
    fail_status: impl Fn() -> bool,
    call: F,
) -> Option<SmartAuto>
where
    F: Fn(&'static str, Value) -> Fut,
    Fut: Future<Output = Result<Value, String>>,
{
    let request = summary::request(settings)?;
    for reread in [false, true] {
        let requested = now_ms();
        let status = if !reread && fail_status() {
            Err("Synthetic Smart Auto status failure".to_owned())
        } else {
            call(
                "get_smart_auto_microphone_status",
                json!({"smartAuto": request}),
            )
            .await
        };
        let status = status.ok();
        let devices = if summary::needs_inventory(status.as_ref()) {
            call("get_audio_input_inventory", json!({})).await.ok()
        } else {
            None
        };
        let completed = now_ms();
        if let Projection::Show(state) = summary::project(
            status.as_ref(),
            devices.as_ref(),
            requested,
            completed,
            reread,
        ) {
            return Some(state);
        }
    }
    Some(SmartAuto::unavailable())
}

/// The overlay's retry command, or its one-shot QA stand-in. A stand-in never
/// runs the real command, so it cannot paste.
pub async fn overlay_retry(
    faults: &Faults,
    real: impl Future<Output = Result<Value, String>>,
) -> Result<Value, String> {
    if faults.take(Fault::OverlayRetryEmpty) {
        return Ok(json!({"kind":"empty","message":"Nothing to paste yet."}));
    }
    if faults.take(Fault::OverlayRetryError) {
        return Err("Synthetic delivery retry failure".to_owned());
    }
    real.await
}

/// What the overlay learns from a retry command result: the outcome and its
/// bounded service message, which becomes the accessible description.
pub fn retry_result_event(generation: u64, attempt: u64, result: &Result<Value, String>) -> Event {
    let value = result.as_ref().ok();
    Event::RetryResult {
        generation,
        attempt,
        result: Delivery::from_retry_kind(value.and_then(|v| v["kind"].as_str())),
        message: value
            .and_then(|v| v["message"].as_str())
            .and_then(delivery_feedback::bounded),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay::{
        inbox::Inbox,
        model::{Indicator, Model, Snapshot},
        smart_auto::{Cue, SmartAutoState},
    };
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };

    fn block_on<T>(work: impl Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
            .block_on(work)
    }
    fn smart_settings() -> Value {
        json!({
            "smartAutoMicrophoneEnabled": true,
            "smartAutoApprovedDeviceIds": ["synthetic-built-in"],
            "smartAutoPreferredDeviceIds": [],
            "smartAutoAllowContinuity": false,
            "smartAutoProbeEnabled": true,
        })
    }
    fn ready(valid_for_ms: u64) -> Value {
        json!({"state":"ready","deviceId":"synthetic-built-in","reason":"preferred_approved","validForMs":valid_for_ms})
    }
    fn service_result(command: &str, status: &Value) -> Result<Value, String> {
        Ok(match command {
            "get_smart_auto_microphone_status" => status.clone(),
            _ => json!({"devices":[{"id":"synthetic-built-in","name":"Synthetic Microphone"}]}),
        })
    }
    fn model() -> Model {
        Model::new(Snapshot {
            generation: 7,
            ..Default::default()
        })
    }

    #[test]
    fn smart_auto_state_expires_at_the_service_deadline_through_the_production_reader() {
        let clock = std::cell::Cell::new(1_000u64);
        let status = ready(30_000);
        let state = block_on(read_smart_auto(
            &smart_settings(),
            || clock.get(),
            || false,
            |command, _| {
                // Each service call takes 200 ms; that time is already spent.
                clock.set(clock.get() + 200);
                let result = service_result(command, &status);
                async move { result }
            },
        ))
        .unwrap();
        assert_eq!(state.until_ms, Some(31_000));
        let mut model = Model::new(Snapshot {
            smart_auto: Some(state),
            ..Default::default()
        });
        assert_eq!(model.smart_auto_cue(1_400), Some(Cue::Ready));
        assert_eq!(model.smart_auto_cue(30_999), Some(Cue::Ready));
        assert_eq!(
            model.smart_auto_cue(31_000),
            None,
            "no host event is needed"
        );
        // Turning Smart Auto off drops the state with the next snapshot.
        model.event(
            Event::Snapshot(Snapshot {
                revision: 1,
                ..Default::default()
            }),
            2,
        );
        assert_eq!(model.smart_auto_cue(2), None);
        assert!(
            block_on(read_smart_auto(
                &json!({"smartAutoMicrophoneEnabled": false}),
                || 0,
                || false,
                |command, _| {
                    let result = service_result(command, &status);
                    async move { result }
                },
            ))
            .is_none()
        );
    }

    #[test]
    fn smart_auto_time_spent_before_the_read_returned_is_unavailable_never_ready() {
        let clock = std::cell::Cell::new(0u64);
        let calls = std::cell::Cell::new(0u32);
        let status = ready(100);
        let state = block_on(read_smart_auto(
            &smart_settings(),
            || clock.get(),
            || false,
            |command, _| {
                calls.set(calls.get() + 1);
                clock.set(clock.get() + 200);
                let result = service_result(command, &status);
                async move { result }
            },
        ))
        .unwrap();
        // Read once, reread once (status and inventory each time), then give up.
        assert_eq!(calls.get(), 4);
        assert_eq!(state, SmartAuto::unavailable());
        let model = Model::new(Snapshot {
            smart_auto: Some(state),
            ..Default::default()
        });
        assert_eq!(model.smart_auto_cue(10_000_000), Some(Cue::Blocked));
    }

    #[test]
    fn a_smart_auto_status_command_failure_is_blocked_once_and_then_recovers() {
        let faults = Faults::arm(Some("smart-auto-status-error"), true);
        let status = ready(30_000);
        let calls = std::cell::Cell::new(0u32);
        let read = || {
            block_on(read_smart_auto(
                &smart_settings(),
                || 0,
                || faults.take(Fault::SmartAutoStatusError),
                |command, _| {
                    calls.set(calls.get() + 1);
                    let result = service_result(command, &status);
                    async move { result }
                },
            ))
            .unwrap()
        };
        assert_eq!(read(), SmartAuto::unavailable());
        assert_eq!(calls.get(), 0, "the synthetic failure replaces the command");
        assert!(matches!(read().state, SmartAutoState::Ready { .. }));
        let off = Faults::arm(Some("smart-auto-status-error"), false);
        assert!(
            !off.take(Fault::SmartAutoStatusError),
            "inert unless permitted"
        );
    }

    // One hydration as the host runs it: read Smart Auto (the service answers
    // after `delay_ms`), then publish only if still the newest request.
    async fn hydrate(
        hydrations: Arc<Hydrations>,
        revision: Arc<AtomicU64>,
        inbox: Inbox,
        request: u64,
        observed_epoch: u64,
        delay_ms: u64,
        status: Value,
    ) -> bool {
        let state = read_smart_auto(
            &smart_settings(),
            || 0,
            || false,
            |command, _| {
                let result = service_result(command, &status);
                async move {
                    tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                    result
                }
            },
        )
        .await;
        hydrations.publish(request, || {
            let revision = revision.fetch_add(1, Ordering::SeqCst) + 1;
            let _ = inbox.publish(Event::Hydrate {
                observed_epoch,
                snapshot: Snapshot {
                    revision,
                    smart_auto: state,
                    ..Default::default()
                },
            });
        })
    }

    #[test]
    fn an_older_smart_auto_read_that_finishes_last_does_not_overwrite_newer_state() {
        let hydrations = Arc::new(Hydrations::default());
        let revision = Arc::new(AtomicU64::new(0));
        let inbox = Inbox::default();
        let blocked = json!({"state":"blocked","message":"Synthetic block","retryAfterMs":null});
        let mut model = Model::new(Snapshot::default());
        let run = |model: &mut Model, first: (Value, u64), second: (Value, u64)| {
            // `first` is the older request, `second` the newer; each finishes
            // after its own delay.
            let observed = model.hydration_epoch();
            let older = hydrations.begin();
            let newer = hydrations.begin();
            let (older_published, newer_published) = block_on(async {
                let a = tokio::spawn(hydrate(
                    hydrations.clone(),
                    revision.clone(),
                    inbox.clone(),
                    older,
                    observed,
                    first.1,
                    first.0,
                ));
                let b = tokio::spawn(hydrate(
                    hydrations.clone(),
                    revision.clone(),
                    inbox.clone(),
                    newer,
                    observed,
                    second.1,
                    second.0,
                ));
                (a.await.unwrap(), b.await.unwrap())
            });
            assert!(
                !older_published && newer_published,
                "only the newest request publishes"
            );
            for event in inbox.drain().0 {
                model.event(event, 0);
            }
        };
        // The older request is slow and finishes after the newer one.
        run(&mut model, (ready(30_000), 60), (blocked.clone(), 0));
        assert_eq!(model.smart_auto_cue(0), Some(Cue::Blocked));
        // The older finishing first is dropped too: the newer will publish.
        run(&mut model, (blocked, 0), (ready(30_000), 60));
        assert_eq!(model.smart_auto_cue(0), Some(Cue::Ready));
    }

    #[test]
    fn the_expiry_refresh_fires_at_the_deadline_unless_a_newer_hydration_took_over() {
        let hydrations = Hydrations::default();
        let fired = std::cell::Cell::new(0u32);
        let request = hydrations.begin();
        block_on(refresh_after(&hydrations, request, 5, || {
            fired.set(fired.get() + 1)
        }));
        assert_eq!(fired.get(), 1, "rereads at the deadline");
        let stale = hydrations.begin();
        let _newer = hydrations.begin();
        block_on(refresh_after(&hydrations, stale, 0, || {
            fired.set(fired.get() + 1)
        }));
        assert_eq!(fired.get(), 1, "a replaced request does nothing");
    }

    #[test]
    fn overlay_retry_messages_reach_the_accessible_description_through_the_production_adapter() {
        let message = "Copied to the clipboard. Paste it manually.";
        let mut model = model();
        model.event(Event::Delivery(7, Delivery::Clipboard), 0);
        assert!(model.retry(1).is_some());
        let ran = std::cell::Cell::new(false);
        let result = block_on(overlay_retry(&Faults::default(), async {
            ran.set(true);
            Ok(json!({"kind":"clipboard_only","message":message}))
        }));
        assert!(ran.get(), "no fault: the real command ran");
        model.event(retry_result_event(7, model.retry_attempt, &result), 2);
        assert_eq!(model.indicator(3), Indicator::Delivery(Delivery::Clipboard));
        assert_eq!(model.announcement(3), message);
        // A result from an older attempt carries nothing into the new one.
        assert!(model.retry(4).is_some());
        model.event(retry_result_event(7, model.retry_attempt - 1, &result), 5);
        assert_eq!(model.indicator(5), Indicator::Delivery(Delivery::Retrying));
        // The message is bounded.
        let long = json!({"kind":"failed","message":"x".repeat(1_000)});
        match retry_result_event(7, 1, &Ok(long)) {
            Event::RetryResult { message, .. } => {
                assert_eq!(message.unwrap().chars().count(), 240)
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn qa_faults_make_empty_and_error_overlay_retries_reachable_once_and_only_when_armed() {
        let real =
            || async { Ok::<Value, String>(json!({"kind":"auto_pasted","message":"Pasted."})) };
        let faults = Faults::arm(Some("overlay-retry-empty,overlay-retry-error"), true);
        let ran = std::cell::Cell::new(false);
        let empty = block_on(overlay_retry(&faults, async {
            ran.set(true);
            real().await
        }));
        assert!(!ran.get(), "the synthetic outcome replaces the command");
        let mut model = model();
        model.event(Event::Delivery(7, Delivery::Clipboard), 0);
        model.retry(1).unwrap();
        model.event(retry_result_event(7, model.retry_attempt, &empty), 2);
        assert_eq!(model.indicator(3), Indicator::Delivery(Delivery::Nothing));
        assert_eq!(model.announcement(3), "Nothing to paste yet.");
        let error = block_on(overlay_retry(&faults, real()));
        assert!(error.is_err());
        match retry_result_event(7, 1, &error) {
            Event::RetryResult {
                result, message, ..
            } => assert!(result == Delivery::Retry && message.is_none()),
            _ => unreachable!(),
        }
        // Consumed: the next retry is the real command again.
        assert_eq!(
            block_on(overlay_retry(&faults, real())).unwrap()["kind"],
            "auto_pasted"
        );
        // A build or launch that did not arm anything never replaces it.
        let unarmed = Faults::arm(Some("overlay-retry-empty"), false);
        assert_eq!(
            block_on(overlay_retry(&unarmed, real())).unwrap()["kind"],
            "auto_pasted"
        );
    }
}
