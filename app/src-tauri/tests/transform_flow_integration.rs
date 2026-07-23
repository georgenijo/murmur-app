//! Integration test for the transform-flow orchestrator (#312, PR-C2).
//!
//! Extends the mock-helper suite (`llm_sidecar_integration.rs`): it drives the
//! full `start_capture -> finish -> ready` happy path with a MOCKED selection
//! provider (a fake `TransformSnapshot` injected via the flow's async seam — the
//! production AX capture in `selection.rs` is never weakened) against the real
//! supervisor talking to the protocol-v1 mock helper. No AX server, no Tauri
//! app, no real GGUF model.

#![cfg(all(target_os = "macos", target_arch = "aarch64"))]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ui_lib::llm_sidecar::{model_file_digest, LlmSidecar, TestSpawnConfig};
use ui_lib::transform_flow::run_happy_path_for_test;

fn helper_path() -> PathBuf {
    std::env::current_exe()
        .expect("integration test executable path")
        .parent()
        .and_then(|deps| deps.parent())
        .expect("Cargo target profile directory")
        .join("examples/mock_llm_helper")
}

struct Fixture {
    _dir: tempfile::TempDir,
    path: PathBuf,
    size: u64,
    sha: String,
}

fn fixture_model() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fixture-model.gguf");
    std::fs::write(&path, b"murmur transform-flow fixture model bytes").unwrap();
    let (size, sha) = model_file_digest(&path).unwrap();
    Fixture {
        _dir: dir,
        path,
        size,
        sha,
    }
}

fn happy_sidecar(fixture: &Fixture) -> Arc<LlmSidecar> {
    Arc::new(LlmSidecar::for_test(TestSpawnConfig {
        helper_path: helper_path(),
        model_path: fixture.path.clone(),
        model_size: fixture.size,
        model_sha256: fixture.sha.clone(),
        scenario_env: vec![("MOCK_SCENARIO".to_string(), "happy".to_string())],
        request_slack: Duration::from_millis(100),
        cancel_grace: Duration::from_millis(150),
        handshake_timeout: Duration::from_secs(5),
        idle_after: Duration::from_secs(300),
    }))
}

#[tokio::test]
async fn happy_path_capture_finish_ready_produces_proposal() {
    let fixture = fixture_model();
    let sidecar = happy_sidecar(&fixture);

    let report = run_happy_path_for_test(
        &sidecar,
        "Rewrite this politely.",
        "gimme the report now",
    )
    .await;

    // The flow emitted exactly the forward sequence — never an error state.
    assert_eq!(
        report.emitted_states,
        vec![
            "listening".to_string(),
            "thinking".to_string(),
            "ready".to_string()
        ],
        "expected listening -> thinking -> ready",
    );
    // The proposal from the mock helper landed on the session.
    assert_eq!(report.proposed.as_deref(), Some("mock-output"));
    // The instruction was frozen onto the session for the review popover.
    assert_eq!(report.instruction.as_deref(), Some("Rewrite this politely."));
    // The listening popover was shown.
    assert!(report.popover_shown);
    // Healthy helper is kept resident for reuse.
    assert!(sidecar.has_live_child());
}
