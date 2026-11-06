use super::*;
use storage::Boundary;
use tempfile::TempDir;
pub(super) fn engine(root: &std::path::Path, webkit: bool) -> Engine {
    let mut e = Engine {
        unpreserved_content: false,
        failed_source: None,
        failed_content: None,
        storage: Storage {
            root: root.into(),
            ..Default::default()
        },
        webkit_exists: webkit,
        seen: HashSet::new(),
        revision: 0,
        initialized: false,
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
pub(super) fn reopen(e: &Engine) -> Engine {
    engine(&e.storage.root, e.webkit_exists)
}
#[test]
fn crash_after_journal_before_history_replays_statistics_only() {
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), false);
    e.storage.fail = Some((STATE.into(), Boundary::AfterRename));
    assert!(e
        .execute(Command::Complete {
            source: "crashed".into(),
            entry: Some(entry("crashed", "synthetic")),
            event: Some(dictation())
        })
        .is_err());
    let s = reopen(&e).snapshot().unwrap();
    assert!(s.history.is_empty());
    assert_eq!(s.statistics.total_recordings, 1);
}
#[test]
fn crash_after_history_before_statistics_has_no_duplicate_history() {
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), false);
    e.storage.fail = Some(("stats.json".into(), Boundary::BeforeRename));
    assert!(e
        .execute(Command::Complete {
            source: "crashed".into(),
            entry: Some(entry("crashed", "synthetic")),
            event: Some(dictation())
        })
        .is_err());
    let s = reopen(&e).snapshot().unwrap();
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
    let mut e = reopen(&e);
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
        e.storage.fail = Some(("stats.json".into(), Boundary::BeforeRename));
        assert!(e
            .execute(Command::Complete {
                source: uuid(),
                entry: None,
                event: Some(event)
            })
            .is_err());
        e = reopen(&e);
    }
    let s = reopen(&e).snapshot().unwrap().statistics;
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
    for (i, timestamp) in [100, 100, 50].into_iter().enumerate() {
        let mut v = entry(&i.to_string(), "synthetic");
        v["timestamp"] = json!(timestamp);
        e.execute(Command::Complete {
            source: i.to_string(),
            entry: Some(v),
            event: Some(dictation()),
        })
        .unwrap();
    }
    let s = reopen(&e).snapshot().unwrap();
    assert_eq!(s.statistics.total_words, 9);
    assert_eq!(s.history.len(), 3);
    assert_eq!(s.statistics.daily_buckets["2026-11-05"].recordings, 3);
}
#[test]
fn statistics_n_fails_then_n_plus_one_applies_both_in_order() {
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), false);
    e.storage.fail = Some(("stats.json".into(), Boundary::BeforeRename));
    assert!(e
        .execute(Command::Complete {
            source: "N".into(),
            entry: None,
            event: Some(dictation())
        })
        .is_err());
    assert_eq!(e.state().unwrap().journal.len(), 1);
    e.execute(Command::Complete {
        source: "N+1".into(),
        entry: None,
        event: Some(StatisticsEvent::Dictation {
            words: 10,
            seconds: 1.0,
            mode: None,
            day: "2026-11-04".into(),
        }),
    })
    .unwrap();
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
    let old_stats = std::fs::read(dir.path().join("stats.json")).unwrap();
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
            std::fs::write(&stats_path, &old_stats).unwrap();
        }
        let s = reopen(&e).snapshot().unwrap();
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
        reopen(&e).snapshot().unwrap().statistics.total_recordings,
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
        unpreserved_content: false,
        failed_source: None,
        failed_content: None,
        storage: Storage {
            root: dir.path().into(),
            fail: Some((STATE.into(), Boundary::BeforeRename)),
            ..Default::default()
        },
        webkit_exists: true,
        seen: HashSet::new(),
        revision: 0,
        initialized: false,
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
            e = reopen(&e);
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
        e = reopen(&e);
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
    let journal = std::fs::read(dir.path().join(STATE)).unwrap();
    let migration = std::fs::read(dir.path().join(MIGRATION)).ok();
    save_settings(dir.path(), r#"{"retainHistory":true}"#).unwrap();
    assert_eq!(std::fs::read(dir.path().join(STATE)).unwrap(), journal);
    assert_eq!(std::fs::read(dir.path().join(MIGRATION)).ok(), migration);
    e = reopen(&e);
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
    e = reopen(&e);
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
    e = reopen(&e);
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
    e = reopen(&e);
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
        let s = reopen(&e).snapshot().unwrap();
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
    e = reopen(&e);
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
    e = reopen(&e);
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
        e = reopen(&e);
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
    e = reopen(&e);
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
    let originals: Vec<_> = (0..40)
        .map(|i| {
            let mut v = entry(&format!("original:{i}"), "synthetic");
            v["pinned"] = json!(true);
            v
        })
        .collect();
    std::fs::write(
        dir.path().join("history.json"),
        json!(originals).to_string(),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("benchmark-reports.json"),
        json!([report(20), report(21)]).to_string(),
    )
    .unwrap();
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
    e = reopen(&e);
    assert!(!e.snapshot().unwrap().retry_notice);
    e = reopen(&e);
    let s = e.snapshot().unwrap();
    assert!(s.retry_notice);
    assert_eq!(s.history.len(), 200);
    assert_eq!(s.history.iter().filter(|v| v["pinned"] == true).count(), 20);
    assert_eq!(s.reports.len(), 10);
    assert!(s
        .history
        .iter()
        .any(|v| v["id"].as_str().unwrap().starts_with("original:")));
    assert!(s
        .reports
        .iter()
        .any(|v| v["createdAt"] == report(21)["createdAt"]));
    assert_eq!(
        e.holding().unwrap().history.entries.len()
            + e.document(Destination::History)
                .unwrap()
                .unwrap()
                .data
                .as_array()
                .unwrap()
                .len(),
        200
    );
}
#[test]
fn fresh_install_without_webkit_commits_immediately_and_never_holds() {
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), false);
    assert!(e.snapshot().unwrap().v1_committed);
    complete(&mut e, "a", true);
    assert!(e.holding().unwrap().history.entries.is_empty());
    assert_eq!(reopen(&e).snapshot().unwrap().history.len(), 1);
}
#[test]
fn retired_store_migration_file_lost_recommits_without_changing_destinations() {
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), false);
    complete(&mut e, "a", true);
    let before = std::fs::read(dir.path().join("history.json")).unwrap();
    std::fs::remove_file(dir.path().join(MIGRATION)).unwrap();
    e = reopen(&e);
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
    e = reopen(&e);
    e.import(legacy()).unwrap();
    assert_eq!(e.snapshot().unwrap().statistics, before.statistics);
    assert_eq!(e.snapshot().unwrap().history, before.history);
}
#[test]
fn migration_file_lost_before_or_after_commit_never_imports_twice() {
    for committed in [false, true] {
        let dir = TempDir::new().unwrap();
        let mut e = engine(dir.path(), true);
        if !committed {
            e.storage
                .write(
                    MIGRATION,
                    &Migration {
                        schema: 1,
                        ..Default::default()
                    },
                )
                .unwrap();
            e.storage.fail = Some((MIGRATION.into(), Boundary::BeforeRename));
        }
        assert_eq!(e.import(legacy()).is_ok(), committed);
        for _ in 0..3 {
            std::fs::remove_file(dir.path().join(MIGRATION)).unwrap();
            e = reopen(&e);
            e.import(legacy()).unwrap();
            let s = e.snapshot().unwrap();
            assert_eq!(s.history.len(), 1);
            assert_eq!(s.statistics.total_recordings, 2);
        }
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
    e = reopen(&e);
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
    let after = reopen(&e).snapshot().unwrap();
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
    assert_eq!(s, reopen(&e).snapshot().unwrap().statistics);
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
    e = reopen(&e);
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
    e = reopen(&e);
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
    e = reopen(&e);
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

#[test]
fn review_initialize_must_sync_unknown_import_before_draining() {
    for destination in [Destination::History, Destination::Reports] {
        let dir = TempDir::new().unwrap();
        let mut e = engine(dir.path(), true);
        complete(&mut e, "held", true);
        e.add_content(Destination::Reports, report(2)).unwrap();
        e.storage.fail = Some((destination.file().into(), Boundary::AfterRename));
        assert!(e.execute(Command::Import(legacy())).is_err());
        e.storage.trace.borrow_mut().clear();
        let held = std::fs::read(dir.path().join(HOLD)).unwrap();
        e.storage.fail_sync.set(true);
        assert!(e.initialize().is_err());
        assert_eq!(*e.storage.trace.borrow(), vec!["directory_sync"]);
        assert_eq!(std::fs::read(dir.path().join(HOLD)).unwrap(), held);
        e.storage.trace.borrow_mut().clear();
        e.initialize().unwrap();
        let trace = e.storage.trace.borrow();
        assert_eq!(trace[0], "directory_sync");
        assert!(
            trace
                .iter()
                .position(|v| v == "completion-holding.json:BeforeRename")
                .unwrap()
                > 0
        );
        drop(trace);
        let snapshot = e.execute(Command::Import(legacy())).unwrap();
        assert_eq!(snapshot.history.len(), 2);
        assert_eq!(snapshot.reports.len(), 2);
    }
}

#[test]
fn review_unreadable_holding_blocks_writes_and_arming_without_replacement() {
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), true);
    complete(&mut e, "held", true);
    let before = std::fs::read(dir.path().join(HOLD)).unwrap();
    e.storage.fail_read = Some(HOLD.into());
    assert!(e.initialize().is_err());
    e.storage.trace.borrow_mut().clear();
    let result = e.execute(Command::Complete {
        source: "blocked".into(),
        entry: Some(entry("blocked", "synthetic")),
        event: Some(dictation()),
    });
    assert!(matches!(result, Err(Error::Io)));
    assert_eq!(
        MigrationGate::default().tick(result.is_ok(), false, 20_000),
        GateAction::Wait
    );
    assert!(!e
        .storage
        .trace
        .borrow()
        .iter()
        .any(|v| v.contains("Rename")));
    assert_eq!(std::fs::read(dir.path().join(HOLD)).unwrap(), before);
    e.storage.fail_read = None;
    assert_eq!(e.execute(Command::Snapshot).unwrap().history.len(), 1);
}

