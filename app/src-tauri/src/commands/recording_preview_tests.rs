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
