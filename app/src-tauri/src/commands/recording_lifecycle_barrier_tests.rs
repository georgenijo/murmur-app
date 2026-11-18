//! Stage 2a-1: the stale-attempt handling of the two D-less lifecycle arms that
//! run a mutex-taking operation **before** they read the current recording id
//! (site 212 `Accepted`: the telemetry and diagnostics claims; site 213
//! `InitializationFailed`: the production-capture-failure record).
//!
//! Those operations take mutexes, so the window between "the event arrived" and
//! "the current id is read" is not sub-microsecond: another start can allocate the
//! next generation inside it. Each test advances the generation from a barrier
//! point *inside* the operation (`crate::barrier_hook`, one line at the top of
//! each callee) and asserts what today's code does: the id is read **after** the
//! operation, so the late generation is the one the stale rule sees.
//!
//! These are oracle tests: they were written against the pre-refactor bridge and
//! must pass there unchanged.
#![cfg(test)]

use super::tests::test_state;
use super::*;
use std::sync::Mutex as StdMutex;
use tauri::Listener;

const RID: u64 = 7;
const NEWER: u64 = 8;

type Terminal = Option<(DictationTerminalOutcome, DictationErrorCode)>;

#[derive(Debug, PartialEq)]
struct Seen {
    events: Vec<String>,
    terminal: Terminal,
    /// The context installed for `RID` is still installed.
    context_kept: bool,
}

/// Handle `event` for `RID` (current, `Starting`, context installed). When
/// `advance_at` names a barrier point, the generation moves to `NEWER` the moment
/// that operation begins.
fn handle(event: AudioLifecycleEvent, advance_at: Option<&'static str>) -> Seen {
    let app_state = AppState::default();
    app_state.recording_id.store(RID, Ordering::SeqCst);
    app_state.dictation.lock_or_recover().status = DictationStatus::Starting;
    app_state.dictation_telemetry.accepted(RID);
    let app = tauri::test::mock_builder()
        .manage(test_state(app_state))
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("build mock Tauri app");
    let handle = app.handle().clone();
    let state = handle.state::<State>();
    let destination = select_recording_destination(
        &crate::frontmost::DeliveryTargetSnapshot::Incomplete,
        crate::smart_destination::Snapshot::Disabled,
    );
    state.app_state.set_active_context(
        RID,
        resolve_recording_context(&state.app_state, destination),
    );
    let events = Arc::new(StdMutex::new(Vec::new()));
    for name in [
        "recording-initialization-failed",
        "recording-status-changed",
    ] {
        let events = Arc::clone(&events);
        handle.listen_any(name, move |_| events.lock().unwrap().push(name.to_string()));
    }
    let advancer = handle.clone();
    let _barrier = crate::barrier_hook::install(move |point| {
        if advance_at == Some(point) {
            advancer
                .state::<State>()
                .app_state
                .recording_id
                .store(NEWER, Ordering::SeqCst);
        }
    });
    handle_audio_lifecycle_and_ready_tasks(handle.clone(), RID, event, |_, _| {}, |_, _, _| {});
    let seen = Seen {
        events: events.lock().unwrap().clone(),
        terminal: state.app_state.dictation_telemetry.terminal_for_test(RID),
        context_kept: state.app_state.active_context(RID).is_some(),
    };
    seen
}

fn superseded() -> Terminal {
    Some((
        DictationTerminalOutcome::Superseded,
        DictationErrorCode::StaleOwner,
    ))
}

fn init_failed() -> AudioLifecycleEvent {
    AudioLifecycleEvent::InitializationFailed {
        error: "typed failure".to_string(),
        kind: audio::AudioFailureKind::StreamInvalidated,
        recovery_reason: None,
    }
}

#[test]
fn initialization_failed_reads_the_current_id_after_the_capture_failure_record() {
    // No barrier: R213.2, the failure is the current recording's.
    let steady = handle(init_failed(), None);
    assert_eq!(steady.events, ["recording-initialization-failed"]);
    assert_eq!(
        steady.terminal,
        Some((
            DictationTerminalOutcome::CaptureInitFailure,
            DictationErrorCode::StreamInvalidated
        ))
    );
    assert!(!steady.context_kept);

    // The generation advances while the record is being made (a timed-out start
    // restored Idle and another start allocated N+1): the id read afterwards is
    // N+1, so this is R213.1, the superseded terminal only.
    let raced = handle(
        init_failed(),
        Some("performance.production_capture_failure"),
    );
    assert_eq!(raced.events, Vec::<String>::new());
    assert_eq!(raced.terminal, superseded());
    assert!(raced.context_kept, "a stale failure clears nothing");
}

#[test]
fn accepted_reads_the_current_id_after_the_telemetry_claim() {
    assert_eq!(handle(AudioLifecycleEvent::Accepted, None).terminal, None);
    let raced = handle(AudioLifecycleEvent::Accepted, Some("telemetry.accepted"));
    assert_eq!(raced.terminal, superseded());
    assert_eq!(raced.events, Vec::<String>::new());
}

#[test]
fn accepted_reads_the_current_id_after_the_diagnostics_claim() {
    let raced = handle(AudioLifecycleEvent::Accepted, Some("diagnostics.claim"));
    assert_eq!(raced.terminal, superseded());
    assert_eq!(raced.events, Vec::<String>::new());
}
