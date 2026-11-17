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

use super::tests::test_state;
use super::*;
use std::cell::Cell;
use std::sync::Mutex as StdMutex;
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
struct Outcome {
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
    let app_state = AppState::default();
    app_state.recording_id.store(current, Ordering::SeqCst);
    app_state.dictation.lock_or_recover().status = status;
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
