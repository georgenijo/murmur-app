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
            #[cfg(unix)]
            for metadata in [
                "required",
                "required-history",
                "identity",
                "pending.quarantine_sequence",
            ] {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(
                    std::fs::metadata(local.path().join(format!("{file}.{metadata}")))
                        .unwrap()
                        .permissions()
                        .mode()
                        & 0o777,
                    0o600
                );
            }
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

fn artifacts(root: &std::path::Path, owner: &str) -> Vec<std::path::PathBuf> {
    let mut paths: Vec<_> = std::fs::read_dir(root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(&format!("{owner}.quarantine-"))
        })
        .collect();
    paths.sort();
    paths
}

fn set_mtime(path: &std::path::Path, seconds: u64) {
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_times(
            std::fs::FileTimes::new()
                .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds)),
        )
        .unwrap();
}

#[test]
fn pruning_preserves_current_old_receipt_and_destination_in_separate_families() {
    for key in ["dictation-settings", "murmur-theme-library"] {
        for suffix in ["", ".pending"] {
            let dir = tempfile::tempdir().unwrap();
            let file = crate::state::file(key).unwrap();
            save_source(dir.path(), key, &document(key, 2).to_string()).unwrap();
            let owner = format!("{file}{suffix}");
            let storage = Storage::new(dir.path().into());
            // Eight newer same-family entries and eight identity entries must not
            // evict the just-quarantined, timestamp-preserved current evidence.
            for family in [&owner, &format!("{file}.identity")] {
                for n in 0..8 {
                    let path = dir.path().join(format!("{family}.quarantine-legacy-{n}"));
                    std::fs::write(&path, b"synthetic older quarantine").unwrap();
                    set_mtime(&path, 2000 + n);
                }
            }
            let path = dir.path().join(&owner);
            std::fs::write(&path, b"synthetic current evidence").unwrap();
            set_mtime(&path, 1);
            storage.quarantine(&owner).unwrap();
            prune_source_artifacts(dir.path(), key).unwrap();
            assert!(artifacts(dir.path(), &owner)
                .iter()
                .any(|p| std::fs::read(p).unwrap() == b"synthetic current evidence"));
            assert!(!artifacts(dir.path(), file).is_empty() || !suffix.is_empty());
        }
    }
}

#[test]
fn pruning_missing_source_without_markers_keeps_evidence_and_blocks_public_status() {
    for key in ["dictation-settings", "murmur-theme-library"] {
        let dir = tempfile::tempdir().unwrap();
        let file = crate::state::file(key).unwrap();
        let backup = dir.path().join(format!("{file}.quarantine-legacy-backup"));
        std::fs::write(&backup, document(key, 1).to_string()).unwrap();
        set_mtime(&backup, 1);
        for n in 0..9 {
            let path = dir
                .path()
                .join(format!("{file}.identity.quarantine-legacy-{n}"));
            std::fs::write(&path, b"synthetic identity").unwrap();
            set_mtime(&path, 2000 + n);
        }
        for _ in 0..3 {
            let service = CompletionService::start(dir.path().into(), true).unwrap();
            assert!(service.lifecycle(lifecycle::Operation::Status).is_err());
            drop(service);
            assert!(source_required(dir.path(), key).unwrap());
            assert!(backup.exists());
        }
        // Even loss of the remaining backup must not erase the durable guard.
        std::fs::remove_file(backup).unwrap();
        assert!(source_required(dir.path(), key).unwrap());
        let service = CompletionService::start(dir.path().into(), true).unwrap();
        assert!(service.lifecycle(lifecycle::Operation::Status).is_err());
        drop(service);
        restore_source(dir.path(), key, &document(key, 2)).unwrap();
        let service = CompletionService::start(dir.path().into(), true).unwrap();
        assert_eq!(
            service
                .lifecycle(lifecycle::Operation::Status)
                .unwrap()
                .values[key],
            document(key, 2)
        );
    }
}

