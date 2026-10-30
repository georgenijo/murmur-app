use super::*;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::Layer;

#[test]
fn nonempty_preview_evidence_is_bound_to_recording_and_cannot_regress() {
    let state = AppState::default();
    state.recording_id.store(41, Ordering::SeqCst);
    state.dictation.lock_or_recover().status = DictationStatus::Recording;
    note_nonempty_dictation_preview(&state, 41);
    assert_eq!(
        state.dictation_nonempty_preview_id.load(Ordering::SeqCst),
        41
    );
    state.dictation.lock_or_recover().status = DictationStatus::Processing;
    state.recording_id.store(42, Ordering::SeqCst);
    note_nonempty_dictation_preview(&state, 42); // Stop before completion.
    assert_eq!(
        state.dictation_nonempty_preview_id.load(Ordering::SeqCst),
        41
    );
    state.dictation.lock_or_recover().status = DictationStatus::Recording;
    note_nonempty_dictation_preview(&state, 42);
    note_nonempty_dictation_preview(&state, 41); // Late old completion.
    assert_eq!(
        state.dictation_nonempty_preview_id.load(Ordering::SeqCst),
        42
    );
}

#[test]
fn vad_disagreement_uses_only_matching_nonempty_preview_evidence() {
    #[derive(Clone)]
    struct Events(Arc<Mutex<Vec<String>>>);
    impl<S: tracing::Subscriber> Layer<S> for Events {
        fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
            self.0.lock().unwrap().push(format!("{event:?}"));
        }
    }
    let events = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::Registry::default().with(Events(events.clone()));
    let state = AppState::default();
    tracing::subscriber::with_default(subscriber, || {
        emit_preview_vad_disagreement(&state, 0, 100);
        emit_preview_vad_disagreement(&state, 41, 100);
        assert!(events.lock().unwrap().is_empty());
        state
            .dictation_nonempty_preview_id
            .store(41, Ordering::SeqCst);
        emit_preview_vad_disagreement(&state, 42, 100);
        assert!(events.lock().unwrap().is_empty());
        emit_preview_vad_disagreement(&state, 41, 100);
    });
    let captured = events.lock().unwrap();
    assert_eq!(captured.len(), 1);
    assert!(captured[0].contains("pipeline.preview_vad_disagreement"));
    assert!(captured[0].contains("recording_id"));
    assert!(captured[0].contains("sample_count"));
}

#[test]
fn nonblocking_preview_ownership_recovers_status_poison_and_keeps_recording_fences() {
    let state = AppState::default();
    state.recording_id.store(41, Ordering::SeqCst);
    state.dictation.lock_or_recover().status = DictationStatus::Recording;
    let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _status = state.dictation.lock_or_recover();
        panic!("intentional dictation-status poison");
    }));
    assert!(poisoned.is_err() && state.dictation.is_poisoned());
    assert!(dictation_partial_is_current_if_uncontended(&state, 41));
    assert!(!dictation_partial_is_current_if_uncontended(&state, 0));
    state.recording_id.store(42, Ordering::SeqCst);
    assert!(!dictation_partial_is_current_if_uncontended(&state, 41));
    assert!(dictation_partial_is_current_if_uncontended(&state, 42));
    state.cancel_recording(42);
    assert!(!dictation_partial_is_current_if_uncontended(&state, 42));
    state.recording_id.store(43, Ordering::SeqCst);
    state.dictation.lock_or_recover().status = DictationStatus::Processing;
    assert!(!dictation_partial_is_current_if_uncontended(&state, 43));
    assert!(
        state.dictation.is_poisoned(),
        "recovery must not depend on clearing poison"
    );
}

