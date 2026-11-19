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
    CondWitness, ErrClass, LiveOp, MirrorField, MirrorValue, MirrorWitness, NoReplyReason, PredId,
    Projection, ReplyKind, ShadowHeader, ShadowMsg, TickerGate, TickerSkipReason, WitnessedEffect,
    PRE_CAPACITY,
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
        Mark::TickerSkipped {
            target: rid,
            reason: TickerSkipReason::NoContext,
        },
    );
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
    // A flight is six messages: header, witness, reply mark, ticker-skipped mark,
    // conditional mark, close.
    assert_eq!(pop_all(&transport).len(), 8 * 6);
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

/// Stage 2c-2: the producer-cost probe at the **live capture sites'** API
/// (`dictation_executor::shadow::Flight`), the one every site in `commands/` goes
/// through. For every input family in every status, the decision is taken outside the
/// measurement and then replayed through the flight exactly as a site does: recorder
/// before `Pre`, buffered `Pre` witnesses, the observed fact, the header at the
/// decision, the `In` witnesses, `seal` (the `seq`), `submit`, the later witnesses,
/// `finish`. The thread allocates and frees nothing; the transport's own counters
/// show the messages were really produced.
#[test]
fn the_live_capture_sites_allocate_and_free_nothing() {
    use crate::dictation_executor::shadow::{view_projection, Flight, Noted};
    use crate::dictation_shadow::types::{EffectResult, ObservedFact};

    struct Disarm;
    impl Drop for Disarm {
        fn drop(&mut self) {
            crate::dictation_shadow::set_test_transport(None);
        }
    }

    let transport = default_transport();
    crate::dictation_shadow::set_test_transport(Some(Arc::clone(&transport)));
    let _disarm = Disarm;
    let mut flights = 0_u64;
    let mut messages = 0_usize;
    // Warm anything lazy outside the measurement.
    drop(Flight::begin());
    for status in Status::ALL {
        for current in [RID, RecordingId(2), RecordingId::NONE] {
            let core = view(status, current);
            for input in every_input_family() {
                let decision = core.handle(input.clone(), NOW);
                let projection = view_projection(&core);
                let reply = decision.reply.clone();
                let ((), allocations, frees) = count_heap_calls(|| {
                    let flight = Flight::begin();
                    flight.enter_pre();
                    flight.observed(ObservedFact::SpeculativeFlowing(true));
                    let note = |flight: &Flight, index: usize, at: Placement| {
                        flight.effect(Noted {
                            index,
                            placement: at,
                            effect: &decision.effects[index].effect,
                            taken: true,
                            result: EffectResult::Ran,
                            mirror: None,
                            ticker: None,
                        });
                    };
                    for (index, placed) in decision.effects.iter().enumerate() {
                        if placed.at == Placement::Pre {
                            note(&flight, index, Placement::Pre);
                        }
                    }
                    flight.decided(
                        input.clone(),
                        projection,
                        NOW,
                        decision.row,
                        reply.as_ref(),
                        decision.effects.len(),
                    );
                    for (index, placed) in decision.effects.iter().enumerate() {
                        if placed.at == Placement::In {
                            note(&flight, index, Placement::In);
                        }
                    }
                    let sealed = flight.seal(Some(projection));
                    sealed.submit();
                    for (index, placed) in decision.effects.iter().enumerate() {
                        if matches!(placed.at, Placement::PostDT | Placement::PostT) {
                            note(&flight, index, placed.at);
                        }
                    }
                    flight.finish_leg(decision.effects.len());
                });
                assert_eq!(allocations, 0, "a live capture site allocated");
                assert_eq!(frees, 0, "a live capture site freed");
                flights += 1;
                // Drain outside the measurement, so the queue never fills and the
                // accept path is the one measured.
                let drained = pop_all(&transport).len();
                assert!(drained >= 2, "the flight produced no messages");
                messages += drained;
            }
        }
    }
    assert!(flights >= 100, "{flights}");
    assert_eq!(transport.lost(), 0, "{messages} messages, none lost");
    // And with the queue full: the drop path is two counters, nothing else.
    let full = local_transport(2);
    crate::dictation_shadow::set_test_transport(Some(Arc::clone(&full)));
    let core = view(Status::Recording, RID);
    let input = Input::CancelRequest(CancelInput { expected: None });
    let decision = core.handle(input.clone(), NOW);
    let projection = view_projection(&core);
    let ((), allocations, frees) = count_heap_calls(|| {
        for _ in 0..4 {
            let flight = Flight::begin();
            flight.enter_pre();
            flight.decided(
                input.clone(),
                projection,
                NOW,
                decision.row,
                decision.reply.as_ref(),
                decision.effects.len(),
            );
            flight.seal(None).submit();
            flight.finish_leg(decision.effects.len());
        }
    });
    assert_eq!(allocations, 0, "the drop path allocated");
    assert_eq!(frees, 0, "the drop path freed");
    assert!(full.lost() > 0);
}

