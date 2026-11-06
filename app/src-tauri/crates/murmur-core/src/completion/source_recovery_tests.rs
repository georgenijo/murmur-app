use super::*;

fn document(key: &str, revision: u64) -> Value {
    crate::state::normalize(key, if key == "dictation-settings" {
        json!({"language":if revision==1 {"es"} else {"ja"},"ownedFieldsRevision":revision,"microphoneIdMigrationComplete":true})
    } else { json!({"version":1,"revision":revision,"themes":[]}) }).unwrap()
}

#[test]
fn corrupt_receipt_beside_valid_destination_preserves_startup() {
    for key in ["dictation-settings", "murmur-theme-library"] {
        let dir = tempfile::tempdir().unwrap();
        let file = crate::state::file(key).unwrap();
        let value = document(key, 2);
        save_source(dir.path(), key, &value.to_string()).unwrap();
        std::fs::write(dir.path().join(format!("{file}.pending")), b"broken").unwrap();
        for _ in 0..3 {
            recover_source(dir.path(), key).unwrap();
            let disk: Value =
                serde_json::from_slice(&std::fs::read(dir.path().join(file)).unwrap()).unwrap();
            assert_eq!(disk, value);
        }
        assert!(Storage::new(dir.path().into())
            .quarantined(&format!("{file}.pending"))
            .unwrap());
    }
}

#[test]
fn stale_receipts_cannot_replace_newer_destinations() {
    for key in ["dictation-settings", "murmur-theme-library"] {
        let dir = tempfile::tempdir().unwrap();
        let file = crate::state::file(key).unwrap();
        let old = document(key, 1);
        let new = document(key, 2);
        save_source(dir.path(), key, &old.to_string()).unwrap();
        // Capture an authentic interrupted transition, then publish a newer document.
        let mut storage = Storage::new(dir.path().into());
        storage.fail = Some((file.into(), storage::Boundary::BeforeRename));
        assert!(publish_source(&mut storage, key, &old).is_err());
        let receipt = std::fs::read(dir.path().join(format!("{file}.pending"))).unwrap();
        recover_source(dir.path(), key).unwrap();
        save_source(dir.path(), key, &new.to_string()).unwrap();
        std::fs::write(dir.path().join(format!("{file}.pending")), receipt).unwrap();
        assert!(recover_source(dir.path(), key).is_err());
        assert_eq!(
            serde_json::from_slice::<Value>(&std::fs::read(dir.path().join(file)).unwrap())
                .unwrap(),
            new
        );
    }
}

#[test]
fn stale_same_store_receipt_cannot_replay_after_newer_destination_is_lost() {
    for key in ["dictation-settings", "murmur-theme-library"] {
        let dir = tempfile::tempdir().unwrap();
        let file = crate::state::file(key).unwrap();
        let old = document(key, 1);
        let new = document(key, 2);
        let mut storage = Storage::new(dir.path().into());
        storage.fail = Some((file.into(), storage::Boundary::BeforeRename));
        assert!(publish_source(&mut storage, key, &old).is_err());
        let pending = std::fs::read(dir.path().join(format!("{file}.pending"))).unwrap();
        recover_source(dir.path(), key).unwrap();
        save_source(dir.path(), key, &new.to_string()).unwrap();
        std::fs::remove_file(dir.path().join(file)).unwrap();
        std::fs::write(dir.path().join(format!("{file}.pending")), pending).unwrap();
        assert!(recover_source(dir.path(), key).is_err());
        assert!(!dir.path().join(file).exists());
        restore_source(dir.path(), key, &new).unwrap();
        recover_source(dir.path(), key).unwrap();
    }
}

#[test]
fn selected_backup_resolves_failed_generation_at_every_publish_boundary() {
    use storage::Boundary;
    for key in ["dictation-settings", "murmur-theme-library"] {
        let file = crate::state::file(key).unwrap();
        for corrupt_destination in [false, true] {
            for step in [
                "required",
                "identity",
                "pending",
                "publish-required",
                "destination",
            ] {
                for boundary in [
                    Boundary::BeforeRename,
                    Boundary::AfterRename,
                    Boundary::DirectorySync,
                ] {
                    let dir = tempfile::tempdir().unwrap();
                    let backup = document(key, 2);
                    std::fs::write(dir.path().join(format!("{file}.required")), b"true").unwrap();
                    std::fs::write(
                        dir.path().join(format!("{file}.pending")),
                        b"broken receipt",
                    )
                    .unwrap();
                    if corrupt_destination {
                        std::fs::write(dir.path().join(file), b"broken destination").unwrap();
                    }
                    assert!(recover_source(dir.path(), key).is_err());
                    let mut storage = Storage::new(dir.path().into());
                    if step == "publish-required" {
                        storage
                            .faults
                            .push_back((format!("{file}.required"), boundary, 1));
                    } else {
                        let name = if step == "destination" {
                            file.into()
                        } else {
                            format!("{file}.{step}")
                        };
                        storage.fail = Some((name, boundary));
                    }
                    assert!(
                        restore_source_with_storage(&mut storage, key, &backup).is_err(),
                        "{key} {step} {boundary:?}"
                    );
                    // Simulated process restart: construct a fresh storage/recovery owner.
                    let committed = step == "destination"
                        || step == "publish-required"
                        || (step == "pending" && boundary != Boundary::BeforeRename);
                    if committed {
                        recover_source(dir.path(), key).unwrap();
                    } else {
                        assert!(recover_source(dir.path(), key).is_err());
                        restore_source(dir.path(), key, &backup).unwrap();
                    }
                    for _ in 0..2 {
                        recover_source(dir.path(), key).unwrap();
                        assert_eq!(
                            serde_json::from_slice::<Value>(
                                &std::fs::read(dir.path().join(file)).unwrap()
                            )
                            .unwrap(),
                            backup
                        );
                    }
                    assert!(storage.quarantined(&format!("{file}.pending")).unwrap());
                    let service = CompletionService::start(dir.path().into(), true).unwrap();
                    assert_eq!(
                        service
                            .lifecycle(lifecycle::Operation::Status)
                            .unwrap()
                            .values[key],
                        backup
                    );
                    drop(service);
                    // Explicit restore cannot replace a readable newer destination.
                    assert!(restore_source(dir.path(), key, &document(key, 1)).is_err());
                }
            }
        }
    }
}