#[test]
fn review_no_webkit_migration_initialization_failure_must_retry() {
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), true);
    e.webkit_exists = false;
    e.storage.fail = Some((MIGRATION.into(), Boundary::BeforeRename));
    assert!(e.initialize().is_err());
    assert!(
        e.execute(Command::Snapshot).unwrap().v1_committed,
        "initialization must retry the pending no-WebKit commit"
    );
}

#[test]
fn review_deleted_edited_original_must_not_return_from_legacy() {
    let dir = TempDir::new().unwrap();
    let old = legacy().history.unwrap();
    std::fs::write(dir.path().join("history.json"), old.to_string()).unwrap();
    let mut e = engine(dir.path(), true);
    let target = e.execute(Command::Snapshot).unwrap().history[0].clone();
    e.execute(Command::History(HistoryMutation::Edit {
        target,
        text: "edited synthetic".into(),
    }))
    .unwrap();
    e = reopen(&e); // Original legacy identity survives relaunch and further edits.
    let target = e.execute(Command::Snapshot).unwrap().history[0].clone();
    e.execute(Command::History(HistoryMutation::Delete {
        targets: vec![target],
    }))
    .unwrap();
    assert!(e.execute(Command::Snapshot).unwrap().history.is_empty());
    std::fs::write(dir.path().join("history.json"), "broken").unwrap();
    let s = e.execute(Command::Import(legacy())).unwrap();
    assert!(
        s.history.is_empty(),
        "deleted original returned with its pre-edit text"
    );
}