#[test]
fn the_live_capture_module_names_no_lock_box_channel_or_wake() {
    // The production text of the module every live site goes through: no lock, no
    // allocation, no wait, no wake, no environment read, no thread.
    let source =
        super::policy::production_source(include_str!("../../dictation_executor/shadow.rs"));
    for token in [
        "Mutex",
        "RwLock",
        ".lock(",
        "Condvar",
        "Barrier",
        "mpsc",
        "Box::new",
        "Box<",
        "vec!",
        "String",
        "format!",
        "to_string",
        "to_vec",
        "Arc::new",
        "Rc::new",
        "unpark",
        "notify",
        "sleep",
        ".recv",
        ".wait(",
        ".join(",
        "thread::spawn",
        "thread::park",
        "thread::sleep",
        "thread::Builder",
        "env::var",
        "println!",
        "eprintln!",
        "SystemTime",
        "Instant::now",
    ] {
        assert!(
            !source.contains(token),
            "the live capture module names `{token}`"
        );
    }
    // The module's thread-locals are `const`-initialised `Cell`s (no lazy allocation, no
    // destructor registration), and its only `Vec` is the reply scope's, which the
    // command wrapper opens once per invocation and a flight reaches only after its
    // header was submitted, with D released: never on the capture path under D.
    for (at, _) in source.match_indices("thread_local!") {
        let block = &source[at..source[at..]
            .find("\n}\n")
            .map_or(source.len(), |end| at + end)];
        for line in block
            .lines()
            .filter(|line| line.trim_start().starts_with("static "))
        {
            assert!(
                line.contains("Cell<") && line.contains("const {"),
                "a capture thread-local must be a const-initialised Cell: {line}"
            );
        }
    }
    let vec_lines: Vec<&str> = source
        .lines()
        .filter(|line| line.contains("Vec<"))
        .collect();
    assert_eq!(
        vec_lines.len(),
        1,
        "the live capture module has one Vec, the reply scope's: {vec_lines:?}"
    );
    assert!(vec_lines[0].contains("REPLY_SINK") || vec_lines[0].contains("RefCell<Vec<Live>>"));
}

/// D-S2c-7 e: an unwind after the decision was taken and part of its `In` effects
/// committed, before the header was sealed. The decision is not lost from the
/// comparison: the flight's drop takes the header's ticket and drops it unsent, which
/// records a loss, and the recorder does not then report a `PreAbandoned` for a
/// decision that entered D.
#[test]
fn an_unwind_after_a_partial_in_commit_is_recorded_as_loss_not_pre_abandoned() {
    use crate::dictation_executor::shadow::{view_projection, Flight, Noted};
    use crate::dictation_shadow::replay::Replay;
    use crate::dictation_shadow::types::EffectResult;

    // A decision with at least two `In` effects: the first commits, the unwind comes
    // before the second.
    let (input, core, decision) = Status::ALL
        .into_iter()
        .flat_map(|status| {
            [RID, RecordingId(2), RecordingId::NONE]
                .into_iter()
                .map(move |current| (status, current))
        })
        .find_map(|(status, current)| {
            let core = view(status, current);
            every_input_family().into_iter().find_map(|input| {
                let decision = core.handle(input.clone(), NOW);
                let in_effects = decision
                    .effects
                    .iter()
                    .filter(|placed| placed.at == Placement::In)
                    .count();
                (in_effects >= 2).then_some((input, core, decision))
            })
        })
        .expect("some decision has two `In` effects");
    let replay = Replay::begin();
    {
        let flight = Flight::begin();
        flight.enter_pre();
        flight.decided(
            input,
            view_projection(&core),
            NOW,
            decision.row,
            decision.reply.as_ref(),
            decision.effects.len(),
        );
        let first_in = decision
            .effects
            .iter()
            .position(|placed| placed.at == Placement::In)
            .expect("an `In` effect");
        flight.effect(Noted {
            index: first_in,
            placement: Placement::In,
            effect: &decision.effects[first_in].effect,
            taken: true,
            result: EffectResult::Ran,
            mirror: None,
            ticker: None,
        });
        // The unwind: the flight is dropped with its header unsealed.
        drop(flight);
    }
    let report = replay.end();
    let stats = &report.stats;
    assert_eq!(
        stats.abandoned_pre_d, 0,
        "a decision that entered D must not read as PreAbandoned: {stats:?}"
    );
    assert_eq!(
        report.lost, 1,
        "the unwind must be recorded as one lost header: {stats:?}"
    );
    assert_eq!(stats.compared, 0, "nothing was compared: {stats:?}");
}

