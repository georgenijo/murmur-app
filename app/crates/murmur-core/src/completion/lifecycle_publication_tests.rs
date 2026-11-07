//! Synthetic host/core races. The pause is Storage's cfg(test)-only boundary,
//! after recovery/read/patch and immediately before receipt publication.
use super::*;
use lifecycle::{Operation, SOURCE_WRITE_LOCK};
use std::{sync::mpsc, time::Duration};

fn on_owner(service: &CompletionService, f: impl FnOnce(&mut Engine) + Send + 'static) {
    let (send, receive) = mpsc::channel();
    service
        .owner
        .send
        .as_ref()
        .unwrap()
        .send(Box::new(move |engine| {
            f(engine);
            send.send(()).unwrap();
        }))
        .unwrap();
    receive.recv_timeout(Duration::from_secs(5)).unwrap();
}

// Always unblock the owner before its service handle joins on a failed check.
struct ReleaseOnDrop(Option<mpsc::Sender<()>>);
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        if let Some(release) = self.0.take() {
            let _ = release.send(());
        }
    }
}

fn disk(root: &std::path::Path) -> Value {
    serde_json::from_slice(&std::fs::read(root.join("settings.json")).unwrap()).unwrap()
}

enum Retry {
    Command,
    Producer,
    Shutdown,
}

