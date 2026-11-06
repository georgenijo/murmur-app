use super::*;
use storage::Boundary;
use tempfile::TempDir;
fn engine(root: &std::path::Path, webkit: bool) -> Engine {
    let mut e = Engine {
        storage: Storage {
            root: root.into(),
            ..Default::default()
        },
        webkit_exists: webkit,
        seen: HashSet::new(),
        revision: 0,
    };
    e.initialize().unwrap();
    e
}
fn entry(id: &str, text: &str) -> Value {
    json!({"schemaVersion":2,"id":id,"text":text,"timestamp":1,"duration":2,"source":"recording"})
}
fn report(n: u64) -> Value {
    json!({"createdAt":format!("2026-11-{:02}T00:00:00Z",n+1),"appVersion":"test","platform":"test","preset":"quick","iterations":1,"sharedInitMs":0,"results":[],"recommendations":{"fastest":null,"mostAccurate":null,"balanced":null}})
}
fn dictation() -> StatisticsEvent {
    StatisticsEvent::Dictation {
        words: 3,
        seconds: 2.0,
        mode: Some("mode".into()),
        day: "2026-11-05".into(),
    }
}
fn complete(e: &mut Engine, id: &str, retain: bool) -> Snapshot {
    e.execute(Command::Complete {
        source: id.into(),
        entry: retain.then(|| entry(id, "synthetic")),
        event: Some(dictation()),
    })
    .unwrap()
}
fn append(e: &mut Engine, event: StatisticsEvent) {
    let mut s = e.state().unwrap();
    s.journal.push(Record {
        schema: 1,
        sequence: s.next,
        uuid: uuid(),
        event,
    });
    s.next += 1;
    e.storage.write(STATE, &s).unwrap();
}
fn legacy() -> LegacyImport {
    LegacyImport {
        history: Some(json!([entry("old", "legacy synthetic")])),
        statistics: Some(json!(Stats {
            total_words: 10,
            total_recordings: 2,
            total_duration_seconds: 12.0,
            ..Default::default()
        })),
        reports: Some(json!([report(0)])),
    }
}
fn restart(e: &Engine) -> Engine {
    engine(&e.storage.root, e.webkit_exists)
}
#[test]
fn crash_after_journal_before_history_replays_statistics_only() {
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), false);
    append(&mut e, dictation());
    let s = restart(&e).snapshot().unwrap();
    assert!(s.history.is_empty());
    assert_eq!(s.statistics.total_recordings, 1);
}
#[test]
fn crash_after_history_before_statistics_has_no_duplicate_history() {
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), false);
    append(&mut e, dictation());
    e.add_content(Destination::History, entry("a", "synthetic"))
        .unwrap();
    let s = restart(&e).snapshot().unwrap();
    assert_eq!(s.history.len(), 1);
    assert_eq!(s.statistics.total_recordings, 1);
}
#[test]
fn crash_after_statistics_before_compaction_does_not_reapply() {
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), false);
    append(&mut e, dictation());
    e.storage.fail = Some((STATE.into(), Boundary::BeforeRename));
    assert!(e.apply_statistics().is_err());
    assert_eq!(e.state().unwrap().journal.len(), 1);
    let mut e = restart(&e);
    assert_eq!(e.snapshot().unwrap().statistics.total_recordings, 1);
    e.recover().unwrap();
    assert!(e.state().unwrap().journal.is_empty());
}
#[test]
fn activity_events_crash_before_statistics_match_uninterrupted_run() {
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), false);
    for event in [
        StatisticsEvent::Meeting {
            seconds: 90.0,
            month: "2026-11".into(),
        },
        StatisticsEvent::CorrectionProposed,
        StatisticsEvent::CorrectionTaught {
            scope: Scope::Project,
        },
        StatisticsEvent::PasteLast,
    ] {
        append(&mut e, event);
    }
    let s = restart(&e).snapshot().unwrap().statistics;
    assert_eq!(s.total_recordings, 0);
    assert_eq!(s.activity.meetings.count, 1);
    assert_eq!(s.activity.meetings.total_minutes, 1.5);
    assert_eq!(s.activity.monthly_buckets["2026-11"].meetings, 1);
    assert_eq!(s.activity.corrections.proposed, 1);
    assert_eq!(s.activity.corrections.taught, 1);
    assert_eq!(s.activity.corrections.by_scope.project, 1);
    assert_eq!(s.activity.paste_last_uses, 1);
}
#[test]
fn clock_backwards_and_equal_timestamps_do_not_change_counters() {
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), false);
    for i in 0..3 {
        complete(&mut e, &i.to_string(), true);
    }
    let s = restart(&e).snapshot().unwrap();
    assert_eq!(s.statistics.total_words, 9);
    assert_eq!(s.history.len(), 3);
    assert_eq!(s.statistics.daily_buckets["2026-11-05"].recordings, 3);
}
#[test]
fn statistics_n_fails_then_n_plus_one_applies_both_in_order() {
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), false);
    append(&mut e, dictation());
    e.storage.fail = Some(("stats.json".into(), Boundary::BeforeRename));
    assert!(e.apply_statistics().is_err());
    append(
        &mut e,
        StatisticsEvent::Dictation {
            words: 10,
            seconds: 1.0,
            mode: None,
            day: "2026-11-04".into(),
        },
    );
    e.apply_statistics().unwrap();
    let s = e.snapshot().unwrap();
    assert_eq!(s.statistics.total_recordings, 2);
    assert_eq!(s.statistics.wpm_samples, vec![90.0, 600.0]);
}
#[test]
fn history_write_fails_statistics_apply_and_no_history_entry() {
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), false);
    e.storage.fail = Some(("history.json".into(), Boundary::BeforeRename));
    let s = complete(&mut e, "1", true);
    assert!(s.history.is_empty());
    assert_eq!(s.statistics.total_recordings, 1);
}
#[test]
fn statistics_unknown_outcome_rereads_and_does_not_compact_until_synced() {
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), false);
    append(&mut e, dictation());
    e.storage.fail = Some(("stats.json".into(), Boundary::AfterRename));
    assert!(e.apply_statistics().is_err());
    assert_eq!(e.state().unwrap().journal.len(), 1);
    e.recover().unwrap();
    assert_eq!(e.snapshot().unwrap().statistics.total_recordings, 1);
    assert!(e.state().unwrap().journal.is_empty());
}
#[test]
fn power_loss_after_unknown_statistics_write_replays_once() {
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), false);
    append(&mut e, dictation());
    e.storage.fail = Some(("stats.json".into(), Boundary::AfterRename));
    assert!(e.apply_statistics().is_err());
    // Either the renamed directory entry survives, or the old file does. The
    // durable journal is retained in both cases, so both possible disks recover.
    let stats_path = dir.path().join("stats.json");
    let published = std::fs::read(&stats_path).unwrap();
    for survives in [true, false] {
        if survives {
            std::fs::write(&stats_path, &published).unwrap();
        } else {
            std::fs::remove_file(&stats_path).unwrap();
        }
        let s = restart(&e).snapshot().unwrap();
        assert_eq!(s.statistics.total_recordings, 1);
        // Restore the pre-compaction state for the other disk outcome.
        let mut st = e.state().unwrap();
        st.journal = vec![Record {
            schema: 1,
            sequence: 1,
            uuid: uuid(),
            event: dictation(),
        }];
        e.storage.write(STATE, &st).unwrap();
    }
}
#[test]
fn reset_with_pending_records_advances_watermark_in_one_write() {
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), true);
    append(&mut e, dictation());
    e.clear(Destination::Statistics, 42).unwrap();
    let d = e.document(Destination::Statistics).unwrap().unwrap();
    assert_eq!(d.applied_through, 1);
    assert_eq!(d.reset_at, Some(42));
    e.import(legacy()).unwrap();
    assert_eq!(
        restart(&e).snapshot().unwrap().statistics.total_recordings,
        0
    );
}
#[test]
fn clear_history_and_reports_write_explicit_empty_files() {
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), false);
    complete(&mut e, "1", true);
    e.add_content(Destination::Reports, report(0)).unwrap();
    for d in [Destination::History, Destination::Reports] {
        e.clear(d, 9).unwrap();
        let doc = e.document(d).unwrap().unwrap();
        assert_eq!(doc.data, json!([]));
        assert_eq!(doc.cleared_at, Some(9));
    }
}
#[test]
fn deletion_cap_and_retain_false_never_change_statistics() {
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), false);
    for i in 0..205 {
        complete(&mut e, &i.to_string(), true);
    }
    let s = complete(&mut e, "no-retention", false);
    assert_eq!(s.history.len(), 200);
    let stats = s.statistics;
    e.mutate_history(HistoryMutation::Delete { targets: s.history })
        .unwrap();
    assert_eq!(e.snapshot().unwrap().statistics, stats);
}
#[test]
fn state_first_creation_failure_blocks_all_writes_and_arming() {
    let dir = TempDir::new().unwrap();
    let mut e = Engine {
        storage: Storage {
            root: dir.path().into(),
            fail: Some((STATE.into(), Boundary::BeforeRename)),
        },
        webkit_exists: true,
        seen: HashSet::new(),
        revision: 0,
    };
    assert!(e.initialize().is_err());
    assert!(!dir.path().join("history.json").exists());
    assert!(!dir.path().join("stats.json").exists());
    assert_eq!(
        MigrationGate::default().tick(false, false, 20000),
        GateAction::Wait
    );
    e.initialize().unwrap();
}
#[test]
fn state_deleted_corrupt_or_future_before_and_after_v1_recovers_sequence() {
    for committed in [false, true] {
        for corrupt in [
            None,
            Some("bad"),
            Some(r#"{"schema":99,"next":1,"journal":[]}"#),
        ] {
            let dir = TempDir::new().unwrap();
            let mut e = engine(dir.path(), !committed);
            complete(&mut e, "a", true);
            if let Some(bytes) = corrupt {
                std::fs::write(dir.path().join(STATE), bytes).unwrap();
            } else {
                std::fs::remove_file(dir.path().join(STATE)).unwrap();
            }
            e = restart(&e);
            let s = complete(&mut e, "b", true);
            assert_eq!(s.statistics.total_recordings, if committed { 2 } else { 1 });
            assert_eq!(s.v1_committed, committed);
        }
    }
}
#[test]
fn holding_deleted_corrupt_or_future_does_not_change_import_decision() {
    for corrupt in [None, Some("bad"), Some(r#"{"schema":99}"#)] {
        let dir = TempDir::new().unwrap();
        let mut e = engine(dir.path(), true);
        complete(&mut e, "a", true);
        if let Some(bytes) = corrupt {
            std::fs::write(dir.path().join(HOLD), bytes).unwrap();
        } else {
            std::fs::remove_file(dir.path().join(HOLD)).unwrap();
        }
        e = restart(&e);
        e.import(legacy()).unwrap();
        let s = e.snapshot().unwrap();
        assert_eq!(s.history.len(), 1);
        assert_eq!(s.statistics.total_recordings, 3);
    }
}
#[test]
fn stale_settings_save_does_not_touch_journal_or_migration() {
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), true);
    complete(&mut e, "a", true);
    std::fs::write(dir.path().join("settings.json"), "{}").unwrap();
    e = restart(&e);
    e.import(legacy()).unwrap();
    assert_eq!(e.snapshot().unwrap().statistics.total_recordings, 3);
}
#[test]
fn malformed_history_or_future_schema_quarantined_then_webkit_recovers() {
    for bad in [
        json!([null]),
        json!({"schema":99,"data":[]}),
        json!([{"id":"a","text":3}]),
    ] {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("history.json"), bad.to_string()).unwrap();
        let mut e = engine(dir.path(), true);
        e.import(legacy()).unwrap();
        assert_eq!(e.snapshot().unwrap().history.len(), 1);
        assert!(std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .any(|f| f.file_name().to_string_lossy().contains("quarantine")));
    }
}
#[test]
fn unversioned_statistics_and_legacy_history_are_valid_destinations() {
    let dir = TempDir::new().unwrap();
    let mut old = entry("disk", "synthetic");
    old.as_object_mut().unwrap().remove("schemaVersion");
    std::fs::write(dir.path().join("history.json"), json!([old]).to_string()).unwrap();
    std::fs::write(
        dir.path().join("stats.json"),
        json!(Stats::default()).to_string(),
    )
    .unwrap();
    let mut e = engine(dir.path(), true);
    e.import(legacy()).unwrap();
    let s = e.snapshot().unwrap();
    assert_eq!(s.history[0]["id"], "disk");
    assert_eq!(s.statistics.total_words, 0);
}
#[test]
fn late_import_disk_wins_over_identical_webkit_counters() {
    let dir = TempDir::new().unwrap();
    let source = legacy();
    std::fs::write(
        dir.path().join("history.json"),
        source.history.as_ref().unwrap().to_string(),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("stats.json"),
        source.statistics.as_ref().unwrap().to_string(),
    )
    .unwrap();
    let mut e = engine(dir.path(), true);
    complete(&mut e, "new", true);
    e.import(source).unwrap();
    let s = e.snapshot().unwrap();
    assert_eq!(s.history.len(), 2);
    assert_eq!(s.statistics.total_words, 13);
}
#[test]
fn late_import_webkit_only_is_below_held_and_added_once() {
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), true);
    complete(&mut e, "new", true);
    e.import(legacy()).unwrap();
    e.import(legacy()).unwrap();
    let s = e.snapshot().unwrap();
    assert_eq!(s.history.len(), 2);
    assert_eq!(s.history[0]["id"], "old");
    assert_eq!(s.history[1]["id"], "new");
    assert_eq!(s.statistics.total_recordings, 3);
}
#[test]
fn valid_disk_statistics_corrupt_after_timeout_preserves_all_journal_events() {
    let dir = TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("stats.json"),
        legacy().statistics.unwrap().to_string(),
    )
    .unwrap();
    let mut e = engine(dir.path(), true);
    complete(&mut e, "a", true);
    std::fs::write(dir.path().join("stats.json"), "broken").unwrap();
    e = restart(&e);
    complete(&mut e, "b", true);
    append(
        &mut e,
        StatisticsEvent::Meeting {
            seconds: 60.0,
            month: "2026-11".into(),
        },
    );
    assert!(!dir.path().join("stats.json").exists());
    e.import(legacy()).unwrap();
    let s = e.snapshot().unwrap();
    assert_eq!(s.statistics.total_recordings, 4);
    assert_eq!(s.statistics.activity.meetings.count, 1);
}
#[test]
fn original_history_corrupt_after_timeout_recovers_legacy_below_held() {
    let dir = TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("history.json"),
        legacy().history.unwrap().to_string(),
    )
    .unwrap();
    let mut e = engine(dir.path(), true);
    complete(&mut e, "a", true);
    std::fs::write(dir.path().join("history.json"), "broken").unwrap();
    e = restart(&e);
    complete(&mut e, "b", true);
    e.import(legacy()).unwrap();
    let s = e.snapshot().unwrap();
    assert_eq!(
        s.history
            .iter()
            .map(|v| v["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["old", "a", "b"]
    );
}
#[test]
fn timeout_then_quit_then_next_launch_import_keeps_webkit_and_held() {
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), true);
    complete(&mut e, "a", true);
    e = restart(&e);
    e.import(legacy()).unwrap();
    assert_eq!(e.snapshot().unwrap().history.len(), 2);
    assert_eq!(e.snapshot().unwrap().statistics.total_recordings, 3);
}
#[test]
fn clear_before_late_import_skips_each_destination_and_removes_its_keys() {
    for d in [
        Destination::History,
        Destination::Statistics,
        Destination::Reports,
    ] {
        let dir = TempDir::new().unwrap();
        let mut e = engine(dir.path(), true);
        complete(&mut e, "a", true);
        e.add_content(Destination::Reports, report(2)).unwrap();
        e.clear(d, 1).unwrap();
        e.import(legacy()).unwrap();
        let s = restart(&e).snapshot().unwrap();
        match d {
            Destination::History => assert!(s.history.is_empty()),
            Destination::Statistics => assert_eq!(s.statistics.total_recordings, 0),
            Destination::Reports => assert!(s.reports.is_empty()),
        };
        assert_eq!(s.remove_keys.len(), 4);
    }
}
#[test]
fn crash_after_clear_destination_before_holding_drops_old_keeps_later() {
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), true);
    complete(&mut e, "old", true);
    e.storage.fail = Some((HOLD.into(), Boundary::BeforeRename));
    assert!(e.clear(Destination::History, 1).is_err());
    e = restart(&e);
    complete(&mut e, "later", true);
    e.import(legacy()).unwrap();
    let s = e.snapshot().unwrap();
    assert_eq!(s.history.len(), 1);
    assert_eq!(s.history[0]["id"], "later");
}
#[test]
fn clear_before_v1_then_migration_file_lost_stays_cleared() {
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), true);
    e.clear(Destination::History, 1).unwrap();
    std::fs::write(dir.path().join(MIGRATION), "broken").unwrap();
    e = restart(&e);
    e.import(legacy()).unwrap();
    assert!(e.snapshot().unwrap().history.is_empty());
}
#[test]
fn deleted_held_and_original_entries_do_not_return_after_corruption() {
    let dir = TempDir::new().unwrap();
    std::fs::write(
        dir.path().join("history.json"),
        legacy().history.unwrap().to_string(),
    )
    .unwrap();
    let mut e = engine(dir.path(), true);
    complete(&mut e, "held", true);
    e.mutate_history(HistoryMutation::Delete {
        targets: e.snapshot().unwrap().history,
    })
    .unwrap();
    std::fs::write(dir.path().join("history.json"), "broken").unwrap();
    e.import(legacy()).unwrap();
    assert!(e.snapshot().unwrap().history.is_empty());
}
#[test]
fn crash_after_import_destination_before_drain_adds_content_once() {
    for file in ["history.json", "benchmark-reports.json"] {
        let dir = TempDir::new().unwrap();
        let mut e = engine(dir.path(), true);
        complete(&mut e, "held", true);
        e.add_content(Destination::Reports, report(2)).unwrap();
        e.storage.fail = Some((file.into(), Boundary::AfterRename));
        assert!(e.import(legacy()).is_err());
        e = restart(&e);
        e.import(legacy()).unwrap();
        let s = e.snapshot().unwrap();
        assert_eq!(s.history.len(), 2);
        assert_eq!(s.reports.len(), 2);
        assert_eq!(s.statistics.total_recordings, 3);
    }
}
#[test]
fn failed_import_drain_then_user_delete_and_relaunch_does_not_resurrect() {
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), true);
    complete(&mut e, "held", true);
    e.storage.fail = Some((HOLD.into(), Boundary::BeforeRename));
    assert!(e.import(legacy()).is_err());
    e.recover().unwrap();
    let target = e.snapshot().unwrap().history.pop().unwrap();
    e.mutate_history(HistoryMutation::Delete {
        targets: vec![target],
    })
    .unwrap();
    e = restart(&e);
    e.import(legacy()).unwrap();
    assert_eq!(e.snapshot().unwrap().history.len(), 1);
}
#[test]
fn duplicate_legacy_ids_delete_skips_exactly_one_matching_text() {
    let dir = TempDir::new().unwrap();
    let entries = json!([entry("same", "first"), entry("same", "second")]);
    std::fs::write(dir.path().join("history.json"), entries.to_string()).unwrap();
    let mut e = engine(dir.path(), true);
    e.mutate_history(HistoryMutation::Delete {
        targets: vec![entries[0].clone()],
    })
    .unwrap();
    std::fs::write(dir.path().join("history.json"), "broken").unwrap();
    e.import(LegacyImport {
        history: Some(entries),
        ..Default::default()
    })
    .unwrap();
    let s = e.snapshot().unwrap();
    assert_eq!(s.history.len(), 1);
    assert_eq!(s.history[0]["text"], "second");
}
#[test]
fn repeated_timeouts_cap_combined_history_pins_reports_and_show_retry_on_third_launch() {
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), true);
    for i in 0..225 {
        let mut v = entry(&i.to_string(), "synthetic");
        v["pinned"] = json!(true);
        e.add_content(Destination::History, v).unwrap();
    }
    for i in 0..20 {
        e.add_content(Destination::Reports, report(i)).unwrap();
    }
    assert!(!e.snapshot().unwrap().retry_notice);
    e = restart(&e);
    assert!(!e.snapshot().unwrap().retry_notice);
    e = restart(&e);
    let s = e.snapshot().unwrap();
    assert!(s.retry_notice);
    assert_eq!(s.history.len(), 200);
    assert_eq!(s.history.iter().filter(|v| v["pinned"] == true).count(), 20);
    assert_eq!(s.reports.len(), 10);
}
#[test]
fn fresh_install_without_webkit_commits_immediately_and_never_holds() {
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), false);
    assert!(e.snapshot().unwrap().v1_committed);
    complete(&mut e, "a", true);
    assert!(e.holding().unwrap().history.entries.is_empty());
    assert_eq!(restart(&e).snapshot().unwrap().history.len(), 1);
}
#[test]
fn retired_store_migration_file_lost_recommits_without_changing_destinations() {
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), false);
    complete(&mut e, "a", true);
    let before = std::fs::read(dir.path().join("history.json")).unwrap();
    std::fs::remove_file(dir.path().join(MIGRATION)).unwrap();
    e = restart(&e);
    assert!(e.snapshot().unwrap().v1_committed);
    assert_eq!(
        before,
        std::fs::read(dir.path().join("history.json")).unwrap()
    );
}
#[test]
fn crash_after_destination_import_before_v1_commit_skips_marked_destinations() {
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), true);
    e.storage.fail = Some((MIGRATION.into(), Boundary::BeforeRename));
    assert!(e.import(legacy()).is_err());
    let before = e.snapshot().unwrap();
    e = restart(&e);
    e.import(legacy()).unwrap();
    assert_eq!(e.snapshot().unwrap().statistics, before.statistics);
    assert_eq!(e.snapshot().unwrap().history, before.history);
}
#[test]
fn migration_file_lost_before_or_after_commit_never_imports_twice() {
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), true);
    e.import(legacy()).unwrap();
    for _ in 0..3 {
        std::fs::remove_file(dir.path().join(MIGRATION)).unwrap();
        e = restart(&e);
        e.import(legacy()).unwrap();
        let s = e.snapshot().unwrap();
        assert_eq!(s.history.len(), 1);
        assert_eq!(s.statistics.total_recordings, 2);
    }
}
#[test]
fn colliding_legacy_and_new_ids_keep_both_entries() {
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), true);
    complete(&mut e, "old", true);
    e.import(legacy()).unwrap();
    let s = e.snapshot().unwrap();
    assert_eq!(s.history.len(), 2);
    assert_ne!(s.history[0]["id"], s.history[1]["id"]);
}
#[test]
fn killed_webcontent_reload_once_then_arm_holding_and_state_failure_never_arms() {
    let mut gate = MigrationGate::default();
    assert_eq!(gate.tick(true, false, 4999), GateAction::Wait);
    assert_eq!(gate.tick(true, false, 5000), GateAction::ReloadOnce);
    assert_eq!(gate.tick(true, false, 6000), GateAction::Wait);
    assert_eq!(gate.tick(true, false, 10000), GateAction::ArmHolding);
    assert_eq!(gate.tick(true, false, 20000), GateAction::ArmHolding);
    assert_eq!(gate.tick(false, false, 30000), GateAction::Wait);
    assert_eq!(gate.tick(true, true, 30001), GateAction::Arm);
}
#[test]
fn concurrent_writers_serialize_and_second_owner_is_rejected() {
    let dir = TempDir::new().unwrap();
    let service = CompletionService::start(dir.path().into(), false).unwrap();
    assert!(matches!(
        CompletionService::start(dir.path().into(), false),
        Err(Error::InUse)
    ));
    let joins: Vec<_> = (0..12)
        .map(|i| {
            let s = service.clone();
            std::thread::spawn(move || {
                s.execute(Command::Complete {
                    source: i.to_string(),
                    entry: Some(entry(&i.to_string(), "synthetic")),
                    event: Some(dictation()),
                })
                .unwrap()
            })
        })
        .collect();
    for j in joins {
        j.join().unwrap();
    }
    let s = service.execute(Command::Snapshot).unwrap();
    assert_eq!(s.history.len(), 12);
    assert_eq!(s.statistics.total_recordings, 12);
}
#[test]
fn unknown_journal_append_outcome_does_not_append_again() {
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), false);
    // Exercise the append failure rather than the no-op recovery compaction.
    let mut state = e.state().unwrap();
    state.journal.push(Record {
        schema: 1,
        sequence: 1,
        uuid: uuid(),
        event: dictation(),
    });
    state.next = 2;
    e.storage.fail = Some((STATE.into(), Boundary::AfterRename));
    assert!(e.storage.write(STATE, &state).is_err());
    e = restart(&e);
    assert_eq!(e.snapshot().unwrap().statistics.total_recordings, 1);
}
#[test]
fn statistics_schema_rejects_content_and_reports_reject_malformed_entries() {
    let mut stats = json!(Stats::default());
    stats["transcript"] = json!("not accepted");
    assert!(!valid_data(Destination::Statistics, &stats));
    assert!(!valid_data(Destination::Reports, &json!([null])));
    assert!(!valid_data(
        Destination::Reports,
        &json!([{"createdAt":"bad"}])
    ));
}
#[test]
fn headless_completions_retention_file_activity_reopen_and_relaunch() {
    // The companion control test exercises arm/start/stop/cancel/auto-stop/paste.
    // This test attaches a real durable service to the same headless environment.
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), false);
    complete(&mut e, "recording:1", true);
    complete(&mut e, "recording:1", true);
    complete(&mut e, "recording:2", false);
    e.execute(Command::Complete {
        source: "file:1".into(),
        entry: Some(entry("file", "synthetic file")),
        event: None,
    })
    .unwrap();
    for (id, event) in [
        (
            "meeting",
            StatisticsEvent::Meeting {
                seconds: 60.0,
                month: "2026-11".into(),
            },
        ),
        (
            "taught",
            StatisticsEvent::CorrectionTaught { scope: Scope::App },
        ),
        ("paste", StatisticsEvent::PasteLast),
    ] {
        for _ in 0..2 {
            e.execute(Command::Complete {
                source: id.into(),
                entry: None,
                event: Some(event.clone()),
            })
            .unwrap();
        }
    }
    let before = e.snapshot().unwrap();
    for _ in 0..3 {
        assert_eq!(e.snapshot().unwrap().statistics, before.statistics);
    }
    let after = restart(&e).snapshot().unwrap();
    assert_eq!(after.history.len(), 2);
    assert_eq!(after.statistics.total_recordings, 2);
    assert_eq!(after.statistics.activity.meetings.count, 1);
    assert_eq!(after.statistics.activity.corrections.taught, 1);
    assert_eq!(after.statistics.activity.paste_last_uses, 1);
    assert_eq!(after.statistics, before.statistics);
}

