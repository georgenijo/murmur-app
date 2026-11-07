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