#[test]
fn foreign_receipts_and_receipt_permissions() {
    for key in ["dictation-settings", "murmur-theme-library"] {
        let file = crate::state::file(key).unwrap();
        for destination_exists in [false, true] {
            let foreign = tempfile::tempdir().unwrap();
            let local = tempfile::tempdir().unwrap();
            let old = document(key, 1);
            let new = document(key, 2);
            save_source(foreign.path(), key, &old.to_string()).unwrap();
            save_source(local.path(), key, &new.to_string()).unwrap();
            let mut storage = Storage::new(foreign.path().into());
            storage.fail = Some((file.into(), storage::Boundary::BeforeRename));
            assert!(publish_source(&mut storage, key, &old).is_err());
            let pending = foreign.path().join(format!("{file}.pending"));
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(
                    std::fs::metadata(&pending).unwrap().permissions().mode() & 0o777,
                    0o600
                );
            }
            if !destination_exists {
                std::fs::remove_file(local.path().join(file)).unwrap();
            }
            std::fs::copy(pending, local.path().join(format!("{file}.pending"))).unwrap();
            assert!(recover_source(local.path(), key).is_err());
            // A structurally valid receipt for the other source is foreign too,
            // rather than ordinary malformed input silently ignored at startup.
            let mut receipt: Value = serde_json::from_slice(
                &std::fs::read(foreign.path().join(format!("{file}.pending"))).unwrap(),
            )
            .unwrap();
            let other = if key == "dictation-settings" {
                "murmur-theme-library"
            } else {
                "dictation-settings"
            };
            receipt["key"] = json!(other);
            receipt["value"] = document(other, 1);
            receipt["targetDigest"] = json!(source_digest(&receipt["value"]));
            receipt["targetRevision"] = json!(source_revision(other, &receipt["value"]));
            std::fs::write(
                local.path().join(format!("{file}.pending")),
                receipt.to_string(),
            )
            .unwrap();
            assert!(recover_source(local.path(), key).is_err());
            if destination_exists {
                assert_eq!(
                    serde_json::from_slice::<Value>(
                        &std::fs::read(local.path().join(file)).unwrap()
                    )
                    .unwrap(),
                    new
                );
            } else {
                assert!(!local.path().join(file).exists());
            }
        }
    }
}

#[test]
fn evidence_and_orphan_temps_are_bounded_oldest_first() {
    use std::fs::{File, FileTimes};
    for key in ["dictation-settings", "murmur-theme-library"] {
        let dir = tempfile::tempdir().unwrap();
        let file = crate::state::file(key).unwrap();
        save_source(dir.path(), key, &document(key, 2).to_string()).unwrap();
        for owner in [file.to_string(), format!("{file}.pending")] {
            for kind in ["quarantine", "tmp"] {
                for n in 0..20 {
                    let path = dir.path().join(format!("{owner}.{kind}-{n:02}"));
                    let f = File::create(path).unwrap();
                    f.set_times(
                        FileTimes::new().set_modified(
                            std::time::UNIX_EPOCH + std::time::Duration::from_secs(n),
                        ),
                    )
                    .unwrap();
                }
                recover_source(dir.path(), key).unwrap();
                let names: Vec<_> = std::fs::read_dir(dir.path())
                    .unwrap()
                    .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                    .filter(|n| n.starts_with(&format!("{owner}.{kind}-")))
                    .collect();
                assert!(!names.is_empty() && names.len() <= 8);
                assert!(names.contains(&format!("{owner}.{kind}-19")));
                assert!(!names.contains(&format!("{owner}.{kind}-00")));
            }
        }
    }
}

#[test]
fn interrupted_transition_replays_only_base_absent_or_exact_target() {
    use storage::Boundary;
    for key in ["dictation-settings", "murmur-theme-library"] {
        for boundary in [
            Boundary::BeforeRename,
            Boundary::AfterRename,
            Boundary::DirectorySync,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let file = crate::state::file(key).unwrap();
            let old = document(key, 1);
            let new = document(key, 2);
            save_source(dir.path(), key, &old.to_string()).unwrap();
            let mut storage = Storage::new(dir.path().into());
            storage.fail = Some((file.into(), boundary));
            assert!(publish_source(&mut storage, key, &new).is_err());
            let pending = std::fs::read(dir.path().join(format!("{file}.pending"))).unwrap();
            for _ in 0..2 {
                recover_source(dir.path(), key).unwrap();
                assert_eq!(
                    serde_json::from_slice::<Value>(&std::fs::read(dir.path().join(file)).unwrap())
                        .unwrap(),
                    new
                );
                // Crash after destination sync but before retiring the receipt.
                std::fs::write(dir.path().join(format!("{file}.pending")), &pending).unwrap();
            }
        }
    }
}
