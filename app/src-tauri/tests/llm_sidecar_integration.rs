//! Integration tests for the local-LLM sidecar supervisor (#312).
//!
//! These drive the real supervisor against the versioned mock helper
//! (`target/<profile>/examples/mock_llm_helper`). No real GGUF model or
//! llama.cpp is involved: the "model" is a small temp file whose pins the test
//! derives.

#![cfg(all(target_os = "macos", target_arch = "aarch64"))]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ui_lib::llm_sidecar::{
    model_file_digest, CancelToken, LlmSidecar, TestSpawnConfig, TransformError, TransformOutput,
};

fn helper_path() -> PathBuf {
    std::env::current_exe()
        .expect("integration test executable path")
        .parent()
        .and_then(|deps| deps.parent())
        .expect("Cargo target profile directory")
        .join("examples/mock_llm_helper")
}

/// A small fixture "model" file plus its exact pins.
struct Fixture {
    _dir: tempfile::TempDir,
    path: PathBuf,
    size: u64,
    sha: String,
}

fn fixture_model() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fixture-model.gguf");
    std::fs::write(&path, b"murmur local-llm fixture model bytes").unwrap();
    let (size, sha) = model_file_digest(&path).unwrap();
    Fixture {
        _dir: dir,
        path,
        size,
        sha,
    }
}

fn config_with(scenario: &str, fixture: &Fixture, env: Vec<(String, String)>) -> TestSpawnConfig {
    let mut scenario_env = vec![("MOCK_SCENARIO".to_string(), scenario.to_string())];
    scenario_env.extend(env);
    TestSpawnConfig {
        helper_path: helper_path(),
        model_path: fixture.path.clone(),
        model_size: fixture.size,
        model_sha256: fixture.sha.clone(),
        scenario_env,
        request_slack: Duration::from_millis(100),
        cancel_grace: Duration::from_millis(150),
        handshake_timeout: Duration::from_secs(5),
        idle_after: Duration::from_secs(300),
    }
}

fn sidecar(scenario: &str, fixture: &Fixture) -> Arc<LlmSidecar> {
    Arc::new(LlmSidecar::for_test(config_with(scenario, fixture, vec![])))
}

async fn run_transform(
    sidecar: &Arc<LlmSidecar>,
    deadline: Duration,
) -> Result<TransformOutput, TransformError> {
    // A fresh per-request cancel token (item 11): each request scopes its own.
    sidecar
        .transform(
            "Rewrite this politely.",
            "gimme the report",
            deadline,
            CancelToken::new(),
        )
        .await
}