#[test]
#[cfg(not(feature = "preview-decode-reuse"))]
fn disabled_reuse_passes_identical_pcm_settings_and_output_to_whole_clip_decoder() {
    struct ExactDecoder {
        calls: usize,
        fail_first: bool,
    }
    impl transcriber::TranscriptionBackend for ExactDecoder {
        fn name(&self) -> &str {
            "exact"
        }
        fn load_model(&mut self, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn is_model_loaded(&self, _: &str) -> bool {
            true
        }
        fn transcribe(
            &mut self,
            samples: &[f32],
            language: &str,
            prompt: Option<&str>,
            punctuation: bool,
        ) -> Result<String, String> {
            assert_eq!(language, "en");
            assert_eq!(prompt, Some("Synthetic vocabulary"));
            assert!(!punctuation);
            let expected = if self.calls == 0 {
                vec![0.25f32, -0.0]
            } else {
                vec![0.0f32, 0.25, -0.0, -0.5]
            };
            assert_eq!(
                samples.iter().map(|s| s.to_bits()).collect::<Vec<_>>(),
                expected.iter().map(|s| s.to_bits()).collect::<Vec<_>>()
            );
            self.calls += 1;
            Ok(if self.fail_first && self.calls == 1 {
                ""
            } else {
                "  Exact UTF-8 café—bytes.\n"
            }
            .into())
        }
        fn token_count(&self, _: &str) -> Option<usize> {
            None
        }
        fn model_exists(&self) -> bool {
            true
        }
        fn models_dir(&self) -> Result<std::path::PathBuf, String> {
            Ok(std::env::temp_dir())
        }
        fn reset(&mut self) {}
    }
    let settings = dictation_context::TranscriptionSettings {
        model_name: transcriber::COREML_MODEL_NAME.into(),
        language: "en".into(),
        vad_sensitivity: 2,
        prompt: Some("Synthetic vocabulary".into()),
        smart_punctuation: false,
    };
    for fail_first in [false, true] {
        let mut baseline = ExactDecoder {
            calls: 0,
            fail_first,
        };
        let mut candidate = ExactDecoder {
            calls: 0,
            fail_first,
        };
        let filtered = [0.25, -0.0];
        let original = [0.0, 0.25, -0.0, -0.5];
        let old = transcribe_with_coreml_vad_retry_report(
            &mut baseline,
            &settings.model_name,
            &filtered,
            &original,
            true,
            &settings.language,
            settings.prompt.as_deref(),
            false,
        )
        .unwrap();
        let new =
            decode_final_transcript(&mut candidate, &settings, &filtered, &original, true).unwrap();
        assert_eq!(new.text.as_bytes(), old.text.as_bytes());
        assert_eq!(new.empty_error, old.empty_error);
        assert_eq!(candidate.calls, baseline.calls);
    }
}

/// Local-only probe: inputs and output JSON live outside the repository. The
/// clock covers final ASR after replay, not unavailable native hotkey/capture.
#[test]
#[ignore = "requires local Core ML models and synthetic WAV manifest"]
#[cfg(all(
    feature = "preview-decode-reuse",
    target_os = "macos",
    target_arch = "aarch64"
))]
fn synthetic_preview_reuse_probe() {
    use transcriber::TranscriptionBackend;
    let manifest_path =
        std::env::var("MURMUR_REUSE_SYNTHETIC_MANIFEST").expect("synthetic manifest path");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(manifest_path).unwrap()).unwrap();
    let output = std::env::var("MURMUR_REUSE_SYNTHETIC_OUTPUT").expect("private output path");
    let mut backend = transcriber::CoreMlBackend::new();
    backend.load_model(transcriber::COREML_MODEL_NAME).unwrap();
    let settings = dictation_context::TranscriptionSettings {
        model_name: transcriber::COREML_MODEL_NAME.into(),
        language: "auto".into(),
        vad_sensitivity: 0,
        prompt: None,
        smart_punctuation: true,
    };
    let mut rows = Vec::new();
    for fixture in manifest.as_array().unwrap() {
        let samples = transcriber::parse_wav_to_samples(
            &std::fs::read(fixture["wav"].as_str().unwrap()).unwrap(),
        )
        .unwrap();
        // Warm the exact model and input before measuring either arm.
        backend.transcribe(&samples, "auto", None, true).unwrap();
        for iteration in 0..3 {
            let mut reuse = crate::preview_reuse::PreviewReuse::default();
            reuse.begin(1);
            for end in (11_200..samples.len()).step_by(11_200) {
                if let Some(range) = reuse.plan(1, &samples[..end]) {
                    match backend.transcribe_preview(
                        &samples[range.clone()],
                        "auto",
                        None,
                        true,
                        transcriber::PreviewPass::SyntheticDiagnostic,
                    ) {
                        Ok(text) => reuse.commit(1, range, &samples[..end], &text),
                        Err(_) => reuse.fail(1),
                    }
                }
                // Same trailing-window UI work as the native ticker.
                backend
                    .transcribe_preview(
                        dictation_partial_window(&samples[..end]),
                        "auto",
                        None,
                        true,
                        transcriber::PreviewPass::SyntheticDiagnostic,
                    )
                    .unwrap();
            }
            let plan = reuse.take_final(1, &samples);
            let tail_samples = plan
                .as_ref()
                .map(|p| samples.len() - p.start)
                .unwrap_or(samples.len());
            let mut off = None;
            let mut on = None;
            // Alternate arm order to reduce warm/order bias.
            for enabled in if iteration % 2 == 0 {
                [false, true]
            } else {
                [true, false]
            } {
                let start = std::time::Instant::now();
                let result = decode_final_transcript(
                    &mut backend,
                    &settings,
                    &samples,
                    &samples,
                    false,
                    if enabled { plan.as_ref() } else { None },
                )
                .unwrap();
                let value = serde_json::json!({"ms": start.elapsed().as_secs_f64() * 1000.0, "text": result.text});
                if enabled {
                    on = Some(value);
                } else {
                    off = Some(value);
                }
            }
            // Independent replay outside the measured interval checks overlap
            // agreement; it does not instrument the measured call's branch.
            let replayed_overlap_accepted = plan.as_ref().is_some_and(|plan| {
                backend
                    .transcribe(&samples[plan.start..], "auto", None, true)
                    .ok()
                    .and_then(|tail| plan.stitch(&tail))
                    .is_some()
            });
            rows.push(serde_json::json!({"replayed_overlap_accepted": replayed_overlap_accepted, "id": fixture["id"], "reference": fixture["reference"], "duration_s": samples.len() as f64 / 16000.0,
                "iteration": iteration, "planned_tail_samples": tail_samples, "off": off, "on": on}));
        }
    }
    std::fs::write(output, serde_json::to_vec_pretty(&rows).unwrap()).unwrap();
}