#[test]
fn pending_content_replays_once_at_every_publication_boundary() {
    for boundary in [
        Boundary::BeforeRename,
        Boundary::AfterRename,
        Boundary::DirectorySync,
    ] {
        let dir = TempDir::new().unwrap();
        let mut e = engine(dir.path(), false);
        e.storage.fail = Some(("history.json".into(), boundary));
        complete(&mut e, "pending", true);
        assert!(dir.path().join(PENDING).exists());
        let mut next = reopen(&e);
        assert_eq!(next.execute(Command::Snapshot).unwrap().history.len(), 1);
        assert!(!dir.path().join(PENDING).exists());
        assert_eq!(reopen(&next).snapshot().unwrap().history.len(), 1);
        let row = next.snapshot().unwrap().history[0].clone();
        next.execute(Command::History(HistoryMutation::Delete {
            targets: vec![row],
        }))
        .unwrap();
        assert!(reopen(&next).snapshot().unwrap().history.is_empty());
    }
}
#[test]
fn pending_retention_off_clear_and_capacity() {
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), false);
    complete(&mut e, "private", false);
    assert!(!dir.path().join(PENDING).exists());
    e.storage.fail = Some(("history.json".into(), Boundary::BeforeRename));
    complete(&mut e, "pending", true);
    let pending = std::fs::read(dir.path().join(PENDING)).unwrap();
    e.storage.fail = Some(("history.json".into(), Boundary::BeforeRename));
    assert!(e
        .execute(Command::Complete {
            source: "rejected".into(),
            entry: Some(entry("rejected", "synthetic")),
            event: None
        })
        .is_err());
    assert_eq!(std::fs::read(dir.path().join(PENDING)).unwrap(), pending);
    e.execute(Command::Clear {
        destination: Destination::History,
        at: 10,
    })
    .unwrap();
    assert!(reopen(&e).snapshot().unwrap().history.is_empty());
}
#[test]
fn explicit_barriers_seal_and_snapshot_replay() {
    let dir = TempDir::new().unwrap();
    let service = CompletionService::start(dir.path().into(), false).unwrap();
    futures_executor::block_on(service.flush()).unwrap();
    futures_executor::block_on(service.seal_and_flush()).unwrap();
    assert!(matches!(
        service.execute(Command::Snapshot),
        Err(Error::Stopped)
    ));
    let path = dir.path().join("synthetic-window.json");
    std::fs::write(
        path.with_extension("json.pending"),
        br#"{"schema":1,"value":{"visible":false}}"#,
    )
    .unwrap();
    recover_snapshot_validated(&path, |v| v["visible"].is_boolean()).unwrap();
    recover_snapshot_validated(&path, |v| v["visible"].is_boolean()).unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&std::fs::read(path).unwrap()).unwrap()["visible"],
        false
    );
}