fn publication_race(auto_paste: bool, settings_save: bool, trigger: Retry) {
    let root = tempfile::tempdir().unwrap();
    let service = CompletionService::start(root.path().into(), true).unwrap();
    service.execute(Command::Snapshot).unwrap();
    {
        let _guard = SOURCE_WRITE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        save_source(
            root.path(),
            "dictation-settings",
            &json!({"settingsVersion":7,"disabled":true,"autoPaste":false,
                "ownedFieldsRevision":0,"language":"en","launchOnboardingComplete":false})
            .to_string(),
        )
        .unwrap();
        let mut value = disk(root.path());
        value["disabled"] = json!(false);
        value["autoPaste"] = json!(auto_paste);
        value["ownedFieldsRevision"] = json!(1);
        let mut storage = Storage::new(root.path().into());
        storage.fail = Some(("settings.json".into(), storage::Boundary::BeforeRename));
        assert!(publish_source(&mut storage, "dictation-settings", &value).is_err());
        assert!(root.path().join("settings.json.pending").exists());
        assert_eq!(disk(root.path())["ownedFieldsRevision"], 0);
    }
    // Retain the onboarding save before it can recover settings. This is the
    // same FIFO retried by unrelated commands, producers and final shutdown.
    let ui = root.path().join("ui-state.json");
    std::fs::write(&ui, r#"{"schema":999}"#).unwrap();
    assert!(service
        .lifecycle(Operation::Save {
            key: "murmur_onboarding_complete".into(),
            value: json!(true),
        })
        .is_err());
    assert!(root.path().join("settings.json.pending").exists());
    std::fs::remove_file(ui).unwrap();

    let (paused, pause) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let release = ReleaseOnDrop(Some(release));
    on_owner(&service, move |engine| {
        engine.storage.pause = Some((
            "settings.json.pending".into(),
            storage::Boundary::BeforeRename,
            paused,
            resume,
        ));
    });
    let retry = service.clone();
    let (finished, finish) = mpsc::channel();
    let retry_worker = std::thread::spawn(move || {
        match trigger {
            Retry::Shutdown => assert_eq!(
                futures_executor::block_on(retry.shutdown_receipt()),
                crate::shutdown::WriteOutcome::Committed
            ),
            Retry::Command => {
                retry.execute(Command::Snapshot).unwrap();
            }
            Retry::Producer => {
                let producers = producers::Producers::new(retry, "synthetic-session".into());
                for _ in 0..2 {
                    let snapshot = producers.dictation(1,
                        json!({"schemaVersion":2,"id":"synthetic","text":"synthetic", "duration":1.,"source":"recording","timestamp":1,"pinned":false}),
                        true, None, "2026-11-06".into()).unwrap();
                    assert_eq!(snapshot.history.len(), 1);
                    assert_eq!(snapshot.statistics.total_recordings, 1);
                }
            }
        }
        finished.send(()).unwrap();
    });
    pause.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(disk(root.path())["ownedFieldsRevision"], 1);
    assert_eq!(disk(root.path())["disabled"], false);
    assert_eq!(disk(root.path())["autoPaste"], auto_paste);

    let host_root = root.path().to_path_buf();
    let (attempted, attempt) = mpsc::channel();
    let (committed, commit) = mpsc::channel();
    let host = std::thread::spawn(move || {
        // Deterministically distinguish the fixed serialization from the old
        // interleaving. With the defect, let the host commit before resuming.
        let immediate = SOURCE_WRITE_LOCK.try_lock().ok();
        attempted.send(immediate.is_some()).unwrap();
        let _guard = immediate
            .unwrap_or_else(|| SOURCE_WRITE_LOCK.lock().unwrap_or_else(|p| p.into_inner()));
        recover_source(&host_root, "dictation-settings").unwrap();
        let mut current = disk(&host_root);
        current["disabled"] = json!(true);
        current["autoPaste"] = json!(false);
        current["ownedFieldsRevision"] =
            json!(current["ownedFieldsRevision"].as_u64().unwrap() + 1);
        if settings_save {
            current["language"] = json!("fr");
        }
        save_source(&host_root, "dictation-settings", &current.to_string()).unwrap();
        committed.send(()).unwrap();
    });
    let bypassed_fence = attempt.recv_timeout(Duration::from_secs(5)).unwrap();
    if bypassed_fence {
        commit.recv_timeout(Duration::from_secs(5)).unwrap();
    }
    drop(release);
    finish.recv_timeout(Duration::from_secs(5)).unwrap();
    retry_worker.join().unwrap();
    host.join().unwrap();
    let value = disk(root.path());
    assert_eq!(value["disabled"], true, "stale retry undid tray disable");
    assert_eq!(
        value["autoPaste"], false,
        "stale retry undid Auto Paste edit"
    );
    assert_eq!(value["ownedFieldsRevision"], 2, "revision went backwards");
    assert_eq!(value["language"], if settings_save { "fr" } else { "en" });
    assert_eq!(value["launchOnboardingComplete"], true);
    assert!(!bypassed_fence, "core published outside host serialization");
    assert!(!root.path().join("settings.json.pending").exists());
}

#[test]
fn retained_lifecycle_retry_serializes_with_tray_disable() {
    publication_race(false, false, Retry::Command);
}
#[test]
fn retained_lifecycle_retry_preserves_newer_auto_paste() {
    publication_race(true, false, Retry::Command);
}
#[test]
fn retained_lifecycle_retry_serializes_with_settings_save() {
    publication_race(true, true, Retry::Command);
}
#[test]
fn shutdown_recovery_serializes_retained_lifecycle_publication() {
    publication_race(true, true, Retry::Shutdown);
}

#[test]
fn native_producer_retries_lifecycle_without_losing_or_duplicating_completion() {
    publication_race(true, true, Retry::Producer);
}

#[test]
fn lifecycle_rejects_file_snapshots_before_retaining_them() {
    let root = tempfile::tempdir().unwrap();
    let service = CompletionService::start(root.path().into(), true).unwrap();
    for (key, value) in [
        ("dictation-settings", json!({"disabled":false})),
        (
            "murmur-theme-library",
            json!({"version":1,"revision":0,"themes":[]}),
        ),
    ] {
        assert!(matches!(
            service.lifecycle(Operation::Save {
                key: key.into(),
                value
            }),
            Err(Error::Invalid)
        ));
    }
    on_owner(&service, |engine| assert!(engine.accepted.is_empty()));
}
