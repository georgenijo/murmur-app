//! D-S2-4, D-S2-6, D-S2-7, D-S2-8: work a listener queues inside a core span.
//!
//! Every RAII owner dropped inside a span (`PipelineOwner`, `ContinuationGuard`,
//! `IdleGuard`) joins one FIFO and runs when the outermost entry drains, after D is
//! released. These tests stand a drain guard before the span (what every entry
//! declares before T and D) and observe the queue and the history around it.
use super::*;
use crate::core_history::ContinuationGuard;
use crate::dictation_executor::executor::CoreEntry;
use murmur_dictation_core::decision::{ContKind, ContinuationSpec};
use murmur_dictation_core::ids::RecordingId;
use std::sync::atomic::AtomicBool;

fn spec(rid: u64, kind: ContKind) -> ContinuationSpec {
    ContinuationSpec {
        recording_id: RecordingId(rid),
        kind,
    }
}

fn queued() -> Vec<(u64, bool)> {
    DEFERRED_FINISHES.with(|queue| {
        queue
            .borrow()
            .iter()
            .map(|entry| (entry.recording_id, entry.retire_only))
            .collect()
    })
}

/// D-S2-7: a finish deferred inside a span carries its idle emit as data and makes
/// it at the drain; the frontend `idle` is not lost.
#[test]
fn a_deferred_explicit_finish_delivers_its_idle_emit_at_the_drain() {
    let app_state = AppState::default();
    let rid = app_state.next_recording_id();
    app_state.dictation.lock_or_recover().status = DictationStatus::Processing;
    let emitted = Arc::new(AtomicBool::new(false));
    let mut guard = IdleGuard::for_legacy_parse(&app_state, rid);
    let drain = DrainDeferredFinishes::new(&app_state);
    {
        let span = CoreEntry::enter().expect("outside any span");
        let flag = Arc::clone(&emitted);
        guard.finish_owned(move |dictation| {
            // Made under D at the drain, with the finish's own write already done.
            assert_eq!(dictation.status, DictationStatus::Idle);
            flag.store(true, Ordering::SeqCst);
            true
        });
        assert!(
            !emitted.load(Ordering::SeqCst),
            "nothing is made in the span"
        );
        assert_eq!(
            app_state.dictation.lock_or_recover().status,
            DictationStatus::Processing
        );
        assert_eq!(queued(), [(rid, false)]);
        drop(span);
    }
    drop(drain);
    assert!(
        emitted.load(Ordering::SeqCst),
        "the deferred finish made its idle emit"
    );
    assert_eq!(
        app_state.dictation.lock_or_recover().status,
        DictationStatus::Idle
    );
    assert!(queued().is_empty());
}

/// D-S2-7, the stale case: the drain's own D-protected generation check still
/// applies, so a newer recording's status is left alone and no idle is made.
#[test]
fn a_deferred_finish_for_a_superseded_recording_makes_no_idle_emit() {
    let app_state = AppState::default();
    let rid = app_state.next_recording_id();
    app_state.dictation.lock_or_recover().status = DictationStatus::Processing;
    let emitted = Arc::new(AtomicBool::new(false));
    let mut guard = IdleGuard::for_legacy_parse(&app_state, rid);
    let drain = DrainDeferredFinishes::new(&app_state);
    {
        let span = CoreEntry::enter().expect("outside any span");
        let flag = Arc::clone(&emitted);
        guard.finish_owned(move |_| {
            flag.store(true, Ordering::SeqCst);
            true
        });
        // A newer recording starts before the drain.
        let newer = app_state.next_recording_id();
        let mut dictation = app_state.dictation.lock_or_recover();
        crate::dictation_executor::FieldAdapter::set_status(
            newer,
            &mut dictation,
            DictationStatus::Recording,
        );
        drop(dictation);
        drop(span);
    }
    drop(drain);
    assert!(!emitted.load(Ordering::SeqCst));
    assert_eq!(
        app_state.dictation.lock_or_recover().status,
        DictationStatus::Recording
    );
}

