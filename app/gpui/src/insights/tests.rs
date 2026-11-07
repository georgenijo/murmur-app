use super::fixtures;
#[test]
fn local_calendar_rollover_empty_days_and_streak() {
    let mut s = fixtures::snapshot();
    assert_eq!(s.recent(7).len(), 7);
    assert_eq!(s.heatmap().len(), 56);
    assert_eq!(s.streak(), 1);
    s.today = s.today.succ_opt().unwrap();
    assert_eq!(s.streak(), 1);
    s.today = s.today.succ_opt().unwrap();
    assert_eq!(s.streak(), 0);
    assert_eq!(s.recent(1)[0].words, 0);
}
#[test]
fn aggregates_and_missing_modes() {
    let mut s = fixtures::snapshot();
    assert_eq!(
        s.totals()
            .iter()
            .find(|r| r.0 == "Average speed")
            .unwrap()
            .1,
        "120"
    );
    s.mode_names.clear();
    assert_eq!(
        s.totals()
            .iter()
            .find(|r| r.0 == "Most-used mode")
            .unwrap()
            .1,
        "Removed mode"
    );
    s.stats.total_duration_seconds = 1e9;
    assert_eq!(
        s.totals()
            .iter()
            .find(|r| r.0 == "Estimated typing time saved")
            .unwrap()
            .1,
        "0.0 min"
    );
}
