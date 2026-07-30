#![cfg(all(target_os = "macos", target_arch = "aarch64"))]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use ui_lib::capture_helper_probe::{
    CaptureProbeError, CaptureProbeSupervisor, TestCaptureProbeConfig,
};

fn helper_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/support/mock_capture_helper.py")
}

fn supervisor(scenario: &str) -> CaptureProbeSupervisor {
    CaptureProbeSupervisor::for_test(TestCaptureProbeConfig {
        helper_path: helper_path(),
        scenario_environment: vec![("MOCK_CAPTURE_SCENARIO".to_string(), scenario.to_string())],
        handshake_timeout: Duration::from_millis(400),
        observe_for: Duration::from_millis(450),
        cancel_grace: Duration::from_millis(100),
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
fn rapid_second_start_is_rejected_without_overlapping_helper() {
    let supervisor = Arc::new(supervisor("pre_handshake_block"));
    let running = Arc::clone(&supervisor);
    let first = std::thread::spawn(move || running.run().unwrap());
    std::thread::sleep(Duration::from_millis(50));

    assert_eq!(supervisor.run().unwrap_err(), CaptureProbeError::Busy);
    let evidence = first.join().unwrap();
    assert_eq!(evidence.termination, "hard_kill");
    assert!(evidence.process_group_empty);
}
