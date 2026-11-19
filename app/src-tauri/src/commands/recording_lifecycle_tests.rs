//! Stage 2a-1: the lifecycle bridge (sites 207-216) against appendix C3.6, driven
//! on a mock Tauri app.
//!
//! The expected values are written from the C3.6 rows, **not** derived from the
//! decide functions: each case says what the user-visible shell must do (the
//! emitted events in order, the final status, the terminal the telemetry ledger
//! recorded, the keyboard flag, whether the context was cleared, which spawns
//! ran), so it fails if the executor wiring, a port payload or a placement drifts
//! from today's inline code. The golden traces and PD witnesses cover the
//! interleavings; this covers every (event, status, id) cell of the table.
#![cfg(test)]

use super::tests::test_state;
use super::*;
use std::cell::{Cell, RefCell};
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Mutex as StdMutex;
use std::task::{Context, Poll, Waker};
use tauri::Listener;

const RID: u64 = 7;
const NEWER: u64 = 8;

const EVENTS: [&str; 8] = [
    "recording-status-changed",
    "audio-initialization-stalled",
    "audio-recovery-started",
    "recording-cancelled",
    "recording-initialization-failed",
    "recording-cleanup-finished",
    "recording-recovery-stalled",
    "recording-interrupted",
];

type Terminal = Option<(DictationTerminalOutcome, DictationErrorCode)>;

#[derive(Debug, PartialEq)]
pub(super) struct Outcome {
    /// Event names in emission order; `recording-status-changed` carries its payload.
    events: Vec<String>,
    status: DictationStatus,
    terminal: Terminal,
    /// `keyboard::set_processing(false)` ran (it was set before the event).
    keyboard_cleared: bool,
    /// The installed context for `RID` was cleared.
    context_cleared: bool,
    /// `last_transcription_at` was stamped.
    stamped: bool,
    /// The self-dispatched auto-stop (`RS`) was scheduled.
    auto_stop: bool,
    /// The ready side tasks (`SD`) were attempted.
    ready_tasks: bool,
}

fn run(
    event: AudioLifecycleEvent,
    status: DictationStatus,
    current: u64,
    context: bool,
) -> Outcome {
    run_with_core(event, status, current, context, CoreSeed::Live)
}

/// What the persisted core holds when the event arrives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CoreSeed {
    /// Equal to the live `status` and recording id: how every effect leaves it.
    Live,
    /// Different from live in both M1 and M2, as a write that bypassed the adapter (or a
    /// bug in a commit) would leave it: the C8.6 assertion must count it and re-sync it.
    Diverged,
}

/// [`run`] with the persisted core seeded as `seed` says. The core is seeded **before**
/// the event, from the fixture's live fields, so an entry that reads it reads exactly what
/// the fixture planted (`recording_lifecycle_cutover_tests`).
pub(super) fn run_with_core(
    event: AudioLifecycleEvent,
    status: DictationStatus,
    current: u64,
    context: bool,
    seed: CoreSeed,
) -> Outcome {
    let app_state = AppState::default();
    app_state.recording_id.store(current, Ordering::SeqCst);
    {
        let mut dictation = app_state.dictation.lock_or_recover();
        dictation.status = status;
        // The fixture writes the live fields directly, so the persisted core is
        // seeded to match (the one way to make a core equal to live without an effect).
        crate::dictation_executor::FieldAdapter::seed_core(&app_state, &mut dictation);
        if seed == CoreSeed::Diverged {
            // Move both persisted fields off live through the core's own commits.
            let other = if status == DictationStatus::Recording {
                murmur_dictation_core::status::Status::Processing
            } else {
                murmur_dictation_core::status::Status::Recording
            };
            dictation.core.set_status(other);
            dictation.core.alloc_id();
        }
    }
    app_state.dictation_telemetry.accepted(RID);
    let app = tauri::test::mock_builder()
        .manage(test_state(app_state))
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("build mock Tauri app");
    let handle = app.handle().clone();
    let state = handle.state::<State>();
    if context {
        let destination = select_recording_destination(
            &crate::frontmost::DeliveryTargetSnapshot::Incomplete,
            crate::smart_destination::Snapshot::Disabled,
        );
        state.app_state.set_active_context(
            RID,
            resolve_recording_context(&state.app_state, destination),
        );
    }
    let events = Arc::new(StdMutex::new(Vec::new()));
    for name in EVENTS {
        let events = Arc::clone(&events);
        handle.listen_any(name, move |event| {
            let entry = if name == "recording-status-changed" {
                format!("{name} {}", event.payload())
            } else {
                name.to_string()
            };
            events.lock().unwrap().push(entry);
        });
    }
    keyboard::set_processing(true);
    let auto_stop = Cell::new(false);
    let ready_tasks = Cell::new(false);
    handle_audio_lifecycle_and_ready_tasks(
        handle.clone(),
        RID,
        event,
        |_, id| {
            assert_eq!(id, RID);
            auto_stop.set(true);
        },
        |_, id, _register| {
            assert_eq!(id, RID);
            ready_tasks.set(true);
        },
    );
    {
        // The C8.6 invariant after the entry, whatever the core held when it arrived: the
        // persisted M1/M2 equal the live status and recording id (a divergence was
        // re-synced before the decision, and every effect commits both).
        let dictation = state.app_state.dictation.lock_or_recover();
        assert_eq!(
            (dictation.core.status(), dictation.core.current_id()),
            (
                crate::dictation_admission::core_status(dictation.status),
                murmur_dictation_core::ids::RecordingId(
                    state.app_state.recording_id.load(Ordering::SeqCst)
                ),
            ),
            "the persisted core equals live after the entry ({seed:?})"
        );
    }
    let outcome = Outcome {
        events: events.lock().unwrap().clone(),
        status: state.app_state.dictation.lock_or_recover().status,
        terminal: state.app_state.dictation_telemetry.terminal_for_test(RID),
        keyboard_cleared: !keyboard::is_processing(),
        context_cleared: context && state.app_state.active_context(RID).is_none(),
        stamped: state
            .app_state
            .last_transcription_at
            .lock_or_recover()
            .is_some(),
        auto_stop: auto_stop.get(),
        ready_tasks: ready_tasks.get(),
    };
    keyboard::set_processing(false);
    outcome
}