/// D-S2-10(a), Sol's scenario: a listener queues an `IdleGuard` finish, then drops a
/// start parked on context capture (a bare `ContinuationGuard`) and a pipeline's
/// owner. All three keep FIFO order; nothing retires inside the span; after the
/// drain every entry is gone, each H1 retired exactly once.
#[test]
fn every_owner_dropped_in_a_span_retires_in_fifo_order_at_the_drain() {
    let app_state = AppState::default();
    let history = app_state.core_history.clone();
    let finishing = PipelineOwner::register(&history, spec(2, ContKind::Pipeline));
    let mut finisher = IdleGuard::for_pipeline(&app_state, 2, Some(finishing));
    let parked_start = ContinuationGuard::register(&history, spec(3, ContKind::ResolveContext));
    let dropped_flight = PipelineOwner::register(&history, spec(4, ContKind::Pipeline));
    assert_eq!(
        (history.pipeline_count(), history.outstanding_ops()),
        (2, 3)
    );
    let counters = history.counters();
    let drain = DrainDeferredFinishes::new(&app_state);
    {
        let span = CoreEntry::enter().expect("outside any span");
        // The finish is queued first, with its owner.
        finisher.finish_owned(|_| true);
        drop(parked_start);
        drop(dropped_flight);
        assert_eq!(
            (history.pipeline_count(), history.outstanding_ops()),
            (2, 3),
            "nothing retires inside the span"
        );
        assert_eq!(queued(), [(2, false), (3, true), (4, true)], "FIFO");
        drop(span);
    }
    drop(drain);
    assert_eq!(
        (history.pipeline_count(), history.outstanding_ops()),
        (0, 0)
    );
    assert_eq!(history.counters(), counters, "each op retired exactly once");
    assert!(queued().is_empty());
}

/// Outside a span every owner still retires at once (the base behaviour).
#[test]
fn outside_a_span_owners_retire_immediately() {
    let app_state = AppState::default();
    let history = app_state.core_history.clone();
    let op = ContinuationGuard::register(&history, spec(3, ContKind::ResolveContext));
    let owner = PipelineOwner::register(&history, spec(4, ContKind::Pipeline));
    assert_eq!(
        (history.pipeline_count(), history.outstanding_ops()),
        (1, 2)
    );
    drop(op);
    assert_eq!(history.outstanding_ops(), 1);
    drop(owner);
    assert_eq!(
        (history.pipeline_count(), history.outstanding_ops()),
        (0, 0)
    );
    assert!(queued().is_empty());
}

/// D-S2-8: an outermost entry that ends with deferred work and no drain guard fails
/// in test builds, so an entry that misses its guard cannot pass unnoticed.
#[test]
#[should_panic(expected = "no drain guard alive")]
fn an_entry_without_a_drain_guard_fails_the_safety_net() {
    let app_state = AppState::default();
    let history = app_state.core_history.clone();
    let op = ContinuationGuard::register(&history, spec(3, ContKind::ResolveContext));
    // No drain guard on this thread. Clear the queue before the panic unwinds so
    // the leftover cannot leak into another test on this thread.
    struct Clear;
    impl Drop for Clear {
        fn drop(&mut self) {
            DEFERRED_FINISHES.with(|queue| queue.borrow_mut().clear());
        }
    }
    let _clear = Clear;
    let span = CoreEntry::enter().expect("outside any span");
    drop(op);
    drop(span);
}

/// D-S2-12: the stop's idle-status measurement ends when the 204b D section ends,
/// before any deferred drain. A finish queued earlier (its idle emit sleeps) runs
/// in the drain `run_pipeline_idle_emit` makes on return; the instant it returns is
/// before that drain.
#[test]
fn the_idle_measurement_ends_before_the_deferred_drain() {
    use murmur_dictation_core::inputs::PostKind;
    let app_state = AppState::default();
    let rid = app_state.next_recording_id();
    app_state.dictation.lock_or_recover().status = DictationStatus::Processing;
    let mut earlier = IdleGuard::for_legacy_parse(&app_state, rid);
    {
        let outer = DrainDeferredFinishes::new(&app_state);
        let span = CoreEntry::enter().expect("outside any span");
        earlier.finish_owned(|_| {
            std::thread::sleep(std::time::Duration::from_millis(150));
            true
        });
        drop(span);
        // The test's own guard must not drain it: the entry under test does.
        std::mem::forget(outer);
        DRAINS_ALIVE.with(|alive| alive.set(alive.get().saturating_sub(1)));
    }
    assert_eq!(queued(), [(rid, false)]);
    let ended = run_pipeline_idle_emit(&app_state, rid, PostKind::StopPost, &|_| {}, &|| true);
    let returned = std::time::Instant::now();
    assert!(
        returned.duration_since(ended) >= std::time::Duration::from_millis(100),
        "the drain (150 ms of idle emit) ran after the stamp: {:?}",
        returned.duration_since(ended)
    );
    assert!(queued().is_empty());
}