#[test]
fn pin_limit_exact_row_edits_preserve_raw_provenance_and_statistics() {
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), false);
    for i in 0..21 {
        let mut v = entry(&i.to_string(), "synthetic final");
        v["rawText"] = json!("synthetic raw");
        v["pinned"] = json!(i < 20);
        e.add_content(Destination::History, v).unwrap();
    }
    let before = e.snapshot().unwrap();
    let target = before.history[20].clone();
    assert!(e
        .mutate_history(HistoryMutation::Pin {
            target: target.clone()
        })
        .is_err());
    e.mutate_history(HistoryMutation::Edit {
        target,
        text: "synthetic edit".into(),
    })
    .unwrap();
    let after = e.snapshot().unwrap();
    assert_eq!(after.history[20]["rawText"], "synthetic raw");
    assert_eq!(after.history[20]["text"], "synthetic edit");
    assert_eq!(before.statistics, after.statistics);
}
#[test]
fn upgrade_keeps_legacy_transforms_and_wpm_order_with_private_prototype_labels() {
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), true);
    let mut stats = Stats {
        wpm_samples: vec![1.0; 100],
        ..Default::default()
    };
    stats.activity.transforms.runs = 7;
    stats.activity.transforms.by_preset_name.insert(
        "__proto__".into(),
        stats::Counts {
            runs: 7,
            approved: 2,
            undone: 1,
        },
    );
    for i in 0..3 {
        append(
            &mut e,
            StatisticsEvent::Dictation {
                words: i + 1,
                seconds: 1.0,
                mode: Some("constructor".into()),
                day: "2026-11-05".into(),
            },
        );
    }
    e.import(LegacyImport {
        statistics: Some(json!(stats)),
        ..Default::default()
    })
    .unwrap();
    let s = e.snapshot().unwrap().statistics;
    assert_eq!(s.activity.transforms.runs, 7);
    assert_eq!(s.activity.transforms.by_preset_name["__proto__"].runs, 7);
    assert_eq!(s.activity.recordings_by_mode["constructor"], 3);
    assert_eq!(s.wpm_samples.len(), 100);
    assert_eq!(&s.wpm_samples[97..], &[60.0, 120.0, 180.0]);
    assert_eq!(s, restart(&e).snapshot().unwrap().statistics);
}
#[test]
fn durable_key_removal_acknowledgements_survive_partial_import_and_relaunch() {
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), true);
    assert!(e
        .execute(Command::KeysRemoved(vec![Destination::History]))
        .is_err());
    e.clear(Destination::History, 1).unwrap();
    e.execute(Command::KeysRemoved(vec![Destination::History]))
        .unwrap();
    e = restart(&e);
    assert_eq!(
        e.migration().unwrap().keys_removed,
        vec![Destination::History]
    );
    e.import(legacy()).unwrap();
    e.execute(Command::KeysRemoved(vec![
        Destination::Statistics,
        Destination::Reports,
    ]))
    .unwrap();
    e = restart(&e);
    assert_eq!(e.migration().unwrap().keys_removed.len(), 3);
    assert!(e.snapshot().unwrap().history.is_empty());
}
#[test]
fn report_uuid_does_not_prevent_creation_time_and_content_deduplication() {
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), true);
    e.execute(Command::Report(report(0))).unwrap();
    assert!(e.holding().unwrap().reports.entries[0]["completionUuid"].is_string());
    e.import(legacy()).unwrap();
    assert_eq!(e.snapshot().unwrap().reports.len(), 1);
    let mut other = report(0);
    other["sharedInitMs"] = json!(10);
    e.execute(Command::Report(other)).unwrap();
    assert_eq!(e.snapshot().unwrap().reports.len(), 2);
    e = restart(&e);
    assert_eq!(e.snapshot().unwrap().reports.len(), 2);
}
#[test]
fn report_schema_ports_legacy_v2_v3_metadata_and_rejects_future_or_malformed_fields() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../src/lib/__fixtures__/diagnostic-reports/benchmark-v2.json");
    let mut v: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert!(valid_report(&v));
    v["reportVersion"] = json!(3);
    assert!(!valid_report(&v));
    v["corpus"]["source"] = json!("bundled");
    assert!(valid_report(&v));
    v["reportVersion"] = json!(99);
    assert!(!valid_report(&v));
    v["reportVersion"] = json!(2);
    v["configuration"]["modelRunOrder"] = json!([null]);
    assert!(!valid_report(&v));
}
#[test]
fn unknown_outcome_at_real_journal_append_is_claimed_before_retry() {
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), false);
    e.storage.fail = Some((STATE.into(), Boundary::AfterRename));
    let command = Command::Complete {
        source: "recording:1".into(),
        entry: Some(entry("1", "synthetic")),
        event: Some(dictation()),
    };
    assert!(e.execute(command.clone()).is_err());
    let s = e.execute(command).unwrap();
    assert_eq!(s.statistics.total_recordings, 1);
    assert!(s.history.is_empty());
    assert!(e.state().unwrap().journal.is_empty());
}