// ---------------------------------------------------------------- CP4 at the return

/// Arms the shadow on `transport` for the length of a test.
struct Armed;
impl Armed {
    fn on(transport: &Arc<crate::dictation_shadow::transport::Transport>) -> Self {
        crate::dictation_shadow::set_test_transport(Some(Arc::clone(transport)));
        Self
    }
}
impl Drop for Armed {
    fn drop(&mut self) {
        crate::dictation_shadow::set_test_transport(None);
    }
}

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime")
        .block_on(future)
}

/// One decision that has a reply and no effect (a header that is complete the moment it
/// is sealed), handed to the flight the way a command's section does.
fn replying_flight(
    reply: &murmur_dictation_core::replies::Reply,
) -> crate::dictation_executor::shadow::Flight {
    use crate::dictation_executor::shadow::{view_projection, Flight};
    let core = view(Status::Idle, RID);
    let input = Input::CancelRequest(CancelInput { expected: None });
    let decision = core.handle(input.clone(), NOW);
    let flight = Flight::begin();
    flight.enter_pre();
    flight.decided(
        input,
        view_projection(&core),
        NOW,
        decision.row,
        Some(reply),
        0,
    );
    flight.seal(None).submit();
    flight
}

fn reply_and_close(messages: &[ShadowMsg]) -> (Vec<ReplyKind>, Vec<bool>) {
    let mut replies = Vec::new();
    let mut closes = Vec::new();
    for message in messages {
        match message {
            ShadowMsg::Reply { kind, .. } => replies.push(*kind),
            ShadowMsg::Close { completed, .. } => closes.push(*completed),
            _ => {}
        }
    }
    (replies, closes)
}

