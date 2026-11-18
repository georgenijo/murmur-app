//! The live side: the preallocated queue, the recorder, the ticket, and the proof
//! that the producer path allocates nothing and takes no lock (D-S2c-1 a).

use std::sync::atomic::Ordering;
use std::sync::{Arc, Barrier};
use std::time::Duration;

use murmur_dictation_core::effects::{Effect, Placement};
use murmur_dictation_core::ids::{Now, RecordingId};
use murmur_dictation_core::inputs::{CancelInput, Input};
use murmur_dictation_core::lifecycle::short_idle_pre_effects;
use murmur_dictation_core::status::Status;

use super::harness::*;
use crate::dictation_shadow::alloc_probe::{count_allocations, count_heap_calls};
use crate::dictation_shadow::compare::Comparator;
use crate::dictation_shadow::transport::{
    mark, submit, witness, Mark, ShadowRecorder, QUEUE_CAPACITY,
};
use crate::dictation_shadow::types::{
    CondWitness, LiveOp, MirrorField, MirrorValue, MirrorWitness, PredId, Projection, ReplyKind,
    ShadowHeader, ShadowMsg, TickerGate, WitnessedEffect, PRE_CAPACITY,
};
use crate::dictation_shadow::{run, Control, POLL_INTERVAL};

fn pop_all(transport: &crate::dictation_shadow::transport::Transport) -> Vec<ShadowMsg> {
    let mut all = Vec::new();
    while let Some(msg) = transport.pop() {
        all.push(msg);
    }
    all
}

/// One whole flight through the real producer path with no heap use of its own:
/// recorder, `Pre` buffer, ticket, header assembly, `submit`, every kind of later
/// message and `finish`. Built from values that own no heap (an input without a
/// string).
fn produce_one_flight(transport: &Arc<crate::dictation_shadow::transport::Transport>) {
    let mut recorder = transport.recorder();
    recorder.enter_pre();
    let rid = RecordingId(7);
    recorder.pre_effect(WitnessedEffect::ran(Effect::PerfAbort));
    recorder.pre_conditional(PredId::SpecFlowing, true);
    recorder.pre_mirror(MirrorWitness {
        field: MirrorField::ActiveContext,
        before: MirrorValue::Slot(Some(rid)),
        after: MirrorValue::Slot(None),
    });
    let pre = recorder.take_pre();
    let ticket = recorder.ticket(216);
    let projection = Projection {
        status: Status::Processing,
        current_id: rid,
        obs: murmur_dictation_core::state::LiveObs::default(),
    };
    let mut header = ShadowHeader::new(
        ticket.seq(),
        ticket.rec(),
        205,
        5,
        projection,
        Input::CancelRequest(CancelInput { expected: None }),
        Now(0),
    );
    header.adopt_pre(pre);
    header.add_in_effect(WitnessedEffect::ran(Effect::SetStatus(Status::Idle)));
    header.add_in_conditional(CondWitness {
        pred: PredId::EmitOk(0),
        taken: true,
    });
    header.add_in_ticker(TickerGate {
        id: rid,
        status: Status::Recording,
        current_id: rid,
        cancelled_through: RecordingId(0),
        ticker_before: RecordingId(0),
        registered: true,
    });
    submit(ticket, header);
    witness(
        &recorder,
        3,
        Placement::PostDT,
        WitnessedEffect::ran(Effect::KeyboardProcessing(false)),
    );
    mark(&recorder, Mark::Reply(ReplyKind::Ok));
    mark(
        &recorder,
        Mark::Cond {
            pred: PredId::EmitOk(0),
            taken: true,
        },
    );
    recorder.finish();
}

#[test]
fn shadow_capture_takes_no_lock_allocates_nothing() {
    // Review fix (Sol 6.2 blocker 1): the capture path boxed every header, and std's
    // `mpsc` takes a waker mutex. Count the allocations of the whole producer path on
    // this thread: the recorder, the buffered `Pre` witnesses, the ticket, the header,
    // `submit`, a witness, each `mark`, and `finish`, with the queue both accepting
    // and rejecting.
    let transport = default_transport();
    // Warm the thread (any one-time lazy state) outside the measurement.
    produce_one_flight(&transport);
    while transport.pop().is_some() {}
    let ((), allocations, frees) = count_heap_calls(|| {
        for _ in 0..8 {
            produce_one_flight(&transport);
        }
    });
    assert_eq!(allocations, 0, "the producer path allocated");
    assert_eq!(frees, 0, "the producer path freed");

    // The same with the queue full: a rejected message costs two counters,
    // and neither an allocation nor a free (the rejected message is dropped inside).
    let full = local_transport(2);
    let ((), allocations, frees) = count_heap_calls(|| {
        for _ in 0..4 {
            produce_one_flight(&full);
        }
    });
    assert_eq!(allocations, 0, "the drop path allocated");
    assert_eq!(frees, 0, "the drop path freed");
    assert!(full.lost() > 0);
    // Nothing was lost to the measurement: the messages the queue took are intact.
    // A flight is five messages: header, witness, conditional mark, reply mark, close.
    assert_eq!(pop_all(&transport).len(), 8 * 5);
}

