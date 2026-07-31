#![cfg(all(target_os = "macos", target_arch = "aarch64"))]

use std::path::PathBuf;
use std::sync::{mpsc, Arc};
use std::time::Duration;
use ui_lib::capture_helper_probe::{
    CaptureProbeError, CaptureProbeSupervisor, TestCaptureProbeConfig,
};

fn helper_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/support/mock_capture_helper.py")
}

fn supervisor(scenario: &str) -> CaptureProbeSupervisor {
    supervisor_with_environment(vec![(
        "MOCK_CAPTURE_SCENARIO".to_string(),
        scenario.to_string(),
    )])
}

fn supervisor_with_environment(
    scenario_environment: Vec<(String, String)>,
) -> CaptureProbeSupervisor {
    CaptureProbeSupervisor::for_test(TestCaptureProbeConfig {
        helper_path: helper_path(),
        scenario_environment,
        permission_status_sequence: Vec::new(),
        handshake_timeout: Duration::from_millis(400),
        observe_for: Duration::from_millis(450),
        cancel_grace: Duration::from_millis(100),
        spawned_signal: None,
    })
}

#[test]
fn every_simulated_hang_is_hard_killed_reaped_and_bounded() {
    for (scenario, expected_phase) in [
        ("pre_handshake_block", None),
        ("enumeration_block", Some("enumeration")),
        ("open_block", Some("stream_open")),
        ("starts_without_callbacks", Some("awaiting_first_callback")),
        ("after_first_audio_block", Some("active")),
        ("ignore_cancel", Some("active")),
        ("graceful_stop_block", Some("stopping")),
        ("descendant_block", Some("active")),
    ] {
        let evidence = supervisor(scenario).run().unwrap();
        assert_eq!(evidence.last_phase, expected_phase, "scenario={scenario}");
        assert_eq!(evidence.termination, "hard_kill", "scenario={scenario}");
        assert_eq!(
            evidence.exit_signal,
            Some(libc::SIGKILL),
            "scenario={scenario}"
        );
        assert!(
            evidence.process_group_empty,
            "owned process group survived: scenario={scenario}"
        );
        assert!(
            evidence.elapsed_ms < 1_000,
            "termination exceeded bound: scenario={scenario} elapsed={}",
            evidence.elapsed_ms
        );
        assert!(!evidence.audio_content_retained);
    }
}

#[test]
fn cooperative_stop_exits_and_same_supervisor_can_start_fresh_helper() {
    let supervisor = supervisor("happy");
    let first = supervisor.run().unwrap();
    assert_eq!(first.outcome, "ok");
    assert_eq!(first.termination, "cooperative");
    assert!(first.process_group_empty);

    let second = supervisor.run().unwrap();
    assert_eq!(second.outcome, "ok");
    assert_eq!(second.termination, "cooperative");
    assert!(second.process_group_empty);
    assert_ne!(first.helper_pid, second.helper_pid);
}

#[test]
fn malformed_or_out_of_order_control_frames_fail_closed() {
    for scenario in [
        "wrong_nonce",
        "wrong_version",
        "malformed",
        "truncated",
        "oversized",
        "ready_out_of_order",
        "first_callback_without_awaiting",
        "duplicate_phase",
        "phase_regression",
    ] {
        let evidence = supervisor(scenario).run().unwrap();
        assert_eq!(evidence.outcome, "protocol", "scenario={scenario}");
        assert_eq!(evidence.termination, "hard_kill", "scenario={scenario}");
        assert_eq!(evidence.exit_signal, Some(libc::SIGKILL));
        assert!(evidence.process_group_empty, "scenario={scenario}");
        assert!(!evidence.audio_content_retained);
    }
}