use DictationStatus::{Idle, Processing, Recording, Recovering, Starting};

const ALL: [DictationStatus; 5] = [Idle, Starting, Recording, Recovering, Processing];

fn superseded() -> Terminal {
    Some((
        DictationTerminalOutcome::Superseded,
        DictationErrorCode::StaleOwner,
    ))
}

/// What a row that changes nothing looks like.
fn untouched(status: DictationStatus, terminal: Terminal) -> Outcome {
    Outcome {
        events: Vec::new(),
        status,
        terminal,
        keyboard_cleared: false,
        context_cleared: false,
        stamped: false,
        auto_stop: false,
        ready_tasks: false,
    }
}

#[test]
fn site_207_ready() {
    // R207.1: a non-current recording, any status, is superseded under D.
    for status in ALL {
        for context in [false, true] {
            let got = run(AudioLifecycleEvent::Ready, status, NEWER, context);
            assert_eq!(
                got,
                Outcome {
                    ready_tasks: true,
                    ..untouched(status, superseded())
                },
                "R207.1 {status:?} context={context}"
            );
        }
    }
    // R207.2 / R207.3: current but not Starting.
    for status in [Idle, Recovering, Processing, Recording] {
        let got = run(AudioLifecycleEvent::Ready, status, RID, true);
        assert_eq!(
            got,
            Outcome {
                ready_tasks: true,
                ..untouched(status, superseded())
            },
            "R207.2/.3 {status:?}"
        );
    }
    // R207.4: Starting without a context waits, quietly.
    assert_eq!(
        run(AudioLifecycleEvent::Ready, Starting, RID, false),
        Outcome {
            ready_tasks: true,
            ..untouched(Starting, None)
        }
    );
    // R207.5: Starting with a context publishes Recording.
    assert_eq!(
        run(AudioLifecycleEvent::Ready, Starting, RID, true),
        Outcome {
            events: vec![r#"recording-status-changed "recording""#.into()],
            stamped: true,
            ready_tasks: true,
            ..untouched(Recording, None)
        }
    );
}

#[test]
fn site_208_still_connecting() {
    for status in ALL {
        let current = run(AudioLifecycleEvent::StillConnecting, status, RID, false);
        let events = if status == Starting {
            vec!["audio-initialization-stalled".to_string()]
        } else {
            Vec::new()
        };
        assert_eq!(
            current,
            Outcome {
                events,
                ..untouched(status, None)
            },
            "R208.1/.2 {status:?}"
        );
        let stale = run(AudioLifecycleEvent::StillConnecting, status, NEWER, false);
        assert_eq!(stale, untouched(status, None), "R208.2 stale {status:?}");
    }
}

fn recovering(reason: AudioCancelReason, status: DictationStatus) -> Outcome {
    run(
        AudioLifecycleEvent::Recovering { reason },
        status,
        RID,
        true,
    )
}

#[test]
fn site_209_recovering() {
    // R209.1: a non-current recording is superseded and changes nothing else.
    for status in ALL {
        let got = run(
            AudioLifecycleEvent::Recovering {
                reason: AudioCancelReason::User,
            },
            status,
            NEWER,
            true,
        );
        assert_eq!(got, untouched(status, superseded()), "R209.1 {status:?}");
    }
    // R209.2 (PD-5): current, but not Starting or Recording.
    for status in [Idle, Recovering, Processing] {
        for reason in [
            AudioCancelReason::User,
            AudioCancelReason::HardDeadline,
            AudioCancelReason::RuntimeFailure,
        ] {
            assert_eq!(
                recovering(reason, status),
                untouched(status, superseded()),
                "R209.2 {status:?} {reason:?}"
            );
        }
    }
    let status_recovering = r#"recording-status-changed "recovering""#.to_string();
    let base = |events: &[&str], terminal: Terminal, context_cleared: bool| Outcome {
        events: std::iter::once(status_recovering.clone())
            .chain(events.iter().map(|event| event.to_string()))
            .collect(),
        status: Recovering,
        terminal,
        context_cleared,
        ..untouched(Recovering, None)
    };
    for (previous, outcome, code) in [
        (
            Starting,
            DictationTerminalOutcome::UserCancelledStarting,
            DictationErrorCode::CancelledStarting,
        ),
        (
            Recording,
            DictationTerminalOutcome::UserCancelledRecording,
            DictationErrorCode::CancelledRecording,
        ),
    ] {
        // R209.3 / R209.4.
        assert_eq!(
            recovering(AudioCancelReason::User, previous),
            base(
                &["audio-recovery-started", "recording-cancelled"],
                Some((outcome, code)),
                true
            ),
            "R209.3/.4 {previous:?}"
        );
        // R209.5.
        for (reason, code) in [
            (
                AudioCancelReason::DeviceChanged,
                DictationErrorCode::DeviceChanged,
            ),
            (
                AudioCancelReason::SystemSleep,
                DictationErrorCode::SystemSleep,
            ),
            (
                AudioCancelReason::SystemWake,
                DictationErrorCode::SystemWake,
            ),
        ] {
            assert_eq!(
                recovering(reason, previous),
                base(
                    &["audio-recovery-started"],
                    Some((DictationTerminalOutcome::RuntimeInterruption, code)),
                    true
                ),
                "R209.5 {previous:?} {reason:?}"
            );
        }
        // R209.6: the context is kept and no terminal is claimed.
        assert_eq!(
            recovering(AudioCancelReason::RuntimeFailure, previous),
            base(&["audio-recovery-started"], None, false),
            "R209.6 {previous:?}"
        );
        // R209.7: the context is cleared, still no terminal.
        assert_eq!(
            recovering(AudioCancelReason::HardDeadline, previous),
            base(&["audio-recovery-started"], None, true),
            "R209.7 {previous:?}"
        );
    }
}

#[test]
fn site_210_cleanup_finished() {
    let idle = r#"recording-status-changed "idle""#.to_string();
    for status in ALL {
        // R210.1: a newer recording is never reset: no terminal, no emit.
        assert_eq!(
            run(AudioLifecycleEvent::CleanupFinished, status, NEWER, false),
            untouched(status, None),
            "R210.1 {status:?}"
        );
        let got = run(AudioLifecycleEvent::CleanupFinished, status, RID, false);
        let expected = match status {
            // R210.2 / R210.3: restored to Idle.
            Idle | Recovering => Outcome {
                events: vec!["recording-cleanup-finished".into(), idle.clone()],
                ..untouched(Idle, None)
            },
            // R210.4: the event only.
            Starting | Recording | Processing => Outcome {
                events: vec!["recording-cleanup-finished".into()],
                ..untouched(status, None)
            },
        };
        assert_eq!(got, expected, "R210.2-.4 {status:?}");
    }
}

#[test]
fn site_211_idle() {
    for status in ALL {
        // R211.1: not current.
        assert_eq!(
            run(AudioLifecycleEvent::Idle, status, NEWER, true),
            untouched(status, superseded()),
            "R211.1 {status:?}"
        );
    }
    // R211.2 (PD-2): current and already Idle: superseded, no keyboard reset.
    assert_eq!(
        run(AudioLifecycleEvent::Idle, Idle, RID, true),
        untouched(Idle, superseded())
    );
    // R211.3-.6 (PD-7 for Processing): forced to Idle over any live status.
    for status in [Processing, Starting, Recording, Recovering] {
        assert_eq!(
            run(AudioLifecycleEvent::Idle, status, RID, true),
            Outcome {
                events: vec![r#"recording-status-changed "idle""#.into()],
                keyboard_cleared: true,
                context_cleared: true,
                ..untouched(
                    Idle,
                    Some((
                        DictationTerminalOutcome::PipelineFailure,
                        DictationErrorCode::RuntimeFailure
                    ))
                )
            },
            "R211.3-.6 {status:?}"
        );
    }
}

#[test]
fn site_212_accepted() {
    for status in ALL {
        assert_eq!(
            run(AudioLifecycleEvent::Accepted, status, RID, false),
            untouched(status, None),
            "current {status:?}"
        );
        assert_eq!(
            run(AudioLifecycleEvent::Accepted, status, NEWER, false),
            untouched(status, superseded()),
            "stale {status:?}"
        );
    }
}

fn init_failed(recovery_reason: Option<AudioCancelReason>) -> AudioLifecycleEvent {
    AudioLifecycleEvent::InitializationFailed {
        error: "typed failure".to_string(),
        kind: audio::AudioFailureKind::StreamInvalidated,
        recovery_reason,
    }
}

#[test]
fn site_213_initialization_failed_never_writes_status() {
    for status in ALL {
        // R213.1.
        assert_eq!(
            run(init_failed(None), status, NEWER, true),
            untouched(status, superseded()),
            "R213.1 {status:?}"
        );
        // R213.2: the status is left exactly as it was.
        assert_eq!(
            run(init_failed(None), status, RID, true),
            Outcome {
                events: vec!["recording-initialization-failed".into()],
                context_cleared: true,
                ..untouched(
                    status,
                    Some((
                        DictationTerminalOutcome::CaptureInitFailure,
                        DictationErrorCode::StreamInvalidated
                    ))
                )
            },
            "R213.2 {status:?}"
        );
    }
    assert_eq!(
        run(
            init_failed(Some(AudioCancelReason::RuntimeFailure)),
            Starting,
            RID,
            true
        )
        .terminal,
        Some((
            DictationTerminalOutcome::RuntimeInterruption,
            DictationErrorCode::StreamInvalidated
        ))
    );
}

#[test]
fn site_214_recovery_stalled() {
    for status in ALL {
        assert_eq!(
            run(AudioLifecycleEvent::RecoveryStalled, status, RID, false),
            Outcome {
                events: vec!["recording-recovery-stalled".into()],
                ..untouched(status, None)
            },
            "R214.1 {status:?}"
        );
        assert_eq!(
            run(AudioLifecycleEvent::RecoveryStalled, status, NEWER, false),
            untouched(status, None),
            "R214.2 {status:?}"
        );
    }
}

fn interrupted(delivered_samples: u64) -> AudioLifecycleEvent {
    AudioLifecycleEvent::Interrupted {
        reason: AudioCancelReason::DeviceChanged,
        delivered_samples,
        duration_ms: 900,
    }
}

#[test]
fn sites_215_and_216_interrupted() {
    for status in ALL {
        // R215.1: not current (any length): superseded, nothing else.
        for samples in [0, 8_000] {
            assert_eq!(
                run(interrupted(samples), status, NEWER, true),
                untouched(status, superseded()),
                "R215.1 {status:?} {samples}"
            );
        }
        // R215.2: 8000 samples or more auto-transcribes; no status write, no terminal.
        assert_eq!(
            run(interrupted(8_000), status, RID, true),
            Outcome {
                events: vec!["recording-interrupted".into()],
                auto_stop: true,
                ..untouched(status, None)
            },
            "R215.2 {status:?}"
        );
        // R215.3 + R216.1-.4: shorter hands over to the unguarded Idle write (PD-1).
        assert_eq!(
            run(interrupted(7_999), status, RID, true),
            Outcome {
                events: vec![
                    "recording-interrupted".into(),
                    r#"recording-status-changed "idle""#.into()
                ],
                keyboard_cleared: true,
                context_cleared: true,
                ..untouched(
                    Idle,
                    Some((
                        DictationTerminalOutcome::RuntimeInterruption,
                        DictationErrorCode::RuntimeFailure
                    ))
                )
            },
            "R216 {status:?}"
        );
    }
}

// ----------------------------------------------------------- C4.1 step 0 / C4.2

/// A lifecycle event of every kind, built fresh (the type is not `Clone`).
type EventMaker = fn() -> AudioLifecycleEvent;

fn every_event() -> Vec<(&'static str, EventMaker)> {
    vec![
        ("Ready", || AudioLifecycleEvent::Ready),
        ("StillConnecting", || AudioLifecycleEvent::StillConnecting),
        ("Recovering", || AudioLifecycleEvent::Recovering {
            reason: AudioCancelReason::User,
        }),
        ("CleanupFinished", || AudioLifecycleEvent::CleanupFinished),
        ("Idle", || AudioLifecycleEvent::Idle),
        ("Accepted", || AudioLifecycleEvent::Accepted),
        ("RecoveryStalled", || AudioLifecycleEvent::RecoveryStalled),
        ("InitializationFailed", || init_failed(None)),
        ("Interrupted", || interrupted(8_000)),
        ("StartupDiagnostic", || {
            AudioLifecycleEvent::StartupDiagnostic(audio::AudioStartupDiagnostic::GestureAccepted {
                gesture_expected: false,
                double_tap_expected: false,
                press_to_accepted_ms: None,
                double_tap_gap_ms: None,
            })
        }),
    ]
}

/// The bridge refuses an entry nested in its own `In`-phase emit **before** it
/// asks for D. `CleanupFinished` emits `recording-cleanup-finished` with D held;
/// an inline listener re-enters the real bridge with each kind of event, and every
/// D-taking arm would deadlock on the non-reentrant D if the check came after the
/// lock. The body runs on its own thread under a watchdog so a regression fails
/// instead of hanging the suite.
#[test]
fn a_lifecycle_event_nested_in_an_in_phase_emit_is_refused_before_d() {
    for (name, nested) in every_event() {
        let (done, finished) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let outcome = std::panic::catch_unwind(|| nested_entry_outcome(nested));
            let _ = done.send(outcome.map_err(|_| "panicked"));
        });
        match finished.recv_timeout(std::time::Duration::from_secs(20)) {
            Ok(Ok(outcome)) => {
                assert!(outcome.d_held_during_emit, "{name}: the emit ran under D");
                assert_eq!(outcome.nested_calls, 1, "{name}: the listener ran inline");
                // Exactly the outer arm's events: the nested entry did nothing.
                assert_eq!(
                    outcome.events,
                    [
                        "recording-cleanup-finished",
                        r#"recording-status-changed "idle""#
                    ],
                    "{name}: a refused entry emits nothing"
                );
                assert_eq!(outcome.terminal, None, "{name}: no terminal was claimed");
                assert!(
                    !outcome.spawned,
                    "{name}: no stop or side task was scheduled"
                );
            }
            Ok(Err(why)) => panic!("{name}: the nested entry {why}"),
            Err(_) => panic!("{name}: deadlocked: the nested entry asked for D before refusing"),
        }
    }
}