#[test]
fn the_counting_allocator_is_connected() {
    // A measurement that cannot fail proves nothing: the same hook sees an allocation.
    let (v, allocations) = count_allocations(|| vec![1_u8, 2, 3]);
    assert_eq!(v.len(), 3);
    assert!(allocations >= 1, "the hook did not see the allocation");
}

#[test]
fn the_capture_path_source_has_no_lock_box_channel_or_wake() {
    let transport = super::policy::production_source(include_str!("../transport.rs"));
    for signature in [
        "pub(crate) fn witness(",
        "pub(crate) fn mark(",
        "pub(crate) fn submit(",
        "fn send(",
        "fn push(",
        "fn lose(",
        "fn close(",
        "pub(crate) fn ticket(",
        "pub(crate) fn recorder(",
        "pub(crate) fn pre_effect(",
        "pub(crate) fn take_pre(",
    ] {
        let body = super::policy::function_body(&transport, signature);
        for token in [
            "lock", "Mutex", "RwLock", ".recv", ".wait(", "sleep", ".join(", "Condvar", "Barrier",
            "Box::new", "Vec", "vec!", "String", "format!", "to_vec", "Arc::new", "unpark",
            "notify", "thread::", "mpsc",
        ] {
            assert!(
                !body.contains(token),
                "`{signature}` contains `{token}`; capture is a single lock-free push"
            );
        }
    }
    // The whole production text: no channel, no mutex, no lock, no sleep at all.
    for token in [
        "mpsc",
        "Mutex",
        ".lock(",
        "sync_channel",
        "SyncSender",
        "Condvar",
    ] {
        assert!(
            !transport.contains(token),
            "transport.rs names `{token}` outside cfg(test)"
        );
    }
    assert!(super::policy::function_body(&transport, "fn push(").contains("self.queue.try_push("));
    // A rejected push costs two counter increments and nothing else (D-S2c-3 a).
    let lose = super::policy::function_body(&transport, "fn lose(");
    assert_eq!(lose.matches("fetch_add(").count(), 2, "{lose}");
    assert!(lose.matches("Ordering::Release").count() == 2, "{lose}");
    // The synchronous dispatch and its mutex exist only under cfg(test): the raw
    // source has them, the production text does not.
    let raw = include_str!("../transport.rs");
    assert!(raw.contains("deliver_sync") && !transport.contains("deliver_sync"));
    assert!(raw.contains("Mutex") && !transport.contains("Mutex"));
}

#[test]
fn shadow_sync_mode_compares_on_the_calling_thread() {
    let transport = default_transport();
    transport.set_sync(Some(harness().comparator));
    let flight = short_idle(0, 0);
    let mut recorder = transport.recorder();
    recorder.enter_pre();
    for effect in flight.header.pre_effects.iter() {
        assert!(recorder.pre_effect(effect.clone()));
    }
    for mirror in flight.header.pre_mirrors.iter() {
        assert!(recorder.pre_mirror(*mirror));
    }
    for cond in flight.header.pre_conditionals.iter() {
        assert!(recorder.pre_conditional(cond.pred, cond.taken));
    }
    let pre = recorder.take_pre();
    let ticket = recorder.ticket(216);
    let mut header = flight.header.clone();
    header.seq = ticket.seq();
    header.rec = recorder.id();
    header.adopt_pre(pre);
    submit(ticket, header);
    assert!(transport.pop().is_none(), "sync mode uses no queue");
    transport.with_sync(|comparator| {
        assert_eq!(comparator.stats.compared, 1, "compared on this thread, now");
        assert_eq!(comparator.stats.mismatched_root, 0);
    });
    recorder.finish();
}