#[test]
fn shutdown_receipt_never_hides_unpreserved_content() {
    use crate::shutdown::WriteOutcome;
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), false);
    e.storage.fail = Some((PENDING.into(), Boundary::BeforeRename));
    complete(&mut e, "unpreserved", true);
    e.storage.fail = Some((PENDING.into(), Boundary::BeforeRename));
    assert_eq!(e.write_receipt(), WriteOutcome::UnpreservedFailure);
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), false);
    e.storage.fail = Some(("history.json".into(), Boundary::BeforeRename));
    complete(&mut e, "preserved", true);
    e.storage.fail = Some(("history.json".into(), Boundary::BeforeRename));
    assert_eq!(e.write_receipt(), WriteOutcome::PreservedForReplay);
    assert_eq!(e.write_receipt(), WriteOutcome::Committed);
}

#[test]
fn malformed_snapshot_pending_stays_a_recovery_error() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("synthetic-window.json");
    std::fs::write(path.with_extension("json.pending"), b"{invalid").unwrap();
    assert!(recover_snapshot_validated(&path, |v| v["visible"].is_boolean()).is_err());
    assert!(recover_snapshot_validated(&path, |v| v["visible"].is_boolean()).is_err());
    assert!(!path.exists());
}

#[test]
fn malformed_pending_content_blocks_admission_and_never_claims_a_drain() {
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), false);
    std::fs::write(dir.path().join(PENDING), b"{invalid").unwrap();
    for _ in 0..2 {
        assert!(e
            .execute(Command::Complete {
                source: "refused".into(),
                entry: Some(entry("refused", "synthetic")),
                event: None
            })
            .is_err());
    }
    assert_eq!(
        e.write_receipt(),
        crate::shutdown::WriteOutcome::UnpreservedFailure
    );
    assert!(e.snapshot().unwrap().history.is_empty());
}

#[test]
fn pending_holding_validates_entries_before_replacing_readable_content() {
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), true);
    complete(&mut e, "valid", true);
    let before = std::fs::read(dir.path().join(HOLD)).unwrap();
    let mut holding: Value = serde_json::from_slice(&before).unwrap();
    holding["history"]["entries"] = json!([null]);
    std::fs::write(
        dir.path().join(PENDING),
        serde_json::to_vec(&json!({"schema":1,"file":HOLD,"value":holding})).unwrap(),
    )
    .unwrap();
    assert!(e.execute(Command::Snapshot).is_err());
    assert_eq!(std::fs::read(dir.path().join(HOLD)).unwrap(), before);
    assert!(e.storage.quarantined(PENDING).unwrap());
}