struct Nested {
    d_held_during_emit: bool,
    nested_calls: u32,
    events: Vec<String>,
    terminal: Terminal,
    spawned: bool,
}

fn nested_entry_outcome(nested: fn() -> AudioLifecycleEvent) -> Nested {
    let app_state = AppState::default();
    app_state.recording_id.store(RID, Ordering::SeqCst);
    {
        let mut dictation = app_state.dictation.lock_or_recover();
        dictation.status = Recovering;
        // Live fields written directly: seed the persisted core to match, as
        // `run_with_core` does, so a normal run counts no divergence.
        crate::dictation_executor::FieldAdapter::seed_core(&app_state, &mut dictation);
    }
    app_state.dictation_telemetry.accepted(RID);
    let app = tauri::test::mock_builder()
        .manage(test_state(app_state))
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("build mock Tauri app");
    let handle = app.handle().clone();
    let events = Arc::new(StdMutex::new(Vec::new()));
    for name in EVENTS {
        let events = Arc::clone(&events);
        handle.listen_any(name, move |event| {
            let entry = if name == "recording-status-changed" {
                format!("{name} {}", event.payload())
            } else {
                name.to_string()
            };
            events.lock().unwrap().push(entry);
        });
    }
    let spawned = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let d_held = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let calls = Arc::new(std::sync::atomic::AtomicU32::new(0));
    {
        let (handle, spawned, d_held, calls) = (
            handle.clone(),
            Arc::clone(&spawned),
            Arc::clone(&d_held),
            Arc::clone(&calls),
        );
        handle
            .clone()
            .listen_any("recording-cleanup-finished", move |_| {
                calls.fetch_add(1, Ordering::SeqCst);
                let state = handle.state::<State>();
                d_held.store(
                    state.app_state.dictation.try_lock().is_err(),
                    Ordering::SeqCst,
                );
                let (auto, ready) = (Arc::clone(&spawned), Arc::clone(&spawned));
                handle_audio_lifecycle_and_ready_tasks(
                    handle.clone(),
                    RID,
                    nested(),
                    move |_, _| auto.store(true, Ordering::SeqCst),
                    move |_, _, _| ready.store(true, Ordering::SeqCst),
                );
            });
    }
    handle_audio_lifecycle_and_ready_tasks(
        handle.clone(),
        RID,
        AudioLifecycleEvent::CleanupFinished,
        |_, _| {},
        |_, _, _| {},
    );
    let state = handle.state::<State>();
    let outcome = Nested {
        d_held_during_emit: d_held.load(Ordering::SeqCst),
        nested_calls: calls.load(Ordering::SeqCst),
        events: events.lock().unwrap().clone(),
        terminal: state.app_state.dictation_telemetry.terminal_for_test(RID),
        spawned: spawned.load(Ordering::SeqCst),
    };
    outcome
}