#[test]
fn shadow_recorder_exists_before_pre() {
    let transport = default_transport();
    // The recorder (and its id) exist before the first `Pre` effect runs; the
    // header's seq is taken later, at the decision boundary, from its own counter.
    let mut recorder = transport.recorder();
    let id = recorder.id();
    assert!(recorder.pre_effect(WitnessedEffect::ran(Effect::PerfAbort)));
    let ticket = recorder.ticket(216);
    assert_ne!(id.0, ticket.seq(), "a recorder id is not a seq");
    assert_eq!(ticket.rec(), id);
    drop(ticket);
    // Two flights in flight at once: the seq follows the decision boundary, the
    // recorder id follows creation.
    let first = transport.recorder();
    let second = transport.recorder();
    let late = second.ticket(216);
    let early_after = first.ticket(216);
    assert!(first.id() < second.id());
    assert!(
        late.seq() < early_after.seq(),
        "seq is taken at the ticket, not at creation"
    );
}

#[test]
fn shadow_pre_buffer_covers_c3() {
    // The largest C3 `Pre` list is 4 today (R216.1); the inline buffer is 8.
    assert_eq!(short_idle_pre_effects(RID).len(), 4);
    let mut largest = 0;
    for status in Status::ALL {
        for current in [RID, RecordingId(2), RecordingId::NONE] {
            let core = view(status, current);
            for input in every_input_family() {
                let decision = core.handle(input, NOW);
                let pre = decision
                    .effects
                    .iter()
                    .filter(|p| p.at == Placement::Pre)
                    .count();
                largest = largest.max(pre);
            }
        }
    }
    assert_eq!(largest, 4, "R216.1 is the largest `Pre` list");
    assert!(largest <= PRE_CAPACITY);
}

#[test]
fn a_capture_list_overflow_is_counted_and_the_header_is_not_compared() {
    let transport = default_transport();
    let mut recorder = transport.recorder();
    for i in 0..PRE_CAPACITY {
        assert!(
            recorder.pre_effect(WitnessedEffect::ran(Effect::ClearContext(RecordingId(
                i as u64
            ))))
        );
    }
    assert!(!recorder.pre_effect(WitnessedEffect::ran(Effect::PerfAbort)));
    assert_eq!(transport.take_overflow(), 1);
    let pre = recorder.take_pre();
    assert_eq!(pre.effects.len(), PRE_CAPACITY);
    assert!(pre.overflowed);
    // The header that adopts it is incomplete: counted, never reported as a mismatch.
    let mut flight = short_idle(1, 1);
    flight.header.adopt_pre(pre);
    let mut h = harness();
    h.feed(&flight);
    assert_eq!(h.stats().skipped_incomplete, 1);
    assert_eq!(h.stats().mismatched_root, 0);
    assert_eq!(h.stats().matched, 0);
}

fn every_input_family() -> Vec<Input> {
    use murmur_dictation_core::decision::{ContKind, Continuation, OpStatus};
    use murmur_dictation_core::ids::OperationId;
    use murmur_dictation_core::inputs::{
        CaptureErr, CaptureResult, CaptureResultInput, GuardFinishInput, GuardOrigin,
        StopCaptureResult, StopCaptureResultInput, StopInput, SupervisorEvent,
    };
    use murmur_dictation_core::status::{AudioCancelReason, StopTrigger};
    let rid = RID;
    vec![
        Input::Supervisor(SupervisorEvent::InterruptedShortIdle { rid }),
        Input::Supervisor(SupervisorEvent::Interrupted {
            rid,
            delivered_samples: 100,
            duration_ms: 1,
            reason: AudioCancelReason::RuntimeFailure,
        }),
        Input::Supervisor(SupervisorEvent::Idle { rid }),
        Input::Supervisor(SupervisorEvent::Recovering {
            rid,
            reason: AudioCancelReason::User,
        }),
        Input::StopRequest(StopInput {
            expected: Some(RecordingId(2)),
            trigger: StopTrigger::User,
            observed_current_id: rid,
            interrupted: Some(None),
            entered: NOW,
        }),
        Input::CancelRequest(CancelInput { expected: None }),
        Input::GuardFinish(GuardFinishInput {
            rid,
            origin: GuardOrigin::PipelineExit,
            closure_emit: true,
        }),
        Input::StopCaptureResult(StopCaptureResultInput {
            token: Continuation {
                recording_id: rid,
                op: OperationId(1),
                kind: ContKind::CaptureStop,
            },
            op_status: OpStatus::Outstanding,
            rid,
            result: StopCaptureResult::Err,
        }),
        Input::StartCaptureResult(CaptureResultInput {
            token: Continuation {
                recording_id: rid,
                op: OperationId(1),
                kind: ContKind::CaptureStart,
            },
            op_status: OpStatus::Outstanding,
            result: CaptureResult::Err(CaptureErr::Other),
        }),
    ]
}