#[test]
fn incomplete_handshakes_never_classify_as_success() {
    let ready_only = supervisor("ready_without_awaiting").run().unwrap();
    assert_eq!(ready_only.outcome, "handshake_timeout");
    assert_eq!(ready_only.last_phase, Some("stream_open"));
    assert_eq!(ready_only.first_callback_ms, None);
    assert_eq!(ready_only.termination, "hard_kill");

    let no_callback = supervisor("starts_without_callbacks").run().unwrap();
    assert_eq!(no_callback.outcome, "no_first_callback");
    assert_eq!(no_callback.last_phase, Some("awaiting_first_callback"));
    assert_eq!(no_callback.first_callback_ms, None);
    assert_eq!(no_callback.termination, "hard_kill");

    let missing_active = supervisor("missing_active").run().unwrap();
    assert_eq!(missing_active.outcome, "handshake_timeout");
    assert_eq!(missing_active.last_phase, Some("awaiting_first_callback"));
    assert_eq!(missing_active.first_callback_ms, Some(1));
    assert_eq!(missing_active.termination, "hard_kill");
}

#[test]
fn active_observation_window_starts_after_the_complete_handshake() {
    let supervisor = CaptureProbeSupervisor::for_test(TestCaptureProbeConfig {
        helper_path: helper_path(),
        scenario_environment: vec![(
            "MOCK_CAPTURE_SCENARIO".to_string(),
            "delayed_active".to_string(),
        )],
        permission_status_sequence: Vec::new(),
        handshake_timeout: Duration::from_millis(800),
        observe_for: Duration::from_millis(300),
        cancel_grace: Duration::from_millis(100),
        spawned_signal: None,
    });
    let evidence = supervisor.run().unwrap();
    assert_eq!(evidence.outcome, "ok");
    assert_eq!(evidence.termination, "cooperative");
    assert!(
        evidence.elapsed_ms >= 500,
        "active observation was shortened by handshake time: elapsed={}",
        evidence.elapsed_ms
    );
    assert!(
        evidence.elapsed_ms < 1_200,
        "handshake plus observation exceeded the total bound: elapsed={}",
        evidence.elapsed_ms
    );
}