// ------------------------------------------------------------- R215.2 `AutoStop`

fn poll_once<F: Future + Unpin>(future: &mut F) -> Poll<F::Output> {
    Pin::new(future).poll(&mut Context::from_waker(Waker::noop()))
}

type MockApp = tauri::App<tauri::test::MockRuntime>;

fn mock_app(status: DictationStatus, current: u64) -> MockApp {
    let app_state = AppState::default();
    app_state.recording_id.store(current, Ordering::SeqCst);
    app_state.dictation.lock_or_recover().status = status;
    tauri::test::mock_builder()
        .manage(test_state(app_state))
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("build mock Tauri app")
}

fn outstanding(app: &MockApp) -> usize {
    app.handle()
        .state::<State>()
        .app_state
        .core_history
        .outstanding_ops()
}

/// Deliver `Interrupted` with `delivered_samples` for `RID`; `spawn` is the seam
/// that receives the `AutoStop` owner (R215.2 `RS`). Returns how many ops were
/// outstanding when the `recording-interrupted` event was emitted (the effect
/// before `RS`).
fn deliver_interrupted(
    app: &MockApp,
    delivered_samples: u64,
    spawn: impl FnOnce(tauri::AppHandle<tauri::test::MockRuntime>, u64, ContinuationGuard),
) -> usize {
    let handle = app.handle().clone();
    let at_emit = Arc::new(std::sync::atomic::AtomicUsize::new(usize::MAX));
    {
        let (handle, at_emit) = (handle.clone(), Arc::clone(&at_emit));
        handle
            .clone()
            .listen_any("recording-interrupted", move |_| {
                at_emit.store(
                    handle
                        .state::<State>()
                        .app_state
                        .core_history
                        .outstanding_ops(),
                    Ordering::SeqCst,
                );
            });
    }
    handle_audio_lifecycle_events(
        handle,
        RID,
        interrupted(delivered_samples),
        spawn,
        |_, _, _| {},
    );
    at_emit.load(Ordering::SeqCst)
}