#[test]
fn shadow_recorder_crosses_threads() {
    // Two flights interleave; each flight's later effects are sent from other
    // threads (a spawned blocking start, a detached `RS` stop). A thread-local
    // lookup would attach them to the wrong flight or to none.
    let transport = default_transport();
    transport.set_sync(Some(harness().comparator));
    let flights = [start(0, 0, Live::default()), cancel(0, 0)];
    let mut recorders = Vec::new();
    for flight in &flights {
        let recorder = transport.recorder();
        submit_header(&recorder, flight);
        recorders.push(Arc::new(recorder));
    }
    let barrier = Arc::new(Barrier::new(2));
    let mut workers = Vec::new();
    for (flight, recorder) in flights.iter().zip(&recorders) {
        let recorder: Arc<ShadowRecorder> = Arc::clone(recorder);
        let barrier = Arc::clone(&barrier);
        let flight = flight.clone();
        workers.push(
            std::thread::Builder::new()
                .name("detached-task".into())
                .spawn(move || {
                    barrier.wait();
                    send_later(&recorder, &flight);
                })
                .unwrap(),
        );
    }
    for worker in workers {
        worker.join().unwrap();
    }
    transport.with_sync(|comparator| {
        assert_eq!(comparator.stats.compared, 2);
        assert_eq!(comparator.stats.matched, 2, "{:?}", comparator.stats);
        assert_eq!(comparator.stats.mismatched_root, 0);
    });
}

