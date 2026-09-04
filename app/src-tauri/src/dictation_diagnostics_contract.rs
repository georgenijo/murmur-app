use crate::dictation_diagnostics::{
    DictationCaptureArmStatusV1, DictationCaptureCompletion, DictationDiagnostics,
};
use crate::dictation_telemetry::{DictationErrorCode, DictationTerminalOutcome};

#[test]
fn uninitialized_store_is_unarmed_and_refuses_explicit_arm() {
    let store = DictationDiagnostics::default();

    assert_eq!(store.arm_status(), DictationCaptureArmStatusV1::Unarmed);
    assert_eq!(
        store.arm_next().unwrap_err(),
        "dictation diagnostic capture store unavailable"
    );
}

#[test]
fn one_arm_is_claimed_by_exactly_one_live_recording() {
    let root = tempfile::tempdir().unwrap();
    let store = DictationDiagnostics::default();
    store.initialize(root.path().to_path_buf()).unwrap();

    let armed = store.arm_next().unwrap();
    assert!(matches!(
        armed,
        DictationCaptureArmStatusV1::Armed { .. }
    ));
    assert!(store.claim(41));
    assert!(!store.claim(42));
    assert_eq!(
        store.arm_status(),
        DictationCaptureArmStatusV1::Capturing { recording_id: 41 }
    );

    assert!(!store
        .finish(
            42,
            DictationCaptureCompletion::Terminal {
                outcome: DictationTerminalOutcome::Superseded,
                error_code: DictationErrorCode::StaleOwner,
            },
        )
        .unwrap());
    assert!(store
        .finish(
            41,
            DictationCaptureCompletion::Success {
                raw_text: "raw recognition",
                final_text: "final delivery",
                model_id: "parakeet-tdt-0.6b-v3-coreml",
                total_ms: 220,
            },
        )
        .unwrap());
    assert_eq!(store.arm_status(), DictationCaptureArmStatusV1::Unarmed);

    let captures = store.list_captures().unwrap();
    assert_eq!(captures.len(), 1);
    let capture = store.get_capture(&captures[0].capture_id).unwrap().unwrap();
    assert_eq!(capture.recording_id, 41);
    let content = capture.result.content().expect("successful capture has content");
    assert_eq!(content.raw_text.text, "raw recognition");
    assert_eq!(content.final_text.text, "final delivery");
    assert!(!content.raw_text.truncated);
    assert!(!content.final_text.truncated);
}

#[test]
fn unarmed_and_content_free_terminal_paths_never_persist_transcript_text() {
    let root = tempfile::tempdir().unwrap();
    let store = DictationDiagnostics::default();
    store.initialize(root.path().to_path_buf()).unwrap();

    assert!(!store.claim(7));
    assert!(!store
        .finish(
            7,
            DictationCaptureCompletion::Success {
                raw_text: "SENTINEL PRIVATE RAW",
                final_text: "SENTINEL PRIVATE FINAL",
                model_id: "parakeet-tdt-0.6b-v3-coreml",
                total_ms: 100,
            },
        )
        .unwrap());
    assert!(store.list_captures().unwrap().is_empty());

    store.arm_next().unwrap();
    assert!(store.claim(8));
    assert!(store
        .finish(
            8,
            DictationCaptureCompletion::Terminal {
                outcome: DictationTerminalOutcome::NoSpeech,
                error_code: DictationErrorCode::VadNoSpeech,
            },
        )
        .unwrap());

    let captures = store.list_captures().unwrap();
    assert_eq!(captures.len(), 1);
    let capture = store.get_capture(&captures[0].capture_id).unwrap().unwrap();
    assert!(capture.result.content().is_none());
    let encoded = serde_json::to_string(&capture).unwrap();
    assert!(!encoded.contains("SENTINEL"));
    assert!(!encoded.contains("rawText"));
    assert!(!encoded.contains("finalText"));
}

#[test]
fn private_text_is_utf8_safe_bounded_and_marked_when_truncated() {
    let root = tempfile::tempdir().unwrap();
    let store = DictationDiagnostics::default();
    store.initialize(root.path().to_path_buf()).unwrap();
    store.arm_next().unwrap();
    assert!(store.claim(99));

    let oversized = "é".repeat(5_000);
    store
        .finish(
            99,
            DictationCaptureCompletion::Success {
                raw_text: &oversized,
                final_text: &oversized,
                model_id: "parakeet-tdt-0.6b-v3-coreml",
                total_ms: 120,
            },
        )
        .unwrap();

    let summary = store.list_captures().unwrap().pop().unwrap();
    let capture = store.get_capture(&summary.capture_id).unwrap().unwrap();
    let content = capture.result.content().unwrap();
    assert!(content.raw_text.truncated);
    assert!(content.final_text.truncated);
    assert!(content.raw_text.text.len() <= 8 * 1024);
    assert!(content.final_text.text.len() <= 8 * 1024);
    assert!(content.raw_text.text.is_char_boundary(content.raw_text.text.len()));
    assert!(content.final_text.text.is_char_boundary(content.final_text.text.len()));
}