/// The spawned stop's future for `owner`, as production's spawn builds it, driven
/// by hand on a fake host.
fn stop_future<'a>(
    app: &'a MockApp,
    host: &'a super::host_tests::FakeHost,
    expected: u64,
    owner: ContinuationGuard,
) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, String>> + 'a>> {
    let state = app.state::<State>();
    Box::pin(async move {
        stop_native_recording_admitted(
            host,
            state.inner(),
            Some(expected),
            audio_lifecycle::DictationStopTrigger::Other,
            Some(owner),
        )
        .await
    })
}

fn fake_host() -> super::host_tests::FakeHost {
    super::host_tests::FakeHost::new(Arc::new(test_state(AppState::default())))
}

/// The `AutoStop` op is registered before the first effect runs, stays
/// outstanding while the spawned stop waits for T, and is consumed at admission
/// when T is granted.
#[test]
fn the_auto_stop_op_is_pending_while_the_stop_waits_on_t_and_consumed_at_admission() {
    let app = mock_app(Idle, RID);
    let slot = Rc::new(RefCell::new(None));
    let kept = Rc::clone(&slot);
    let at_emit = deliver_interrupted(&app, 8_000, move |_, id, owner| {
        assert_eq!(id, RID);
        *kept.borrow_mut() = Some(owner);
    });
    assert_eq!(at_emit, 1, "registered before the emit, the first effect");
    assert_eq!(
        outstanding(&app),
        1,
        "the owner is held for the spawned stop"
    );

    let host = fake_host();
    let owner = slot.borrow_mut().take().expect("RS handed the owner on");
    let state = app.state::<State>();
    let transition = state
        .app_state
        .recording_transition
        .try_lock()
        .expect("T is free");
    let mut stop = stop_future(&app, &host, RID, owner);
    assert!(poll_once(&mut stop).is_pending(), "the stop waits on T");
    assert_eq!(outstanding(&app), 1, "still pending while T is blocked");

    drop(transition);
    let reply = match poll_once(&mut stop) {
        Poll::Ready(reply) => reply.expect("the stop answers"),
        Poll::Pending => panic!("the stop was granted T"),
    };
    assert_eq!(reply["type"], "not_recording");
    assert_eq!(outstanding(&app), 0, "consumed at stop admission");
}