#[test]
fn snapshot_payload_validation_preserves_destination_and_evidence() {
    for value in [
        Value::Null,
        json!({"schema":1,"values":{"usage-dashboard-collapsed":"invalid"},"destinations":[],"v2_imported":true}),
    ] {
        let dir = TempDir::new().unwrap();
        let mut e = engine(dir.path(), false);
        e.lifecycle(lifecycle::Operation::Status).unwrap();
        let before = std::fs::read(dir.path().join("ui-state.json")).unwrap();
        std::fs::write(
            dir.path().join("ui-state.json.pending"),
            serde_json::to_vec(&json!({"schema":1,"value":value})).unwrap(),
        )
        .unwrap();
        assert!(e.lifecycle(lifecycle::Operation::Status).is_err());
        assert_eq!(
            std::fs::read(dir.path().join("ui-state.json")).unwrap(),
            before
        );
        assert!(e.storage.quarantined("ui-state.json.pending").unwrap());
    }
}

#[test]
fn startup_replays_held_completion_before_no_webkit_automatic_import() {
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), true);
    let mut h = e.holding().unwrap();
    h.history
        .entries
        .push(entry("pending", "synthetic pending"));
    e.storage
        .write(PENDING, &json!({"schema":1,"file":HOLD,"value":h}))
        .unwrap();
    let reopened = engine(dir.path(), false);
    assert_eq!(reopened.snapshot().unwrap().history.len(), 1);
    assert_eq!(
        engine(dir.path(), false).snapshot().unwrap().history.len(),
        1
    );
}

#[test]
fn abandoned_pending_temps_and_resolved_evidence_obey_clear_delete_and_caps() {
    for mutation in [0, 1, 2, 3] {
        let dir = TempDir::new().unwrap();
        let mut e = engine(dir.path(), false);
        let snapshot = complete(&mut e, "valid", true);
        let temp = dir.path().join(format!("{PENDING}.tmp-synthetic"));
        let evidence = dir.path().join(format!(
            "{PENDING}.quarantine-seq-00000000000000000001-synthetic"
        ));
        std::fs::write(&temp, b"synthetic transcript").unwrap();
        std::fs::write(&evidence, b"synthetic transcript").unwrap();
        e.execute(Command::RecoverPending).unwrap();
        assert!(evidence.exists());
        assert!(!temp.exists());
        match mutation {
            0 => {
                e.execute(Command::Clear {
                    destination: Destination::History,
                    at: 2,
                })
                .unwrap();
            }
            1 => {
                e.execute(Command::History(HistoryMutation::Delete {
                    targets: snapshot.history,
                }))
                .unwrap();
            }
            2 => {
                complete(&mut e, "new cap", true);
            }
            _ => {
                complete(&mut e, "not retained", false);
            }
        }
        assert!(!evidence.exists());
        assert!(!temp.exists());
    }
}

#[test]
fn failed_report_and_ui_intents_are_retained_until_exact_publication_recovers() {
    for ui in [false, true] {
        let dir = TempDir::new().unwrap();
        let mut e = engine(dir.path(), false);
        e.lifecycle(lifecycle::Operation::Status).unwrap();
        let pending = if ui { "ui-state.json.pending" } else { PENDING };
        e.storage.fail = Some((pending.into(), Boundary::BeforeRename));
        let result = if ui {
            e.lifecycle(lifecycle::Operation::Save {
                key: "usage-dashboard-collapsed".into(),
                value: json!(true),
            })
            .map(|_| ())
        } else {
            e.execute(Command::Report(report(1))).map(|_| ())
        };
        assert!(result.is_err());
        e.storage.fail = Some((pending.into(), Boundary::BeforeRename));
        assert_eq!(
            e.write_receipt(),
            crate::shutdown::WriteOutcome::UnpreservedFailure
        );
        assert_eq!(e.write_receipt(), crate::shutdown::WriteOutcome::Committed);
        if ui {
            assert_eq!(
                e.lifecycle(lifecycle::Operation::Status).unwrap().values
                    ["usage-dashboard-collapsed"],
                true
            );
        } else {
            assert_eq!(e.snapshot().unwrap().reports.len(), 1);
        }
    }
}

#[test]
fn explicit_pending_recovery_survives_restart_without_replaying_rejected_evidence() {
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), false);
    complete(&mut e, "valid", true);
    std::fs::write(dir.path().join(PENDING), b"{damaged synthetic").unwrap();
    assert!(e.execute(Command::Snapshot).is_err());
    e.execute(Command::RecoverPending).unwrap();
    assert!(e.storage.quarantined(PENDING).unwrap());
    let mut next = engine(dir.path(), false);
    assert_eq!(next.execute(Command::Snapshot).unwrap().history.len(), 1);
    assert_eq!(
        next.write_receipt(),
        crate::shutdown::WriteOutcome::Committed
    );
    next.execute(Command::Clear {
        destination: Destination::History,
        at: 3,
    })
    .unwrap();
    assert!(!next.storage.quarantined(PENDING).unwrap());
    assert!(reopen(&next).snapshot().unwrap().history.is_empty());
}