/// D-S2c-8 f: the reply a flight reports is the value the command **returned**, not the
/// kind it decided: an `Err` for a typed reply, an `Ok` of a type no decision names (its
/// own kind, never read as `Err`), a typed one, a transcription, a plain value.
#[test]
fn a_command_reply_is_witnessed_as_the_value_the_command_returned() {
    use crate::dictation_executor::shadow::commanded;
    use murmur_dictation_core::replies::{ProcessAudioRefusal, Reply, ReplyType};
    use serde_json::json;

    let decided = Reply::typed(ReplyType::NotRecording, Status::Idle);
    let cases: Vec<(Result<serde_json::Value, String>, ReplyKind)> = vec![
        (
            Ok(json!({"type": ReplyType::NotRecording.as_str(), "state": "idle"})),
            ReplyKind::TypedAt(ReplyType::NotRecording, Status::Idle),
        ),
        (
            Ok(json!({"type": ReplyType::AlreadyRecording.as_str(), "state": "recording"})),
            ReplyKind::TypedAt(ReplyType::AlreadyRecording, Status::Recording),
        ),
        // An error text no fixed class names is the live side's own class (never a
        // match for a fixed one); a fixed text is classified.
        (
            Err("failed".to_string()),
            ReplyKind::ErrAs(ErrClass::Unclassified),
        ),
        (
            Err(ProcessAudioRefusal::Benchmark.message().to_string()),
            ReplyKind::ErrAs(ErrClass::ProcessAudio(ProcessAudioRefusal::Benchmark)),
        ),
        (
            Err("Missing dictation context for recording 12".to_string()),
            ReplyKind::ErrAs(ErrClass::MissingContext),
        ),
        (
            Ok(json!({"type": "no_such_reply", "state": "idle"})),
            ReplyKind::Unrecognized,
        ),
        (
            Ok(json!({"type": "transcription", "text": "", "state": "idle"})),
            ReplyKind::TranscriptionAt(Status::Idle),
        ),
        // The normal `process_audio` transcription returns no `state`: the kind alone.
        (
            Ok(json!({"type": "transcription", "text": ""})),
            ReplyKind::Transcription,
        ),
        // A typed reply with no readable state names a kind no decision does (S7).
        (
            Ok(json!({"type": ReplyType::NotRecording.as_str()})),
            ReplyKind::Unrecognized,
        ),
        (
            Ok(json!({"type": ReplyType::NotRecording.as_str(), "state": "bogus"})),
            ReplyKind::Unrecognized,
        ),
        (Ok(json!({"state": "idle"})), ReplyKind::Ok),
    ];
    for (returned, expected) in cases {
        let transport = default_transport();
        let _armed = Armed::on(&transport);
        let result = block_on(commanded(async {
            let flight = replying_flight(&decided);
            drop(flight);
            // The command has not returned yet: the flight's reply and close wait.
            let (replies, closes) = reply_and_close(&pop_all(&transport));
            assert!(
                replies.is_empty() && closes.is_empty(),
                "the flight concluded before the command returned: {replies:?} {closes:?}"
            );
            returned.clone()
        }));
        assert_eq!(result, returned);
        let (replies, closes) = reply_and_close(&pop_all(&transport));
        assert_eq!(replies, vec![expected], "for {returned:?}");
        assert_eq!(closes, vec![true], "the flight closes once, completed");
    }
}

/// A reply that is another site's to return (R201.4, R205.4) has an explicit `Delegated`
/// disposition: its flight reports no reply kind (the comparison stays at the site that
/// renders it) and closes as the named no-reply close, never with the kind it decided
/// (D-S2c-10 R5).
#[test]
fn a_delegated_reply_reports_no_kind_and_closes_as_the_named_no_reply_close() {
    use crate::dictation_executor::shadow::commanded;
    use murmur_dictation_core::replies::{Reply, ReplyType};

    let decided = Reply::typed(ReplyType::RecordingCancelled, Status::Idle);
    let transport = default_transport();
    let _armed = Armed::on(&transport);
    let _: Result<(), String> = block_on(commanded(async {
        let flight = replying_flight(&decided);
        flight.delegated_reply();
        let before = crate::dictation_executor::shadow::conservation_now();
        drop(flight);
        let (replies, closes) = reply_and_close(&pop_all(&transport));
        assert!(replies.is_empty(), "no kind was reported: {replies:?}");
        assert_eq!(closes, vec![false], "closed as the named no-reply close");
        let counted = crate::dictation_executor::shadow::conservation_now().since(before);
        assert_eq!((counted.no_reply, counted.unrendered), (1, 0));
        Ok(())
    }));
    // The command's own return credits nothing to the delegated flight.
    assert_eq!(reply_and_close(&pop_all(&transport)), (vec![], vec![]));
}

/// A flight that no command wrapper is around (a caller that is not an IPC command) has no
/// return to witness: it is never given the kind it decided, closes incomplete (the named
/// no-reply close) and the test build counts it as unrendered (D-S2c-9 d).
#[test]
fn a_flight_outside_a_command_closes_with_no_reply_and_is_counted() {
    use murmur_dictation_core::replies::{Reply, ReplyType};

    let transport = default_transport();
    let _armed = Armed::on(&transport);
    let before = crate::dictation_executor::shadow::conservation_now();
    drop(replying_flight(&Reply::typed(
        ReplyType::NotRecording,
        Status::Idle,
    )));
    let (replies, closes) = reply_and_close(&pop_all(&transport));
    assert!(replies.is_empty(), "no reply was witnessed: {replies:?}");
    assert!(
        !closes.contains(&true),
        "the flight is never closed as completed: {closes:?}"
    );
    let after = crate::dictation_executor::shadow::conservation_now().since(before);
    assert_eq!(after.unrendered, 1);
}