/// Admission consumes the op whatever the stale rule then decides: a stop for an
/// older generation still runs its own stale rule and still retires the op.
#[test]
fn a_stale_auto_stop_is_consumed_at_admission_too() {
    let app = mock_app(Starting, RID + 1);
    let slot = Rc::new(RefCell::new(None));
    let kept = Rc::clone(&slot);
    // The recording is no longer current, so R215.1 registers nothing; register the
    // op the way the arm does for the generation that was current when it ran.
    let guard = ContinuationGuard::register(
        &app.state::<State>().app_state.core_history,
        murmur_dictation_core::decision::ContinuationSpec {
            recording_id: murmur_dictation_core::ids::RecordingId(RID),
            kind: murmur_dictation_core::decision::ContKind::AutoStop,
        },
    );
    *kept.borrow_mut() = Some(guard);
    assert_eq!(outstanding(&app), 1);
    let host = fake_host();
    let owner = slot.borrow_mut().take().unwrap();
    let mut stop = stop_future(&app, &host, RID, owner);
    let reply = match poll_once(&mut stop) {
        Poll::Ready(reply) => reply.expect("the stop answers"),
        Poll::Pending => panic!("T is free"),
    };
    assert_eq!(reply["type"], "recording_superseded");
    assert_eq!(outstanding(&app), 0);
}

