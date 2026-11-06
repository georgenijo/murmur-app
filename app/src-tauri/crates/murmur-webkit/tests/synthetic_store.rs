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
    seed.insert(
        "dictation-stats".into(),
        serde_json::json!({"totalWords":10,"totalRecordings":2,"totalDurationSeconds":12})
            .to_string(),
    );
    let report = serde_json::json!({"createdAt":"2026-11-06T00:00:00Z","appVersion":"synthetic","platform":"synthetic","preset":"quick","iterations":1,"sharedInitMs":0,"results":[],"recommendations":{"fastest":null,"mostAccurate":null,"balanced":null}});
    seed.insert("murmur-benchmark-report".into(), report.to_string());
    seed.insert(
        "murmur-benchmark-reports".into(),
        serde_json::json!([report]).to_string(),
    );
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
    assert!(receipt.v3 && !receipt.retired);
    assert_eq!(
        receipt.values["murmur-appearance"]["theme"]["accent"],
        "#abcdef"
    );
    assert_eq!(service.execute(Command::Snapshot).unwrap().history.len(), 1);
    // Relaunch the permanent importer against the same synthetic WebKit store.
    // A corrupt v1 value would fail import if removal had not actually happened.
    let mut importer = Importer::start(
        mtm,
        || Ok(murmur_webkit::StoreIdentity::synthetic(&store)),
        service.clone(),
    )
    .unwrap()
    .unwrap();
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
    let inspected = importer.inspect_transcript_keys();
    loop {
        if let Ok(absent) = inspected.try_recv() {
            assert!(
                absent,
                "all four exact transcript keys must be absent after relaunch"
            );
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        NSRunLoop::currentRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.01));
    }
    drop(importer);
    // Actual JS evaluation error; navigation/termination delegate callbacks and
    // deadlines are injected (not an OS process kill or physical timeout).
    for failure in [
        murmur_webkit::SyntheticFailure::Navigation,
        murmur_webkit::SyntheticFailure::ProcessTermination,
        murmur_webkit::SyntheticFailure::Javascript,
        murmur_webkit::SyntheticFailure::Deadline,
    ] {
        let root = tempfile::tempdir().unwrap();
        let service = CompletionService::start(root.path().into(), true).unwrap();
        let store = unsafe { WKWebsiteDataStore::nonPersistentDataStore(mtm) };
        let mut importer = Importer::synthetic(mtm, &store, service.clone(), Default::default());
        importer.fail_after_import(failure);
        let failure_deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let status = importer.poll();
            if status == ImportStatus::PreservedForRetry {
                break;
            }
            assert_ne!(status, ImportStatus::Done);
            assert!(
                std::time::Instant::now() < failure_deadline,
                "transport failure must be observed promptly"
            );
            NSRunLoop::currentRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.01));
        }
        assert_eq!(importer.poll(), ImportStatus::PreservedForRetry);
        let receipt = service.lifecycle(Operation::Status).unwrap();
        assert!(receipt.v2);
        assert!(!receipt.transcript_keys_removed);
        // Rust-owned data can be presented after the WebKit transport fails.
        assert!(service.execute(Command::Snapshot).is_ok());
    }
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
    let mut retirement = unsafe {
        murmur_webkit::Retirement::new(
            murmur_webkit::StoreIdentity::synthetic(&store),
            directory.clone(),
            service.clone(),
        )
    };
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
    // API and filesystem phase failures must remain retryable across relaunch.
    for fault in [
        murmur_webkit::RetirementFailure::Purge,
        murmur_webkit::RetirementFailure::Verification,
        murmur_webkit::RetirementFailure::DirectoryRemoval,
        murmur_webkit::RetirementFailure::ParentSync,
    ] {
        let root = tempfile::tempdir().unwrap();
        let service = CompletionService::start(root.path().into(), false).unwrap();
        service.lifecycle(Operation::Status).unwrap();
        let path = root.path().join("migrations.json");
        let mut m: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        m["lifecycle"]["v3"] = serde_json::json!(true);
        std::fs::write(path, serde_json::to_vec(&m).unwrap()).unwrap();
        let directory = root.path().join("synthetic-webkit");
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(directory.join("a"), b"fixture").unwrap();
        std::fs::write(directory.join("b"), b"fixture").unwrap();
        let store = unsafe { WKWebsiteDataStore::nonPersistentDataStore(mtm) };
        let mut retirement = unsafe {
            murmur_webkit::Retirement::new(
                murmur_webkit::StoreIdentity::synthetic(&store),
                directory.clone(),
                service.clone(),
            )
        };
        retirement.inject_failure(fault);
        loop {
            match retirement.poll(mtm) {
                ImportStatus::PreservedForRetry => break,
                ImportStatus::Done => panic!("fault must preserve receipt"),
                _ => {}
            }
            assert!(std::time::Instant::now() < deadline);
            NSRunLoop::currentRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.01));
        }
        assert!(!service.lifecycle(Operation::Status).unwrap().retired);
        drop(retirement);
        drop(service);
        let service = CompletionService::start(root.path().into(), false).unwrap();
        let mut retirement = unsafe {
            murmur_webkit::Retirement::new(
                murmur_webkit::StoreIdentity::synthetic(&store),
                directory.clone(),
                service.clone(),
            )
        };
        loop {
            match retirement.poll(mtm) {
                ImportStatus::Done => break,
                ImportStatus::PreservedForRetry => panic!("retirement retry failed"),
                _ => {}
            }
            assert!(std::time::Instant::now() < deadline);
            NSRunLoop::currentRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.01));
        }
        assert!(!directory.exists());
        assert!(service.lifecycle(Operation::Status).unwrap().retired);
    }
    assert!(Importer::start(
        mtm,
        || panic!("retired startup must never create WebKit"),
        service.clone()
    )
    .unwrap()
    .is_none());
    println!("synthetic standalone WebKit import/removal/relaunch/retirement passed");
}
#[cfg(not(target_os = "macos"))]
fn main() {}
