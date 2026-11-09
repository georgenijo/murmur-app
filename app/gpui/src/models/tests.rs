use super::{fixtures, model::*};
#[test]
fn storage_admission_matches_react() {
    let mut s = fixtures::snapshot();
    let base = s.model("base.en").unwrap().clone();
    assert!(s.can_remove(&base));
    s.selected = base.id.clone();
    assert!(!s.can_remove(&base));
    s.selected = "tiny.en".into();
    let mut active = base.clone();
    active.lifecycle = Lifecycle::Ready;
    assert!(!s.can_remove(&active));
    s.busy = true;
    assert!(!s.can_download(s.model("small.en").unwrap()));
    assert!(!s.can_remove(&base));
}
#[test]
fn command_contract() {
    use super::host::Action;
    assert_eq!(
        Action::Download("tiny.en".into()).command().unwrap(),
        ("download_model", serde_json::json!({"modelName":"tiny.en"}))
    );
    assert!(!known("bogus"));
}
#[test]
fn late_runtime_updates_do_not_regress_catalog() {
    let mut s = fixtures::snapshot();
    let mut update = s.model("base.en").unwrap().clone();
    update.generation = 20;
    update.install = Install::Invalid;
    assert!(s.merge_runtime(update.clone()));
    update.generation = 19;
    update.install = Install::Installed;
    assert!(!s.merge_runtime(update));
    assert_eq!(s.model("base.en").unwrap().install, Install::Invalid);
}

#[test]
fn selection_uses_settings_revision_even_after_runtime_events() {
    let mut snapshot = fixtures::snapshot();
    snapshot.revision = 90;
    snapshot.settings_revision = 7;
    assert_eq!(
        snapshot.select("tiny.en".into()),
        super::host::Action::Select {
            id: "tiny.en".into(),
            settings_revision: 7
        }
    );
    snapshot.revision += 1;
    assert_eq!(
        snapshot.select("tiny.en".into()),
        super::host::Action::Select {
            id: "tiny.en".into(),
            settings_revision: 7
        }
    );
}

#[test]
fn failed_download_leaves_the_model_retryable_with_its_explanation() {
    use crate::workspace::Projection;
    let mut s = fixtures::snapshot();
    let id = "parakeet-tdt-0.6b-v3-coreml".to_string();
    // The service marks a failed install Invalid and the attempt is over.
    s.catalog.iter_mut().find(|m| m.id == id).unwrap().install = Install::Invalid;
    s.failure = Some(Failure {
        model: id.clone(),
        operation: Operation::Download,
        message: "Core ML setup did not finish. Retry Core ML, or choose Whisper Base or the CPU Parakeet fallback.",
        fallbacks: true,
    });
    assert!(s.valid());
    assert!(s.can_download(s.model(&id).unwrap()));
    // An unknown model is never accepted.
    s.failure.as_mut().unwrap().model = "bogus".into();
    assert!(!s.valid());
}

#[test]
fn english_only_without_a_runtime_row_matches_shipped() {
    let mut s = fixtures::snapshot();
    s.catalog.clear();
    assert!(s.english_only());
    let mut s = fixtures::snapshot();
    let selected = s.selected.clone();
    s.catalog
        .iter_mut()
        .find(|m| m.id == selected)
        .unwrap()
        .capabilities = vec!["multilingual".into()];
    assert!(!s.english_only());
}