/// D-S2c-8 amendment 1: the reply scope is one tokio task-local per invocation, and
/// nothing a command spawns can observe it (a spawned task, a blocking task and a thread
/// all start without it), so no continuation can read or settle a flight's reply.
#[test]
fn a_spawned_continuation_cannot_observe_the_reply_scope() {
    use crate::dictation_executor::shadow::{commanded, reply_sink_visible};

    let transport = default_transport();
    let _armed = Armed::on(&transport);
    assert!(!reply_sink_visible(), "no scope outside a command");
    let seen: Result<serde_json::Value, String> = block_on(commanded(async {
        let here = reply_sink_visible();
        let spawned = tokio::spawn(async { reply_sink_visible() }).await.unwrap();
        let blocking = tokio::task::spawn_blocking(reply_sink_visible)
            .await
            .unwrap();
        let thread = std::thread::spawn(reply_sink_visible).join().unwrap();
        Ok(serde_json::json!({
            "here": here, "spawned": spawned, "blocking": blocking, "thread": thread
        }))
    }));
    let seen = seen.unwrap();
    assert_eq!(seen["here"], true, "the command's own task sees its scope");
    for other in ["spawned", "blocking", "thread"] {
        assert_eq!(
            seen[other], false,
            "a {other} continuation saw the reply scope"
        );
    }
    assert!(!reply_sink_visible(), "the scope ends with the invocation");

    // A disarmed shadow does not open the scope at all.
    drop(_armed);
    let seen: Result<serde_json::Value, String> = block_on(commanded(async {
        Ok(serde_json::json!({ "here": reply_sink_visible() }))
    }));
    assert_eq!(
        seen.unwrap()["here"],
        false,
        "disarmed: no scope, no allocation"
    );
}

// ------------------------------------------- the flight's slots (D-S2c-9 c)

/// One flight's `In` effect carrying a mirror the capture wrote into the flight's own
/// slots (what `ObservingPorts`/the `*_observed` twins do), noted the way the executor
/// notes it: take the slot's value, hand it to the flight.
fn mirror_of(id: u64) -> MirrorWitness {
    MirrorWitness {
        field: MirrorField::CancelledThrough,
        before: MirrorValue::Id(RecordingId(0)),
        after: MirrorValue::Id(RecordingId(id)),
    }
}

fn decided_flight(effects: usize) -> crate::dictation_executor::shadow::Flight {
    use crate::dictation_executor::shadow::{view_projection, Flight};
    let core = view(Status::Recording, RID);
    let input = Input::CancelRequest(CancelInput { expected: None });
    let decision = core.handle(input.clone(), NOW);
    let flight = Flight::begin();
    flight.enter_pre();
    flight.decided(
        input,
        view_projection(&core),
        NOW,
        decision.row,
        None,
        effects,
    );
    flight
}

fn note_mirror(flight: &crate::dictation_executor::shadow::Flight, effect: &Effect) {
    use crate::dictation_executor::shadow::Noted;
    let slots = flight.slots().expect("an active flight has slots");
    flight.effect(Noted {
        index: 0,
        placement: Placement::In,
        effect,
        taken: true,
        result: crate::dictation_shadow::types::EffectResult::Ran,
        mirror: slots.mirror.take(),
        ticker: slots.ticker.take(),
    });
}

fn header_mirrors(messages: &[ShadowMsg]) -> Vec<Vec<MirrorWitness>> {
    messages
        .iter()
        .filter_map(|message| match message {
            ShadowMsg::Header(header) => Some(header.in_d_mirrors.to_vec()),
            _ => None,
        })
        .collect()
}

/// The probe that showed the thread-local flight misattributing across an `.await`: two
/// decisions' flights live on **one thread**, their tasks interleaved at an await. Each
/// capture writes into its own flight's slots, so each header carries its own write and
/// nothing of the other's, whatever order the tasks run in. A capture holds nothing that
/// names "the current flight".
#[test]
fn flights_interleaved_across_an_await_on_one_thread_never_see_each_others_slots() {
    let transport = default_transport();
    let _armed = Armed::on(&transport);
    let effect = Effect::MarkCancelled(RecordingId(1));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("a runtime");
    runtime.block_on(async {
        let task = |id: u64, yields: usize| {
            let effect = effect.clone();
            async move {
                let flight = decided_flight(1);
                flight
                    .slots()
                    .expect("slots")
                    .mirror
                    .set(Some(mirror_of(id)));
                for _ in 0..yields {
                    // Another task runs here, on this very thread, writing its own flight.
                    tokio::task::yield_now().await;
                }
                note_mirror(&flight, &effect);
                flight.seal(None).submit();
            }
        };
        tokio::join!(task(11, 3), task(22, 1), task(33, 0));
    });
    let mut seen: Vec<Vec<MirrorWitness>> = header_mirrors(&pop_all(&transport));
    seen.sort_by_key(|mirrors| match mirrors.first().map(|m| m.after) {
        Some(MirrorValue::Id(id)) => id.0,
        _ => 0,
    });
    assert_eq!(
        seen,
        vec![
            vec![mirror_of(11)],
            vec![mirror_of(22)],
            vec![mirror_of(33)]
        ],
        "each flight's header carries exactly its own write"
    );
}