#[test]
fn explicit_pending_recovery_through_public_service_across_two_launches() {
    let dir = TempDir::new().unwrap();
    let service = CompletionService::start(dir.path().into(), false).unwrap();
    service.execute(Command::Snapshot).unwrap();
    service
        .execute(Command::Complete {
            source: "synthetic-service".into(),
            entry: Some(entry("service", "synthetic")),
            event: None,
        })
        .unwrap();
    drop(service);
    std::fs::write(dir.path().join(PENDING), b"{damaged synthetic").unwrap();
    let service = CompletionService::start(dir.path().into(), false).unwrap();
    assert!(service.execute(Command::Snapshot).is_err());
    assert_eq!(
        service
            .execute(Command::RecoverPending)
            .unwrap()
            .history
            .len(),
        1
    );
    drop(service);
    let service = CompletionService::start(dir.path().into(), false).unwrap();
    assert_eq!(service.execute(Command::Snapshot).unwrap().history.len(), 1);
    assert_eq!(
        futures_executor::block_on(service.shutdown_receipt()),
        crate::shutdown::WriteOutcome::Committed
    );
}

#[test]
fn explicit_recovery_replays_usable_pending_instead_of_discarding_it() {
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), false);
    e.storage.fail = Some(("history.json".into(), Boundary::BeforeRename));
    complete(&mut e, "accepted", true);
    assert!(e.storage.root.join(PENDING).exists());
    assert_eq!(e.execute(Command::RecoverPending).unwrap().history.len(), 1);
    assert!(!e.storage.quarantined(PENDING).unwrap());
    assert_eq!(reopen(&e).snapshot().unwrap().history.len(), 1);
}

#[test]
fn report_failure_before_pending_admission_retains_exact_held_report() {
    let dir = TempDir::new().unwrap();
    let mut e = engine(dir.path(), true);
    // A readable pre-migration destination makes cap_held publish evictions
    // before its pending Holding intent is constructed.
    e.storage
        .write(
            "benchmark-reports.json",
            &Document::new(Destination::Reports),
        )
        .unwrap();
    e.storage.fail = Some(("benchmark-reports.json".into(), Boundary::BeforeRename));
    assert!(e.execute(Command::Report(report(1))).is_err());
    assert!(e.failed_content.is_some());
    e.storage.fail = Some(("benchmark-reports.json".into(), Boundary::BeforeRename));
    assert_eq!(
        e.write_receipt(),
        crate::shutdown::WriteOutcome::UnpreservedFailure
    );
    assert_eq!(e.write_receipt(), crate::shutdown::WriteOutcome::Committed);
    assert_eq!(e.snapshot().unwrap().reports.len(), 1);
}

#[test]
fn direct_mutation_failures_cannot_receive_committed_until_exact_snapshot_is_retried() {
    for clear in [false, true] {
        for boundary in [
            Boundary::BeforeRename,
            Boundary::AfterRename,
            Boundary::DirectorySync,
        ] {
            let dir = TempDir::new().unwrap();
            let mut e = engine(dir.path(), false);
            let target = complete(&mut e, "original", true).history[0].clone();
            e.storage.fail = Some(("history.json".into(), boundary));
            let command = if clear {
                Command::Clear {
                    destination: Destination::History,
                    at: 5,
                }
            } else {
                Command::History(HistoryMutation::Pin { target })
            };
            assert!(e.execute(command).is_err());
            e.storage.fail = Some(("history.json".into(), Boundary::BeforeRename));
            assert_eq!(
                e.write_receipt(),
                crate::shutdown::WriteOutcome::UnpreservedFailure
            );
            assert_eq!(e.write_receipt(), crate::shutdown::WriteOutcome::Committed);
            let history = reopen(&e).snapshot().unwrap().history;
            if clear {
                assert!(history.is_empty());
            } else {
                assert_eq!(history.len(), 1);
                assert_eq!(history[0]["pinned"], true);
            }
        }
    }
}
