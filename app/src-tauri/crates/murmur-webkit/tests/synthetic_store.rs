#[cfg(target_os = "macos")]
fn main() {
    use murmur_core::completion::{lifecycle::Operation, Command, CompletionService};
    use murmur_webkit::{ImportStatus, Importer};
    use objc2::MainThreadMarker;
    use objc2_foundation::{NSDate, NSRunLoop};
    use objc2_web_kit::WKWebsiteDataStore;
    let mtm = MainThreadMarker::new().expect("test owns main thread");
    let root = tempfile::tempdir().unwrap();
    let service = CompletionService::start(root.path().into(), true).unwrap();
    // Never obtain the default store. This custom store is memory-only; all
    // durable fixtures and destination files are inside this temp root.
    let store = unsafe { WKWebsiteDataStore::nonPersistentDataStore(mtm) };
    let mut seed = std::collections::BTreeMap::new();
    seed.insert("dictation-history".into(),serde_json::json!([{"schemaVersion":2,"id":"synthetic","text":"Synthetic fixture only","duration":1,"source":"recording","timestamp":1,"pinned":false}]).to_string());
    seed.insert("murmur-appearance".into(),serde_json::json!({"version":1,"mode":"dark","theme":{"version":1,"presetId":"custom","accent":"#abcdef"},"revision":1}).to_string());
    let mut importer = Importer::synthetic(mtm, &store, service.clone(), seed);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(45);
    loop {
        match importer.poll() {
            ImportStatus::Done => break,
            ImportStatus::PreservedForRetry => panic!("synthetic import failed; no content logged"),
            _ => {}
        }
        assert!(
            std::time::Instant::now() < deadline,
            "synthetic importer timed out"
        );
        NSRunLoop::currentRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.01));
    }
    drop(importer);
    let receipt = service.lifecycle(Operation::Status).unwrap();
    assert!(receipt.v1 && receipt.v2 && receipt.transcript_keys_removed);
    assert!(!receipt.v3 && !receipt.retired);
    assert_eq!(
        receipt.values["murmur-appearance"]["theme"]["accent"],
        "#abcdef"
    );
    assert_eq!(service.execute(Command::Snapshot).unwrap().history.len(), 1);
    // Relaunch the permanent importer against the same synthetic WebKit store.
    // A corrupt v1 value would fail import if removal had not actually happened.
    let mut importer = Importer::new(mtm, &store, service.clone());
    loop {
        match importer.poll() {
            ImportStatus::Done => break,
            ImportStatus::PreservedForRetry => panic!("synthetic relaunch failed"),
            _ => {}
        }
        assert!(
            std::time::Instant::now() < deadline,
            "synthetic relaunch timed out"
        );
        NSRunLoop::currentRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.01));
    }
    assert_eq!(service.execute(Command::Snapshot).unwrap().history.len(), 1);
    drop(importer);
    // Model the future #990 updater receipt inside this synthetic root only.
    let path = root.path().join("migrations.json");
    let mut migration: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    migration["lifecycle"]["v3"] = serde_json::json!(true);
    std::fs::write(path, serde_json::to_vec(&migration).unwrap()).unwrap();
    let directory = root.path().join("synthetic-webkit");
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(directory.join("synthetic-cache"), b"fixture").unwrap();
    // All webviews have been dropped; the synthetic custom store has no user data.
    let mut retirement =
        unsafe { murmur_webkit::Retirement::new(store, directory.clone(), service.clone()) };
    loop {
        match retirement.poll(mtm) {
            ImportStatus::Done => break,
            ImportStatus::PreservedForRetry => panic!("synthetic retirement failed"),
            _ => {}
        }
        assert!(
            std::time::Instant::now() < deadline,
            "synthetic retirement timed out"
        );
        NSRunLoop::currentRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.01));
    }
    assert!(!directory.exists());
    assert!(service.lifecycle(Operation::Status).unwrap().retired);
    println!("synthetic standalone WebKit import/removal/relaunch/retirement passed");
}
#[cfg(not(target_os = "macos"))]
fn main() {}