/// The other probe: flights dropped in an order that is not the order they began in. A
/// dropped flight in the middle of its decision records its loss and takes nothing with
/// it; the others still report their own writes (there is no restore of a previous
/// "current" to dangle).
#[test]
fn flights_dropped_out_of_order_lose_only_themselves() {
    let transport = default_transport();
    let _armed = Armed::on(&transport);
    let effect = Effect::MarkCancelled(RecordingId(1));
    let before = crate::dictation_executor::shadow::conservation_now();

    let first = decided_flight(1);
    first.slots().expect("slots").mirror.set(Some(mirror_of(1)));
    let second = decided_flight(1);
    second
        .slots()
        .expect("slots")
        .mirror
        .set(Some(mirror_of(2)));
    let third = decided_flight(1);
    third.slots().expect("slots").mirror.set(Some(mirror_of(3)));

    // The middle one is dropped first, then the first: neither reports a header.
    drop(second);
    drop(first);
    note_mirror(&third, &effect);
    third.seal(None).submit();
    drop(third);

    let after = crate::dictation_executor::shadow::conservation_now().since(before);
    assert_eq!(
        after.unwound, 2,
        "the two dropped flights are recorded losses"
    );
    assert_eq!(
        after.headers, 1,
        "only the surviving flight submitted a header"
    );
    assert_eq!(
        header_mirrors(&pop_all(&transport)),
        vec![vec![mirror_of(3)]],
        "the surviving flight's header carries its own write, nothing else"
    );
}

// ------------------------------------------ named no-reply closes (D-S2c-9 d)

/// A finish a drain ran from the deferred queue has no reply of its own: its flight closes
/// as the named no-reply close, never with the kind it decided and never completed, even
/// inside a command's scope. A synchronous command nested in it keeps its own reply
/// (D-S2c-10 R6: the old depth counter lost it).
#[test]
fn a_queued_finish_closes_with_no_reply_and_a_nested_command_keeps_its_own() {
    use crate::dictation_executor::shadow::{commanded, unreplied};
    use murmur_dictation_core::replies::{Reply, ReplyType};
    use std::future::Future;

    let decided = Reply::typed(ReplyType::NotRecording, Status::Idle);
    let transport = default_transport();
    let _armed = Armed::on(&transport);
    let before = crate::dictation_executor::shadow::conservation_now();
    let _ = block_on(commanded(async {
        unreplied(|| {
            // The finish's own flight.
            drop(replying_flight(&decided));
            // A synchronous command nested inside it, returning at once.
            let mut nested = Box::pin(commanded(async {
                drop(replying_flight(&decided));
                Ok::<_, String>(serde_json::json!({"type": "not_recording", "state": "idle"}))
            }));
            let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
            assert!(nested.as_mut().poll(&mut cx).is_ready());
        });
        Ok::<_, String>(serde_json::json!({"type": "not_recording", "state": "idle"}))
    }));
    let (replies, mut closes) = reply_and_close(&pop_all(&transport));
    assert_eq!(
        replies,
        vec![ReplyKind::TypedAt(ReplyType::NotRecording, Status::Idle)],
        "only the nested command's flight has a reply"
    );
    closes.sort_unstable();
    assert_eq!(
        closes,
        vec![false, true],
        "the finish closes incomplete, the nested command's flight completes"
    );
    let counted = crate::dictation_executor::shadow::conservation_now().since(before);
    assert_eq!((counted.no_reply, counted.unrendered), (1, 0));
}