/// A spawned stop cancelled while it waits on T (its task aborted or dropped)
/// retires the op: nothing consumed it.
#[test]
fn a_cancelled_auto_stop_task_retires_its_op() {
    let app = mock_app(Idle, RID);
    let slot = Rc::new(RefCell::new(None));
    let kept = Rc::clone(&slot);
    deliver_interrupted(&app, 8_000, move |_, _, owner| {
        *kept.borrow_mut() = Some(owner);
    });
    let host = fake_host();
    let owner = slot.borrow_mut().take().unwrap();
    let state = app.state::<State>();
    let _transition = state
        .app_state
        .recording_transition
        .try_lock()
        .expect("T is free");
    let mut stop = stop_future(&app, &host, RID, owner);
    assert!(poll_once(&mut stop).is_pending());
    assert_eq!(outstanding(&app), 1);
    drop(stop);
    assert_eq!(outstanding(&app), 0, "the dropped future retired the op");
}

/// A panic where the owner is in hand (the spawn seam itself, or a spawned task
/// before it consumes) unwinds through the owner's `Drop`.
#[test]
fn a_panicking_auto_stop_spawn_retires_its_op() {
    let app = mock_app(Idle, RID);
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        deliver_interrupted(&app, 8_000, |_, _, _owner| panic!("spawn failed"));
    }));
    assert!(unwound.is_err());
    assert_eq!(outstanding(&app), 0);
    // And a panic while the spawned stop holds the owner pending on T.
    let slot = Rc::new(RefCell::new(None));
    let kept = Rc::clone(&slot);
    deliver_interrupted(&app, 8_000, move |_, _, owner| {
        *kept.borrow_mut() = Some(owner);
    });
    assert_eq!(outstanding(&app), 1);
    let owner = slot.borrow_mut().take().unwrap();
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let _owner = owner;
        panic!("task panicked before admission");
    }));
    assert!(unwound.is_err());
    assert_eq!(outstanding(&app), 0);
}