#[test]
fn pruning_uses_quarantine_sequence_across_clock_rollback_and_restart() {
    for key in ["dictation-settings", "murmur-theme-library"] {
        let dir = tempfile::tempdir().unwrap();
        let file = crate::state::file(key).unwrap();
        save_source(dir.path(), key, &document(key, 2).to_string()).unwrap();
        let owner = format!("{file}.pending");
        let mut first = None;
        for n in 0..20 {
            let storage = Storage::new(dir.path().into());
            let path = dir.path().join(&owner);
            std::fs::write(&path, format!("synthetic generation {n}")).unwrap();
            // Later quarantines have strictly earlier content mtimes.
            set_mtime(&path, 2000 - n);
            storage.quarantine(&owner).unwrap();
            if n == 0 {
                first = artifacts(dir.path(), &owner).first().cloned();
            }
        }
        let paths = artifacts(dir.path(), &owner);
        assert_eq!(paths.len(), 8);
        assert!(!first.unwrap().exists());
        for n in 12..20 {
            assert!(paths.iter().any(
                |p| std::fs::read_to_string(p).unwrap() == format!("synthetic generation {n}")
            ));
        }
    }
}

#[test]
fn pruning_persists_guard_before_a_crash_can_remove_history() {
    for key in ["dictation-settings", "murmur-theme-library"] {
        for after_durable_guard in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let file = crate::state::file(key).unwrap();
            for n in 0..12 {
                std::fs::write(
                    dir.path().join(format!("{file}.quarantine-legacy-{n:02}")),
                    b"synthetic backup",
                )
                .unwrap();
            }
            // A usable destination makes these old backups eligible for pruning.
            std::fs::write(dir.path().join(file), document(key, 1).to_string()).unwrap();
            let storage = Storage::new(dir.path().into());
            if after_durable_guard {
                // Crash after file full-sync + guard rename + directory sync,
                // before removing any resolved evidence.
                storage.fail_prune_after_guard.set(true);
            } else {
                // Unknown guard publication: directory sync fails after rename.
                storage.fail_sync.set(true);
            }
            assert!(storage.prune_artifacts(file).is_err());
            assert_eq!(artifacts(dir.path(), file).len(), 12);
            assert!(dir.path().join(format!("{file}.required-history")).exists());
            std::fs::remove_file(dir.path().join(file)).unwrap();
            let service = CompletionService::start(dir.path().into(), true).unwrap();
            assert!(service.lifecycle(lifecycle::Operation::Status).is_err());
            drop(service);
            assert!(source_required(dir.path(), key).unwrap());
        }
    }
}

#[test]
fn pruning_pins_unresolved_evidence_then_bounds_each_resolved_family() {
    for key in ["dictation-settings", "murmur-theme-library"] {
        let dir = tempfile::tempdir().unwrap();
        let file = crate::state::file(key).unwrap();
        let storage = Storage::new(dir.path().into());
        for owner in [
            file.to_string(),
            format!("{file}.pending"),
            format!("{file}.identity"),
        ] {
            for _ in 0..12 {
                std::fs::write(dir.path().join(&owner), b"synthetic unresolved evidence").unwrap();
                storage.quarantine(&owner).unwrap();
            }
            assert_eq!(artifacts(dir.path(), &owner).len(), 12);
        }
        // Restore resolves the missing source. Each family then has its own cap.
        restore_source(dir.path(), key, &document(key, 2)).unwrap();
        prune_source_artifacts(dir.path(), key).unwrap();
        for owner in [
            file.to_string(),
            format!("{file}.pending"),
            format!("{file}.identity"),
        ] {
            assert_eq!(artifacts(dir.path(), &owner).len(), 8);
        }
        // A live receipt also pins evidence while its base destination remains
        // readable. Retirement of that exact receipt makes the caps eligible.
        let mut interrupted = Storage::new(dir.path().into());
        interrupted.fail = Some((file.into(), storage::Boundary::BeforeRename));
        assert!(publish_source(&mut interrupted, key, &document(key, 2)).is_err());
        for owner in [
            file.to_string(),
            format!("{file}.pending"),
            format!("{file}.identity"),
        ] {
            for n in 0..12 {
                std::fs::write(
                    dir.path()
                        .join(format!("{owner}.quarantine-legacy-live-{n:02}")),
                    b"synthetic pending generation evidence",
                )
                .unwrap();
            }
        }
        prune_source_artifacts(dir.path(), key).unwrap();
        for owner in [
            file.to_string(),
            format!("{file}.pending"),
            format!("{file}.identity"),
        ] {
            assert_eq!(artifacts(dir.path(), &owner).len(), 20);
        }
        recover_source(dir.path(), key).unwrap();
        prune_source_artifacts(dir.path(), key).unwrap();
        for owner in [
            file.to_string(),
            format!("{file}.pending"),
            format!("{file}.identity"),
        ] {
            assert_eq!(artifacts(dir.path(), &owner).len(), 8);
        }
    }
}