/// The import's pipeline failure is modelled as an `Err` outcome for R302.2, and the actual
/// return is what it is compared against (D-S2c-10 R3, amendment 1 iii): the `Err` it
/// returns closes the waiting flights as the named no-reply close; any other return,
/// **the decided success included**, is credited with a kind no decision names, so the
/// comparator reports it.
#[test]
fn an_import_pipeline_error_is_compared_with_the_actual_return() {
    use crate::dictation_executor::shadow::{commanded, import_pipeline_error};
    use murmur_dictation_core::replies::{Reply, ReplyType};

    for decided in [
        Reply::typed(ReplyType::NotRecording, Status::Idle),
        Reply::Transcription {
            text: murmur_dictation_core::replies::TranscriptText::FromPipeline,
            state: Status::Idle,
        },
    ] {
        for (returned, credited) in [
            (Err("the pipeline failed".to_string()), None),
            (
                Ok(serde_json::json!({"type": "not_recording", "state": "idle"})),
                Some(ReplyKind::Unrecognized),
            ),
            (
                Ok(serde_json::json!({"type": "transcription", "text": ""})),
                Some(ReplyKind::Unrecognized),
            ),
        ] {
            let transport = default_transport();
            let _armed = Armed::on(&transport);
            let result = block_on(commanded(async {
                drop(replying_flight(&decided));
                import_pipeline_error();
                returned.clone()
            }));
            assert_eq!(result, returned);
            let (replies, closes) = reply_and_close(&pop_all(&transport));
            assert_eq!(replies, credited.into_iter().collect::<Vec<_>>());
            assert_eq!(
                closes.contains(&true),
                credited.is_some(),
                "for {returned:?}: {closes:?}"
            );
        }
    }
}

/// The capture keeps no ambient "current flight": nothing in the shadow's live module is a
/// thread-local flight, a scope, or a stash another site takes from (D-S2c-9 c).
#[test]
fn the_live_capture_module_keeps_no_ambient_flight_or_stash() {
    let source = include_str!("../../dictation_executor/shadow.rs");
    let code: String = source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    for banned in [
        "static CURRENT",
        "fn with_current",
        "fn stash_",
        "fn take_retired_op",
        "fn take_h2_retired_op",
        "fn armed(",
        "static MIRROR",
        "static TICKER:",
        "static H1_RETIRED",
        "static H2_RETIRED",
    ] {
        assert!(!code.contains(banned), "shadow.rs still has `{banned}`");
    }
    // The only thread-locals left are a write sequence and a one-call mailbox: no flight.
    for block in code.split("thread_local!").skip(1) {
        let head: String = block.lines().take(4).collect::<Vec<_>>().join(" ");
        assert!(
            head.contains("KEYBOARD_WRITES") || head.contains("TICKER_SKIP"),
            "an unexpected thread-local in the live capture: {head}"
        );
        assert!(
            !head.contains("Flight"),
            "a thread-local names a flight: {head}"
        );
    }
}

fn no_reply_reasons(messages: &[ShadowMsg]) -> Vec<NoReplyReason> {
    messages
        .iter()
        .filter_map(|message| match message {
            ShadowMsg::NoReply { reason, .. } => Some(*reason),
            _ => None,
        })
        .collect()
}