/// Only R215.2 registers: a short capture (R215.3) and a stale one (R215.1)
/// leave nothing outstanding, and an `RS` that never runs retires its op.
#[test]
fn only_a_long_current_interruption_registers_and_an_unrun_rs_retires() {
    for (status, current, samples) in [(Idle, RID, 7_999), (Starting, RID + 1, 8_000)] {
        let app = mock_app(status, current);
        let at_emit = deliver_interrupted(&app, samples, |_, _, _| {
            panic!("no stop is scheduled for this row")
        });
        assert_ne!(at_emit, 1);
        assert_eq!(outstanding(&app), 0);
    }
    // The seam receives the owner and drops it unrun (a task that never ran).
    let app = mock_app(Idle, RID);
    deliver_interrupted(&app, 8_000, |_, _, owner| drop(owner));
    assert_eq!(outstanding(&app), 0);
}

/// Consumption happens **at admission**, not at the end of the stop: the op is
/// already retired while the stop is still running (inside the supervisor stop),
/// so an owner held through the whole stop is caught here.
#[test]
fn the_auto_stop_op_is_retired_at_admission_not_held_through_the_stop() {
    let history = crate::core_history::CoreHistoryLock::default();
    let owner = ContinuationGuard::register(
        &history,
        murmur_dictation_core::decision::ContinuationSpec {
            recording_id: murmur_dictation_core::ids::RecordingId(RID),
            kind: murmur_dictation_core::decision::ContKind::AutoStop,
        },
    );
    assert_eq!(history.outstanding_ops(), 1);
    let during = Arc::new(std::sync::atomic::AtomicUsize::new(usize::MAX));
    let probe = {
        let (history, during) = (history.clone(), Arc::clone(&during));
        move || during.store(history.outstanding_ops(), Ordering::SeqCst)
    };
    let reply = super::host_tests::auto_stop_with_probe(owner, probe);
    assert_eq!(reply["type"], "transcription");
    assert_eq!(
        during.load(Ordering::SeqCst),
        0,
        "retired before the stop ran"
    );
    assert_eq!(history.outstanding_ops(), 0);
}

// --------------------------------------------------- D-S2-9: the terminal claim

/// A stale `Ready` (R207.1) decides `TERM(Superseded/StaleOwner)`; the executor's
/// witness for it is the telemetry ledger's real answer.
fn witness_of_a_stale_ready(claim_first: bool) -> Vec<(&'static str, WitnessOutcome)> {
    let app_state = AppState::default();
    app_state.recording_id.store(NEWER, Ordering::SeqCst);
    app_state.dictation.lock_or_recover().status = Recording;
    app_state.dictation_telemetry.accepted(RID);
    if claim_first {
        // Another sink already claimed this id's terminal.
        assert!(app_state.dictation_telemetry.emit_terminal_with_output(
            RID,
            DictationTerminalOutcome::Superseded,
            DictationErrorCode::StaleOwner,
            0
        ));
    }
    let app = tauri::test::mock_builder()
        .manage(test_state(app_state))
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("build mock Tauri app");
    lifecycle_ports::LAST_WITNESS.with(|slot| slot.borrow_mut().clear());
    handle_audio_lifecycle_and_ready_tasks(
        app.handle().clone(),
        RID,
        AudioLifecycleEvent::Ready,
        |_, _| {},
        |_, _, _| {},
    );
    lifecycle_ports::LAST_WITNESS.with(|slot| {
        slot.borrow()
            .iter()
            .filter(|(code, _)| *code == "TERM")
            .copied()
            .collect()
    })
}

use crate::dictation_executor::executor::WitnessOutcome;

/// D-S2-9, winning claim: the ledger accepts the terminal and the witness is `Ran`.
#[test]
fn a_winning_terminal_claim_is_witnessed_as_ran() {
    let witness = witness_of_a_stale_ready(false);
    assert!(!witness.is_empty(), "the stale Ready decides a TERM");
    assert!(
        witness
            .iter()
            .all(|(_, outcome)| *outcome == WitnessOutcome::Ran),
        "{witness:?}"
    );
}

/// D-S2-9, losing claim: a stale `Ready` for an id whose terminal was already
/// claimed. The ledger rejects the claim, so the witness is `Failed`, not `Ran`.
#[test]
fn a_losing_terminal_claim_is_witnessed_as_failed() {
    let witness = witness_of_a_stale_ready(true);
    assert!(!witness.is_empty(), "the stale Ready decides a TERM");
    assert!(
        witness
            .iter()
            .all(|(_, outcome)| *outcome == WitnessOutcome::Failed),
        "{witness:?}"
    );
}