#[tokio::test]
async fn successful_transform_round_trip() {
    let fixture = fixture_model();
    let sidecar = sidecar("happy", &fixture);
    let out = run_transform(&sidecar, Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(out.output, "mock-output");
    assert_eq!(out.output_tokens, 3);
    // The helper is healthy and kept resident for reuse.
    assert!(sidecar.has_live_child());
}

#[tokio::test]
async fn correlated_transform_reports_cold_then_warm_phase_timings() {
    let fixture = fixture_model();
    let sidecar = sidecar("happy", &fixture);
    let cold = sidecar
        .transform_for_pass(
            77,
            "Rewrite this politely.",
            "gimme the report",
            Duration::from_secs(5),
            CancelToken::new(),
        )
        .await;
    assert!(cold.result.is_ok());
    assert_eq!(cold.cache_hit, Some(false));
    assert!(cold.spawn_load_ms.is_some());
    assert!(cold.generation_ms.is_some());
    assert!(cold.diagnostics.host_model_verification_ms.is_some());
    assert!(cold.diagnostics.helper_spawn_ms.is_some());
    assert_eq!(cold.diagnostics.helper_model_verification_ms, Some(1));
    assert_eq!(cold.diagnostics.backend_initialization_ms, Some(1));
    assert_eq!(cold.diagnostics.model_load_ms, Some(1));
    assert!(cold.diagnostics.ready_handshake_ms.is_some());
    assert_eq!(cold.diagnostics.request_receipt_ms, Some(0));
    assert_eq!(cold.diagnostics.first_token_ms, Some(1));
    assert!(cold.diagnostics.failure_phase.is_none());
    assert!(sidecar.resident_pid().is_some());

    let warm = sidecar
        .transform_for_pass(
            78,
            "Rewrite this politely.",
            "gimme the report",
            Duration::from_secs(5),
            CancelToken::new(),
        )
        .await;
    assert!(warm.result.is_ok());
    assert_eq!(warm.cache_hit, Some(true));
    assert!(warm.spawn_load_ms.is_some());
    assert!(warm.generation_ms.is_some());
}

#[tokio::test]
async fn every_helper_startup_phase_failure_is_identified() {
    use ui_lib::llm_sidecar::SidecarDiagnosticPhase;

    for (scenario, expected_phase) in [
        (
            "fail_helpermodelverification",
            SidecarDiagnosticPhase::HelperModelVerification,
        ),
        (
            "fail_backendinitialization",
            SidecarDiagnosticPhase::BackendInitialization,
        ),
        ("fail_modelload", SidecarDiagnosticPhase::ModelLoad),
    ] {
        let fixture = fixture_model();
        let sidecar = sidecar(scenario, &fixture);
        let outcome = sidecar
            .transform_for_pass(
                91,
                "Rewrite this politely.",
                "gimme the report",
                Duration::from_secs(5),
                CancelToken::new(),
            )
            .await;
        assert_eq!(outcome.result.unwrap_err(), TransformError::HandshakeFailed);
        assert_eq!(outcome.diagnostics.failure_phase, Some(expected_phase));
        assert_eq!(outcome.diagnostics.process_exit_code, Some(70));
        assert!(!sidecar.has_live_child());
    }
}

#[tokio::test]
async fn timeout_cancels_then_kills_and_reports_timeout() {
    let fixture = fixture_model();
    let sidecar = sidecar("slow_ignore_cancel", &fixture);
    let err = run_transform(&sidecar, Duration::from_millis(200))
        .await
        .unwrap_err();
    assert_eq!(err, TransformError::Timeout);
    // The helper never confirmed the cancel, so it was killed.
    assert!(!sidecar.has_live_child());
}

#[tokio::test]
async fn cancel_path_keeps_confirmed_helper_alive() {
    let fixture = fixture_model();
    let sidecar = sidecar("slow_ack_cancel", &fixture);
    let err = run_transform(&sidecar, Duration::from_millis(200))
        .await
        .unwrap_err();
    assert_eq!(err, TransformError::Timeout);
    // The helper acknowledged the cooperative cancel, so it stays resident.
    assert!(sidecar.has_live_child());
}

#[tokio::test]
async fn crash_reports_crashed_and_trips_circuit_breaker() {
    let fixture = fixture_model();
    let sidecar = sidecar("crash_on_transform", &fixture);
    for _ in 0..3 {
        let err = run_transform(&sidecar, Duration::from_secs(5))
            .await
            .unwrap_err();
        assert_eq!(err, TransformError::Crashed);
        assert!(!sidecar.has_live_child());
    }
    // Three crashes in the window disable the runtime until an explicit reset.
    let err = run_transform(&sidecar, Duration::from_secs(5))
        .await
        .unwrap_err();
    assert_eq!(err, TransformError::Disabled);

    sidecar.reset();
    // After reset the breaker is clear; the next attempt spawns again (and
    // crashes again, proving it is no longer short-circuited as Disabled).
    let err = run_transform(&sidecar, Duration::from_secs(5))
        .await
        .unwrap_err();
    assert_eq!(err, TransformError::Crashed);
}

#[tokio::test]
async fn process_exit_is_reported_with_generation_phase_and_status() {
    use ui_lib::llm_sidecar::SidecarDiagnosticPhase;

    let fixture = fixture_model();
    let sidecar = sidecar("crash_on_transform", &fixture);
    let outcome = sidecar
        .transform_for_pass(
            92,
            "Rewrite this politely.",
            "gimme the report",
            Duration::from_secs(5),
            CancelToken::new(),
        )
        .await;
    assert_eq!(outcome.result.unwrap_err(), TransformError::Crashed);
    assert_eq!(
        outcome.diagnostics.failure_phase,
        Some(SidecarDiagnosticPhase::FirstToken)
    );
    assert_eq!(outcome.diagnostics.process_exit_code, Some(101));
}

#[tokio::test]
async fn helper_deadline_errors_do_not_trip_the_breaker() {
    let fixture = fixture_model();
    let sidecar = sidecar("error_deadline_on_transform", &fixture);
    // Three self-reported DeadlineExceeded outcomes in the window are a designed
    // result, not a fault: the runtime must stay enabled.
    for _ in 0..3 {
        let err = run_transform(&sidecar, Duration::from_secs(5))
            .await
            .unwrap_err();
        assert_eq!(err, TransformError::Timeout);
        // The helper is healthy and kept for reuse.
        assert!(sidecar.has_live_child());
    }
    // A fourth attempt still runs (Timeout), never short-circuited as Disabled.
    let err = run_transform(&sidecar, Duration::from_secs(5))
        .await
        .unwrap_err();
    assert_eq!(err, TransformError::Timeout);
}

#[tokio::test]
async fn malformed_frame_fails_closed_and_kills() {
    let fixture = fixture_model();
    let sidecar = sidecar("malformed_on_transform", &fixture);
    let err = run_transform(&sidecar, Duration::from_secs(5))
        .await
        .unwrap_err();
    assert_eq!(err, TransformError::Protocol);
    assert!(!sidecar.has_live_child());
}

#[tokio::test]
async fn oversized_frame_fails_closed_and_kills() {
    let fixture = fixture_model();
    let sidecar = sidecar("oversized_on_transform", &fixture);
    let err = run_transform(&sidecar, Duration::from_secs(5))
        .await
        .unwrap_err();
    assert_eq!(err, TransformError::Protocol);
    assert!(!sidecar.has_live_child());
}

#[tokio::test]
async fn wrong_handshake_nonce_fails_closed() {
    let fixture = fixture_model();
    let sidecar = sidecar("wrong_nonce", &fixture);
    let err = run_transform(&sidecar, Duration::from_secs(5))
        .await
        .unwrap_err();
    assert_eq!(err, TransformError::HandshakeFailed);
    assert!(!sidecar.has_live_child());
}

#[tokio::test]
async fn one_transform_in_flight_rejects_the_second() {
    let fixture = fixture_model();
    // A deliberate delay so the two requests overlap.
    let sidecar = Arc::new(LlmSidecar::for_test(config_with(
        "happy",
        &fixture,
        vec![("MOCK_DELAY_MS".to_string(), "600".to_string())],
    )));

    let a = {
        let s = Arc::clone(&sidecar);
        tokio::spawn(async move { run_transform(&s, Duration::from_secs(5)).await })
    };
    // Give the first request time to claim the in-flight slot.
    tokio::time::sleep(Duration::from_millis(150)).await;
    let b = run_transform(&sidecar, Duration::from_secs(5)).await;

    let a = a.await.unwrap();
    assert!(a.is_ok(), "first transform should succeed: {a:?}");
    assert_eq!(b.unwrap_err(), TransformError::Busy);
}

#[tokio::test]
async fn wrong_nonce_on_result_frame_fails_closed() {
    let fixture = fixture_model();
    let sidecar = sidecar("wrong_nonce_on_result", &fixture);
    let err = run_transform(&sidecar, Duration::from_secs(5))
        .await
        .unwrap_err();
    // Every frame's nonce is validated, not just the Ready handshake.
    assert_eq!(err, TransformError::Protocol);
    assert!(!sidecar.has_live_child());
}

#[tokio::test]
async fn cooperative_cancel_mid_request_helper_receives_cancel_and_busy_clears() {
    // Finding 2: cancel_inflight_request must send a protocol Cancel, settle
    // the blocking loop, and clear busy promptly — aborting only the outer
    // tokio future is not enough (BusyGuard lives in spawn_blocking).
    let fixture = fixture_model();
    let sidecar = sidecar("slow_honor_cancel", &fixture);

    let s = Arc::clone(&sidecar);
    let handle = tokio::spawn(async move {
        s.transform(
            "Rewrite this politely.",
            "gimme the report",
            Duration::from_secs(30),
            CancelToken::new(),
        )
        .await
    });

    // Wait until the in-flight slot is claimed.
    let mut claimed = false;
    for _ in 0..100 {
        if sidecar.is_transform_busy() {
            claimed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(claimed, "transform never became busy");

    let started = std::time::Instant::now();
    // Cancel the CURRENT in-flight request via the supervisor entry point;
    // internally it flips only the in-flight request's per-request token.
    sidecar.cancel_inflight_request();
    let err = handle.await.unwrap().unwrap_err();
    assert_eq!(err, TransformError::Cancelled);
    // Must clear well under the 30s deadline (cancel grace is ~150ms).
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "busy did not clear promptly after cooperative cancel: {:?}",
        started.elapsed()
    );
    assert!(!sidecar.is_transform_busy());
    // Helper acknowledged Cancel, so it stays resident (same as deadline cancel
    // on slow_ack_cancel). A second Transform against this mock scenario would
    // hang again — reusability of the supervisor after cancel is covered by
    // the happy-path round-trip and dropping_the_transform_future tests.
    assert!(sidecar.has_live_child());
}

#[tokio::test]
async fn cooperative_cancel_during_ready_handshake_reaps_and_allows_next_request() {
    let fixture = fixture_model();
    let sidecar = Arc::new(LlmSidecar::for_test(config_with(
        "happy",
        &fixture,
        vec![("MOCK_READY_DELAY_MS".to_string(), "600".to_string())],
    )));

    let first_sidecar = Arc::clone(&sidecar);
    let first = tokio::spawn(async move {
        first_sidecar
            .transform(
                "Rewrite this politely.",
                "gimme the report",
                Duration::from_secs(5),
                CancelToken::new(),
            )
            .await
    });

    let mut spawned = false;
    for _ in 0..100 {
        if sidecar.resident_pid().is_some() {
            spawned = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(spawned, "helper never reached the Ready handshake");

    let cancel_started = std::time::Instant::now();
    sidecar.cancel_inflight_request();
    let err = tokio::time::timeout(Duration::from_secs(1), first)
        .await
        .expect("handshake cancellation did not settle promptly")
        .unwrap()
        .unwrap_err();
    assert_eq!(err, TransformError::Cancelled);
    assert!(
        cancel_started.elapsed() < Duration::from_secs(1),
        "busy did not clear promptly after handshake cancel: {:?}",
        cancel_started.elapsed()
    );
    assert!(!sidecar.is_transform_busy());
    assert!(
        !sidecar.has_live_child(),
        "partially started helper was not reaped"
    );

    let output = run_transform(&sidecar, Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(output.output, "mock-output");
}

#[tokio::test]
async fn dropping_the_transform_future_clears_busy_and_does_not_wedge() {
    let fixture = fixture_model();
    // Happy path with a delayed Result so the request is still in flight when
    // the future is dropped.
    let sidecar = Arc::new(LlmSidecar::for_test(config_with(
        "happy",
        &fixture,
        vec![("MOCK_DELAY_MS".to_string(), "400".to_string())],
    )));

    {
        // Drop the transform future well before the delayed Result arrives.
        let fut = run_transform(&sidecar, Duration::from_secs(5));
        let dropped = tokio::time::timeout(Duration::from_millis(50), fut).await;
        assert!(
            dropped.is_err(),
            "future should be cancelled by the timeout"
        );
    }

    // The blocking task keeps running and must clear `busy` when it finishes.
    let mut cleared = false;
    for _ in 0..100 {
        if !sidecar.is_transform_busy() {
            cleared = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(cleared, "busy flag wedged after the future was dropped");

    // And the supervisor is not bricked: a fresh transform proceeds.
    let out = run_transform(&sidecar, Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(out.output, "mock-output");
}

#[tokio::test]
async fn checksum_mismatch_model_refuses_to_spawn() {
    let fixture = fixture_model();
    let mut config = config_with("happy", &fixture, vec![]);
    // Corrupt the pinned hash: the pre-spawn verifier must fail closed.
    config.model_sha256 = "0".repeat(64);
    let sidecar = Arc::new(LlmSidecar::for_test(config));

    let err = run_transform(&sidecar, Duration::from_secs(5))
        .await
        .unwrap_err();
    assert_eq!(err, TransformError::ModelMismatch);
    // The helper was never launched.
    assert!(!sidecar.has_live_child());
}

#[tokio::test]
async fn a_stale_cancel_does_not_leak_into_the_next_request() {
    // Item 11: the cancel signal is a per-request token, not a supervisor-wide
    // flag. A request that has already finished can have its (now-stale) token
    // cancelled with NO effect on the next request, which carries a fresh
    // token and must run to completion. With the old shared `cancel_requested`
    // flag this class of leak/wipe was possible; per-request tokens rule it out.
    let fixture = fixture_model();
    let sidecar = sidecar("happy", &fixture);

    let token_n = CancelToken::new();
    let out = sidecar
        .transform("instr", "input", Duration::from_secs(5), token_n.clone())
        .await
        .unwrap();
    assert_eq!(out.output, "mock-output");

    // Cancel the finished request's token, and hit the supervisor entry point
    // (whose in-flight slot was cleared on completion, so it is a no-op now).
    token_n.cancel();
    sidecar.cancel_inflight_request();

    // The NEXT request has a fresh, un-cancelled token and must succeed.
    let token_next = CancelToken::new();
    let out2 = sidecar
        .transform("instr", "input", Duration::from_secs(5), token_next)
        .await
        .unwrap();
    assert_eq!(out2.output, "mock-output");
    assert!(sidecar.has_live_child());
}