/// D-S2c-15 review, major 1: each of the three paths that intentionally omit a reply says
/// why, **before** the close; a flight that merely has no reply says nothing.
#[test]
fn the_three_no_reply_paths_announce_their_reason_before_the_close() {
    use crate::dictation_executor::shadow::{commanded, import_pipeline_error, unreplied};
    use murmur_dictation_core::replies::{Reply, ReplyType};
    use NoReplyReason::{Deferred, Delegated, ImportPipelineError};

    fn assert_announced_before_close(messages: &[ShadowMsg], reason: NoReplyReason) {
        let at = |wanted: fn(&ShadowMsg) -> bool| messages.iter().position(wanted);
        let announced = at(|m| matches!(m, ShadowMsg::NoReply { .. })).expect("announced");
        let closed = at(|m| {
            matches!(
                m,
                ShadowMsg::Close {
                    completed: false,
                    ..
                }
            )
        })
        .expect("closed");
        assert!(
            announced < closed,
            "the reason precedes the close: {messages:?}"
        );
        assert_eq!(no_reply_reasons(messages), vec![reason]);
    }
    let decided = Reply::typed(ReplyType::RecordingCancelled, Status::Idle);

    // A reply another site returns.
    let transport = default_transport();
    let _armed = Armed::on(&transport);
    let _: Result<(), String> = block_on(commanded(async {
        let flight = replying_flight(&decided);
        flight.delegated_reply();
        drop(flight);
        assert_announced_before_close(&pop_all(&transport), Delegated);
        Ok(())
    }));
    drop(_armed);

    // A finish a queued drain ran.
    let transport = default_transport();
    let _armed = Armed::on(&transport);
    let _: Result<(), String> = block_on(commanded(async {
        unreplied(|| drop(replying_flight(&decided)));
        assert_announced_before_close(&pop_all(&transport), Deferred);
        Ok(())
    }));
    drop(_armed);

    // The import's own pipeline error, returned: announced. Any other return: the flight
    // concludes with a reply the comparator reports, and announces nothing.
    for (returned, reason) in [
        (
            Err("the pipeline failed".to_string()),
            Some(ImportPipelineError),
        ),
        (
            Ok(serde_json::json!({"type": "not_recording", "state": "idle"})),
            None,
        ),
    ] {
        let transport = default_transport();
        let _armed = Armed::on(&transport);
        let _ = block_on(commanded(async {
            drop(replying_flight(&decided));
            import_pipeline_error();
            returned.clone()
        }));
        let messages = pop_all(&transport);
        match reason {
            Some(reason) => assert_announced_before_close(&messages, reason),
            None => assert!(no_reply_reasons(&messages).is_empty(), "{messages:?}"),
        }
    }

    // A flight with no command around it, or one that was just dropped, announces nothing.
    let transport = default_transport();
    let _armed = Armed::on(&transport);
    drop(replying_flight(&Reply::typed(
        ReplyType::NotRecording,
        Status::Idle,
    )));
    assert!(no_reply_reasons(&pop_all(&transport)).is_empty());
}

/// D-S2c-15 review, major 3, through the production wrapper: the refusal a blocked
/// `process_audio` decides is a meeting's; a command that returns the **benchmark**
/// refusal text instead is classified from the text it returned, and the comparison
/// reports it (`diff_kind 4`). A text no fixed class names agrees only with a class whose
/// text the shell owns, and is counted.
#[test]
fn a_wrong_error_text_returned_by_the_command_is_a_reply_mismatch() {
    use crate::dictation_executor::shadow::commanded;
    use murmur_dictation_core::replies::ProcessAudioRefusal;

    let live_kind = |returned: &str| -> ReplyKind {
        let transport = default_transport();
        let _armed = Armed::on(&transport);
        let decided = process_audio_refused(1, 1).decision.reply.unwrap();
        let _ = block_on(commanded(async {
            drop(replying_flight(&decided));
            Err::<serde_json::Value, String>(returned.to_string())
        }));
        reply_and_close(&pop_all(&transport))
            .0
            .pop()
            .expect("a reply")
    };
    let feed = |kind: ReplyKind| {
        let mut h = harness();
        let mut flight = process_audio_refused(1, 1);
        *flight
            .later
            .iter_mut()
            .find_map(|msg| match msg {
                ShadowMsg::Reply { kind, .. } => Some(kind),
                _ => None,
            })
            .unwrap() = kind;
        h.feed(&flight);
        h
    };

    // The right text is a match; the wrong fixed text is a reply mismatch.
    let right = live_kind(ProcessAudioRefusal::MeetingActive.message());
    assert_eq!(
        right,
        ReplyKind::ErrAs(ErrClass::ProcessAudio(ProcessAudioRefusal::MeetingActive))
    );
    assert_clean(&feed(right));
    let wrong = live_kind(ProcessAudioRefusal::Benchmark.message());
    assert_eq!(
        feed(wrong).only_event().diff_kind,
        crate::dictation_shadow::types::DiffKind::Reply.code()
    );
    // A text the producer cannot place never satisfies a fixed class.
    let unplaced = live_kind("something else");
    assert_eq!(unplaced, ReplyKind::ErrAs(ErrClass::Unclassified));
    assert_eq!(
        feed(unplaced).only_event().diff_kind,
        crate::dictation_shadow::types::DiffKind::Reply.code()
    );
}