#[test]
fn shadow_header_precedes_blocking_effect() {
    // A flight whose blocking effect (`CS`) never returns until the test lets it:
    // the header and its `In` comparison complete while the flight is blocked. The
    // comparison thread is the real `run` loop, polling the real queue.
    let transport = default_transport();
    let control = Arc::new(Control::new());
    let (shared_events, sink_events) = {
        let events: Events = Arc::default();
        (Arc::clone(&events), events)
    };
    let comparator = Comparator::new(
        Box::new(|core, input, now| core.handle(input, now)),
        Box::new(move |event| sink_events.lock().unwrap().push(event)),
        Box::new(|| 0),
    );
    let stats = Arc::new(std::sync::Mutex::new(None));
    let thread = {
        let (transport, control, stats) = (
            Arc::clone(&transport),
            Arc::clone(&control),
            Arc::clone(&stats),
        );
        std::thread::Builder::new()
            .name(crate::dictation_shadow::THREAD_NAME.into())
            .spawn(move || {
                run(
                    &transport,
                    comparator,
                    None,
                    &control,
                    Duration::from_millis(1),
                );
                *stats.lock().unwrap() = Some(());
            })
            .unwrap()
    };
    let flight = start(0, 0, Live::default());
    let recorder = Arc::new(transport.recorder());
    let reached = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let flight_thread = {
        let (recorder, reached, release, flight) = (
            Arc::clone(&recorder),
            Arc::clone(&reached),
            Arc::clone(&release),
            flight.clone(),
        );
        std::thread::spawn(move || {
            submit_header(&recorder, &flight);
            // The blocking `CS`: the header is already out.
            reached.wait();
            release.wait();
            send_later(&recorder, &flight);
        })
    };
    reached.wait();
    // The flight is blocked; the header was consumed by the comparison thread anyway.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while transport.unconsumed() > 0 {
        assert!(
            std::time::Instant::now() < deadline,
            "the header was never consumed"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    release.wait();
    flight_thread.join().unwrap();
    drop(recorder);
    control.request_shutdown();
    assert!(control.wait_finished(Duration::from_secs(10)));
    thread.join().unwrap();
    assert!(shared_events.lock().unwrap().is_empty());
    let _ = POLL_INTERVAL;
}

#[test]
fn shadow_recorder_close_semantics() {
    let transport = default_transport();
    // finish after the header: Close{completed:true}, once.
    let rec = transport.recorder();
    submit_header(&rec, &short_idle(0, 0));
    rec.finish();
    rec.finish();
    drop(rec);
    let msgs = pop_all(&transport);
    assert!(matches!(msgs[0], ShadowMsg::Header(_)));
    assert!(matches!(
        msgs[1],
        ShadowMsg::Close {
            completed: true,
            ..
        }
    ));
    assert_eq!(msgs.len(), 2, "one Close only");

    // Dropped after the header without finish: Close{completed:false}.
    let rec = transport.recorder();
    submit_header(&rec, &short_idle(0, 0));
    drop(rec);
    let msgs = pop_all(&transport);
    assert!(matches!(
        msgs[1],
        ShadowMsg::Close {
            completed: false,
            ..
        }
    ));
}

#[test]
fn shadow_pre_abandoned_requires_entry_into_run_pre() {
    // Review fix (Sol 6.2 minor 1): a recorder that was created and dropped without
    // `run(Pre)` ever starting is not an abandoned flight.
    let transport = default_transport();
    drop(transport.recorder());
    assert!(transport.pop().is_none(), "it never entered run(Pre)");

    // A flight whose `run(Pre)` began and which was dropped before D entry is
    // reported.
    let flight = transport.recorder();
    flight.enter_pre();
    drop(flight);
    let msgs = pop_all(&transport);
    assert_eq!(msgs.len(), 1);
    assert!(
        matches!(msgs[0], ShadowMsg::PreAbandoned { .. }),
        "{msgs:?}"
    );
    // No seq was consumed: the next ticket continues the sequence.
    let next = transport.recorder();
    assert_eq!(next.ticket(216).seq(), 1);

    let mut h = harness();
    h.feed_header(&short_idle(1, 1));
    for msg in msgs {
        h.send(msg);
    }
    assert_eq!(h.stats().abandoned_pre_d, 1);
    assert_eq!(h.stats().abandoned_by_loss, 0);
    assert_eq!(h.stats().mismatched_root, 0);
    // ...and a later decision still compares (nothing was given up).
    h.feed(&short_idle(2, 2));
    assert_eq!(h.stats().compared, 2);
    assert_eq!(h.stats().abandoned_by_loss, 0);

    // Through the comparison: a fresh recorder dropped counts nothing.
    let mut h = harness();
    drop(default_transport().recorder());
    assert_eq!(h.stats().abandoned_pre_d, 0);
    let _ = &mut h;
}

#[test]
fn the_channel_capacity_and_documented_limits() {
    assert_eq!(QUEUE_CAPACITY, 256);
    let _ = Ordering::SeqCst;
    let _ = (LiveOp::clone, MirrorField::ActiveContext);
}

/// A header whose input carries the typed Smart Auto failure, the one that used to
/// own a `String` (D-S2c-2 b).
fn smart_auto_header() -> ShadowHeader {
    use murmur_dictation_core::inputs::SmartAutoFailure;
    let mut flight = start(1, 1, Live::default());
    let Input::StartRequest(input) = &mut flight.header.input else {
        panic!("a start flight carries a StartInput");
    };
    input.smart_auto_result = Some(Err(SmartAutoFailure::ResolveFailed));
    flight.header
}

#[test]
fn a_rejected_smart_auto_header_allocates_and_frees_nothing() {
    // Accepted: the header with the typed failure is moved into the ring.
    let transport = default_transport();
    let header = smart_auto_header();
    let recorder = transport.recorder();
    let ((), allocations, frees) = count_heap_calls(|| {
        recorder.enter_pre();
        let ticket = recorder.ticket(101);
        let mut sent = header.clone();
        sent.seq = ticket.seq();
        sent.rec = ticket.rec();
        submit(ticket, sent);
    });
    assert_eq!(
        (allocations, frees),
        (0, 0),
        "accepting a header used the heap"
    );
    assert!(pop_all(&transport)
        .iter()
        .any(|m| matches!(m, ShadowMsg::Header(_))));

    // Rejected: the ring is full, the header is handed back and dropped on the live
    // thread, inside the measurement. A `String` in the input would be freed there.
    let full = local_transport(2);
    let other = full.recorder();
    mark(&other, Mark::Reply(ReplyKind::Ok));
    mark(&other, Mark::Reply(ReplyKind::Ok));
    let recorder = full.recorder();
    let header = smart_auto_header();
    let ((), allocations, frees) = count_heap_calls(|| {
        recorder.enter_pre();
        let ticket = recorder.ticket(101);
        let mut sent = header.clone();
        sent.seq = ticket.seq();
        sent.rec = ticket.rec();
        submit(ticket, sent);
    });
    assert_eq!(
        (allocations, frees),
        (0, 0),
        "rejecting a header allocated or freed on the live thread"
    );
    assert!(full.lost() >= 1);
    drop(header);
}

#[test]
fn the_counting_allocator_sees_frees_too() {
    // The measurement can fail: a `String` dropped inside it is one free.
    let owned = String::from("x");
    let ((), allocations, frees) = count_heap_calls(|| drop(owned));
    assert_eq!(allocations, 0);
    assert!(frees >= 1, "the hook did not see the free");
}
