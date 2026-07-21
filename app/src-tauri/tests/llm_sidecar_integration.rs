//! Integration tests for the local-LLM sidecar supervisor (#312).
//!
//! These drive the real supervisor against the protocol-v1 mock helper
//! (`CARGO_BIN_EXE_mock_llm_helper`). No real GGUF model or llama.cpp is
//! involved: the "model" is a small temp file whose pins the test derives.

#![cfg(all(target_os = "macos", target_arch = "aarch64"))]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ui_lib::llm_sidecar::{
    model_file_digest, LlmSidecar, TestSpawnConfig, TransformError, TransformOutput,
};

fn helper_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_mock_llm_helper"))
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
    sidecar
        .transform("Rewrite this politely.", "gimme the report", deadline)
        .await
}

#[tokio::test]
async fn successful_transform_round_trip() {
    let fixture = fixture_model();
    let sidecar = sidecar("happy", &fixture);
    let out = run_transform(&sidecar, Duration::from_secs(5)).await.unwrap();
    assert_eq!(out.output, "mock-output");
    assert_eq!(out.output_tokens, 3);
    // The helper is healthy and kept resident for reuse.
    assert!(sidecar.has_live_child());
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
        assert!(dropped.is_err(), "future should be cancelled by the timeout");
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
    let out = run_transform(&sidecar, Duration::from_secs(5)).await.unwrap();
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