#[test]
fn same_supervisor_recovers_from_confirmed_hang_to_happy_fresh_launch() {
    let marker = std::env::temp_dir().join(format!(
        "murmur-capture-hang-then-happy-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&marker);
    let supervisor = supervisor_with_environment(vec![
        (
            "MOCK_CAPTURE_SCENARIO".to_string(),
            "hang_then_happy".to_string(),
        ),
        (
            "MOCK_CAPTURE_MARKER".to_string(),
            marker.to_string_lossy().into_owned(),
        ),
    ]);

    let hung = supervisor.run().unwrap();
    assert_eq!(hung.outcome, "handshake_timeout");
    assert_eq!(hung.termination, "hard_kill");
    assert!(hung.process_group_empty);

    let fresh = supervisor.run().unwrap();
    let _ = std::fs::remove_file(marker);
    assert_eq!(fresh.outcome, "ok");
    assert_eq!(fresh.termination, "cooperative");
    assert!(fresh.process_group_empty);
    assert_ne!(hung.helper_pid, fresh.helper_pid);
}

#[test]
fn rapid_second_start_is_rejected_without_overlapping_helper() {
    let (spawned_tx, spawned_rx) = mpsc::channel();
    let supervisor = Arc::new(CaptureProbeSupervisor::for_test(TestCaptureProbeConfig {
        helper_path: helper_path(),
        scenario_environment: vec![(
            "MOCK_CAPTURE_SCENARIO".to_string(),
            "pre_handshake_block".to_string(),
        )],
        permission_status_sequence: Vec::new(),
        handshake_timeout: Duration::from_millis(400),
        observe_for: Duration::from_millis(450),
        cancel_grace: Duration::from_millis(100),
        spawned_signal: Some(spawned_tx),
    }));
    let running = Arc::clone(&supervisor);
    let first = std::thread::spawn(move || running.run().unwrap());
    let spawned_pid = spawned_rx
        .recv_timeout(Duration::from_millis(400))
        .expect("first helper must be owned before the overlap attempt");

    assert_eq!(supervisor.run().unwrap_err(), CaptureProbeError::Busy);
    let evidence = first.join().unwrap();
    assert_eq!(evidence.helper_pid, spawned_pid);
    assert_eq!(evidence.termination, "hard_kill");
    assert!(evidence.process_group_empty);
}

#[test]
fn live_permission_revocation_is_detected_even_when_callbacks_continue() {
    let supervisor = CaptureProbeSupervisor::for_test(TestCaptureProbeConfig {
        helper_path: helper_path(),
        scenario_environment: vec![("MOCK_CAPTURE_SCENARIO".to_string(), "happy".to_string())],
        permission_status_sequence: vec!["granted".to_string(), "denied".to_string()],
        handshake_timeout: Duration::from_millis(400),
        observe_for: Duration::from_secs(5),
        cancel_grace: Duration::from_millis(100),
        spawned_signal: None,
    });

    let evidence = supervisor.run().unwrap();
    assert_eq!(evidence.outcome, "permission_denied");
    assert_eq!(evidence.termination, "cooperative");
    assert!(evidence.elapsed_ms < 1_000);
    assert!(evidence.process_group_empty);
    assert!(!evidence.audio_content_retained);
}

#[test]
fn every_non_granted_permission_status_is_rejected_before_the_helper_spawns() {
    for status in ["denied", "notDetermined", "unknown"] {
        let (spawned_tx, spawned_rx) = mpsc::channel();
        let supervisor = CaptureProbeSupervisor::for_test(TestCaptureProbeConfig {
            helper_path: helper_path(),
            scenario_environment: vec![("MOCK_CAPTURE_SCENARIO".to_string(), "happy".to_string())],
            permission_status_sequence: vec![status.to_string()],
            handshake_timeout: Duration::from_millis(400),
            observe_for: Duration::from_secs(5),
            cancel_grace: Duration::from_millis(100),
            spawned_signal: Some(spawned_tx),
        });

        assert_eq!(
            supervisor.run().unwrap_err(),
            CaptureProbeError::HelperFailed(
                murmur_capture_helper_protocol::FailureCode::PermissionDenied
            ),
            "status={status}"
        );
        assert!(spawned_rx.try_recv().is_err(), "status={status}");
    }
}

#[test]
fn every_loss_of_a_proven_grant_interrupts_active_capture() {
    for status in ["denied", "notDetermined", "unknown"] {
        let supervisor = CaptureProbeSupervisor::for_test(TestCaptureProbeConfig {
            helper_path: helper_path(),
            scenario_environment: vec![("MOCK_CAPTURE_SCENARIO".to_string(), "happy".to_string())],
            permission_status_sequence: vec!["granted".to_string(), status.to_string()],
            handshake_timeout: Duration::from_millis(400),
            observe_for: Duration::from_secs(5),
            cancel_grace: Duration::from_millis(100),
            spawned_signal: None,
        });

        let evidence = supervisor.run().unwrap();
        assert_eq!(evidence.outcome, "permission_denied", "status={status}");
        assert_eq!(evidence.termination, "cooperative", "status={status}");
        assert!(evidence.elapsed_ms < 1_000, "status={status}");
        assert!(evidence.process_group_empty, "status={status}");
        assert!(!evidence.audio_content_retained, "status={status}");
    }
}

#[test]
fn revocation_outcome_survives_a_queued_protocol_failure_during_teardown() {
    let supervisor = CaptureProbeSupervisor::for_test(TestCaptureProbeConfig {
        helper_path: helper_path(),
        scenario_environment: vec![(
            "MOCK_CAPTURE_SCENARIO".to_string(),
            "phase_regression".to_string(),
        )],
        permission_status_sequence: vec!["granted".to_string(), "unknown".to_string()],
        handshake_timeout: Duration::from_millis(400),
        observe_for: Duration::from_secs(5),
        cancel_grace: Duration::from_millis(100),
        spawned_signal: None,
    });

    let evidence = supervisor.run().unwrap();
    assert_eq!(evidence.outcome, "permission_denied");
    assert_eq!(evidence.termination, "hard_kill");
    assert!(evidence.elapsed_ms < 1_000);
    assert!(evidence.process_group_empty);
    assert!(!evidence.audio_content_retained);
}

#[test]
fn production_capture_helper_has_no_process_spawn_or_daemonization_surface() {
    let source = include_str!("../sidecars/capture/src/main.rs");
    for forbidden in [
        "std::process::Command",
        "libc::fork(",
        "libc::posix_spawn",
        "libc::setsid(",
        "libc::setpgid(",
        "libc::daemon(",
    ] {
        assert!(
            !source.contains(forbidden),
            "capture helper must not contain process escape surface {forbidden:?}"
        );
    }
}
